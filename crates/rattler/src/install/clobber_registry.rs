//! Implements a registry for "clobbering" files (files that are appearing in multiple packages)

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use rattler_conda_types::{
    package::{IndexJson, PathsEntry},
    PackageName, PrefixRecord,
};

use fs_err as fs;
/// A registry for clobbering files
/// The registry keeps track of all files that are installed by a package and
/// can be used to rename files that are already installed by another package.
#[derive(Debug, Default, Clone)]
pub struct ClobberRegistry {
    paths_registry: HashMap<PathBuf, usize>,
    clobbers: HashMap<PathBuf, Vec<usize>>,
    package_names: Vec<PackageName>,
}

static CLOBBER_TEMPLATE: &str = "__clobber-from-";

fn clobber_template(package_name: &PackageName) -> String {
    format!("{CLOBBER_TEMPLATE}{}", package_name.as_normalized())
}

impl ClobberRegistry {
    /// Create a new clobber registry that is initialized with the given prefix records.
    pub fn from_prefix_records(prefix_records: &[PrefixRecord]) -> Self {
        let mut registry = Self::default();

        let mut temp_clobbers = Vec::new();
        for prefix_record in prefix_records {
            let package_name = prefix_record.repodata_record.package_record.name.clone();
            registry.package_names.push(package_name.clone());

            for p in &prefix_record.paths_data.paths {
                if let Some(original_path) = &p.original_path {
                    temp_clobbers.push((original_path, package_name.clone()));
                } else {
                    registry
                        .paths_registry
                        .insert(p.relative_path.clone(), registry.package_names.len() - 1);
                }
            }
        }

        for (path, originating_package) in temp_clobbers.iter() {
            let idx = registry
                .package_names
                .iter()
                .position(|n| n == originating_package)
                .expect("package not found even though it was just added");

            let path = *path;
            registry
                .clobbers
                .entry(path.clone())
                .or_insert_with(|| {
                    if let Some(other_idx) = registry.paths_registry.get(path) {
                        vec![*other_idx]
                    } else {
                        Vec::new()
                    }
                })
                .push(idx);
        }

        registry
    }

    fn clobber_name(path: &Path, package_name: &PackageName) -> PathBuf {
        let file_name = path.file_name().unwrap_or_default();
        let mut new_path = path.to_path_buf();
        new_path.set_file_name(format!(
            "{}{}",
            file_name.to_string_lossy(),
            clobber_template(package_name),
        ));
        new_path
    }

    /// Register the paths of a package before linking a package in
    /// order to determine which files may clobber other files (clobbering files are
    /// those that are present in multiple packages).
    ///
    /// This function has to run sequentially, and a `post_process` step
    /// will "unclobber" the files after all packages have been installed.
    pub fn register_paths(
        &mut self,
        index_json: &IndexJson,
        computed_paths: &Vec<(PathsEntry, PathBuf)>,
    ) -> HashMap<PathBuf, PathBuf> {
        let mut clobber_paths = HashMap::new();
        let name = &index_json.name.clone();

        // check if we have the package name already registered
        let name_idx = if let Some(idx) = self.package_names.iter().position(|n| n == name) {
            idx
        } else {
            self.package_names.push(name.clone());
            self.package_names.len() - 1
        };

        for (_, path) in computed_paths {
            // if we find an entry, we have a clobbering path!
            if let Some(e) = self.paths_registry.get(path) {
                if e == &name_idx {
                    // A name cannot appear twice in an environment.
                    // We get into this case if a package is updated (removed and installed again with a new version)
                    continue;
                }
                let new_path = Self::clobber_name(path, &self.package_names[name_idx]);
                self.clobbers
                    .entry(path.clone())
                    .or_insert_with(|| vec![*e])
                    .push(name_idx);

                // We insert the non-renamed path here
                clobber_paths.insert(path.clone(), new_path);
            } else {
                self.paths_registry.insert(path.clone(), name_idx);
            }
        }

        clobber_paths
    }

    /// Unclobber the paths after all installation steps have been completed.
    pub fn unclobber(
        &mut self,
        sorted_prefix_records: &[&PrefixRecord],
        target_prefix: &Path,
    ) -> Result<(), std::io::Error> {
        let sorted_names = sorted_prefix_records
            .iter()
            .map(|p| p.repodata_record.package_record.name.clone())
            .collect::<Vec<_>>();
        let conda_meta = target_prefix.join("conda-meta");

        let mut prefix_records = sorted_prefix_records
            .iter()
            .map(|x| (*x).clone())
            .collect::<Vec<PrefixRecord>>();
        let mut prefix_records_to_rewrite = HashSet::new();

        for (path, clobbered_by) in self.clobbers.iter() {
            let clobbered_by_names = clobbered_by
                .iter()
                .map(|&idx| self.package_names[idx].clone())
                .collect::<Vec<_>>();

            for pfx in sorted_prefix_records {
                println!("{}: {:?}", pfx.repodata_record.package_record.name.as_normalized(), pfx.paths_data);
            }

            // extract the subset of clobbered_by that is in sorted_prefix_records
            let sorted_clobbered_by = sorted_names
                .iter()
                .cloned()
                .enumerate()
                .filter(|(_, n)| clobbered_by_names.contains(n))
                // make sure that the file is actually in the package (because it could have been removed in the meantime)
                .filter(|(idx, _)| sorted_prefix_records[*idx].files.contains(path))
                .collect::<Vec<_>>();

            let winner = match sorted_clobbered_by.last() {
                Some(winner) => winner,
                // In this case, all files have been removed and we can skip any unclobbering
                None => continue,
            };

            if winner.1 == clobbered_by_names[0] {
                tracing::info!(
                    "clobbering decision: keep {} from {:?}",
                    path.display(),
                    winner
                );
            } else {
                let full_path = target_prefix.join(path);
                if full_path.exists() {
                    let loser_name = &clobbered_by_names[0];
                    let loser_path = Self::clobber_name(path, loser_name);

                    if let Err(e) =
                        fs::rename(target_prefix.join(path), target_prefix.join(&loser_path))
                    {
                        tracing::info!("could not rename file: {}", e);
                        continue;
                    }

                    let loser_idx = sorted_clobbered_by
                        .iter()
                        .find(|(_, n)| n == loser_name)
                        .expect("loser not found")
                        .0;

                    rename_path_in_prefix_record(
                        &mut prefix_records[loser_idx],
                        path,
                        &loser_path,
                        true,
                    );
                    prefix_records_to_rewrite.insert(loser_idx);

                    tracing::info!(
                        "clobbering decision: remove {} from {:?}",
                        path.display(),
                        loser_name
                    );
                }

                let winner_path = Self::clobber_name(path, &winner.1);

                tracing::info!(
                    "clobbering decision: choose {} from {:?}",
                    path.display(),
                    winner
                );

                if let Err(e) =
                    fs::rename(target_prefix.join(&winner_path), target_prefix.join(path))
                {
                    tracing::warn!("Could not rename file: {}", e);
                    continue;
                };

                rename_path_in_prefix_record(
                    &mut prefix_records[winner.0],
                    &winner_path,
                    path,
                    false,
                );

                prefix_records_to_rewrite.insert(winner.0);
            }
        }

        for idx in prefix_records_to_rewrite {
            let rec = &prefix_records[idx];
            tracing::info!(
                "Writing updated prefix record to: {:?}",
                conda_meta.join(rec.file_name())
            );
            rec.write_to_path(conda_meta.join(rec.file_name()), true)?;
        }

        Ok(())
    }
}

fn rename_path_in_prefix_record(
    record: &mut PrefixRecord,
    old_path: &Path,
    new_path: &Path,
    new_path_is_clobber: bool,
) {
    for path in record.files.iter_mut() {
        if path == old_path {
            *path = new_path.to_path_buf();
        }
    }

    for path in record.paths_data.paths.iter_mut() {
        if path.relative_path == old_path {
            path.relative_path = new_path.to_path_buf();
            path.original_path = if new_path_is_clobber {
                Some(old_path.to_path_buf())
            } else {
                None
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        str::FromStr,
    };

    use futures::TryFutureExt;
    use insta::assert_yaml_snapshot;
    use rand::seq::SliceRandom;
    use rattler_conda_types::{
        package::IndexJson, PackageRecord, Platform, PrefixRecord, RepoDataRecord, Version,
    };
    use rattler_digest::{Md5, Sha256};
    use rattler_networking::retry_policies::default_retry_policy;
    use rattler_package_streaming::seek::read_package_file;
    use transaction::{Transaction, TransactionOperation};

    use crate::{
        get_test_data_dir,
        install::{transaction, unlink_package, InstallDriver, InstallOptions, PythonInfo},
        package_cache::PackageCache,
    };

    fn get_repodata_record(filename: &str) -> RepoDataRecord {
        let path = fs::canonicalize(get_test_data_dir().join(filename)).unwrap();
        print!("{:?}", path);
        let index_json = read_package_file::<IndexJson>(&path).unwrap();

        // find size and hash
        let size = fs::metadata(&path).unwrap().len();
        let sha256 = rattler_digest::compute_file_digest::<Sha256>(&path).unwrap();
        let md5 = rattler_digest::compute_file_digest::<Md5>(&path).unwrap();

        RepoDataRecord {
            package_record: PackageRecord::from_index_json(
                index_json,
                Some(size),
                Some(sha256),
                Some(md5),
            )
            .unwrap(),
            file_name: filename.to_string(),
            url: url::Url::from_file_path(&path).unwrap(),
            channel: "clobber".to_string(),
        }
    }

    /// Install a package into the environment and write a `conda-meta` file that contains information
    /// about how the file was linked.
    async fn install_package_to_environment(
        target_prefix: &Path,
        package_dir: PathBuf,
        repodata_record: RepoDataRecord,
        install_driver: &InstallDriver,
        install_options: &InstallOptions,
    ) -> anyhow::Result<()> {
        // Link the contents of the package into our environment. This returns all the paths that were linked.
        let paths = crate::install::link_package(
            &package_dir,
            target_prefix,
            install_driver,
            install_options.clone(),
        )
        .await?;

        // Construct a PrefixRecord for the package
        let prefix_record = PrefixRecord {
            repodata_record,
            package_tarball_full_path: None,
            extracted_package_dir: Some(package_dir),
            files: paths
                .iter()
                .map(|entry| entry.relative_path.clone())
                .collect(),
            paths_data: paths.into(),
            requested_spec: None,
            link: None,
        };

        // Create the conda-meta directory if it doesnt exist yet.
        let target_prefix = target_prefix.to_path_buf();
        match tokio::task::spawn_blocking(move || {
            let conda_meta_path = target_prefix.join("conda-meta");
            std::fs::create_dir_all(&conda_meta_path)?;

            // Write the conda-meta information
            let pkg_meta_path = conda_meta_path.join(prefix_record.file_name());
            prefix_record.write_to_path(pkg_meta_path, true)
        })
        .await
        {
            Ok(result) => Ok(result?),
            Err(err) => {
                if let Ok(panic) = err.try_into_panic() {
                    std::panic::resume_unwind(panic);
                }
                // The operation has been cancelled, so we can also just ignore everything.
                Ok(())
            }
        }
    }

    async fn execute_operation(
        target_prefix: &Path,
        download_client: &reqwest_middleware::ClientWithMiddleware,
        package_cache: &PackageCache,
        install_driver: &InstallDriver,
        op: TransactionOperation<PrefixRecord, RepoDataRecord>,
        install_options: &InstallOptions,
    ) {
        // Determine the package to install
        let install_record = op.record_to_install();
        let remove_record = op.record_to_remove();

        if let Some(remove_record) = remove_record {
            unlink_package(target_prefix, remove_record).await.unwrap();
        }

        let install_package = if let Some(install_record) = install_record {
            // Make sure the package is available in the package cache.
            package_cache
                .get_or_fetch_from_url_with_retry(
                    &install_record.package_record,
                    install_record.url.clone(),
                    download_client.clone(),
                    default_retry_policy(),
                )
                .map_ok(|cache_dir| Some((install_record.clone(), cache_dir)))
                .map_err(anyhow::Error::from)
                .await
                .unwrap()
        } else {
            None
        };

        // If there is a package to install, do that now.
        if let Some((record, package_dir)) = install_package {
            install_package_to_environment(
                target_prefix,
                package_dir,
                record.clone(),
                install_driver,
                install_options,
            )
            .await
            .unwrap();
        }
    }

    async fn execute_transaction(
        transaction: Transaction<PrefixRecord, RepoDataRecord>,
        target_prefix: &Path,
        download_client: &reqwest_middleware::ClientWithMiddleware,
        package_cache: &PackageCache,
        install_driver: &InstallDriver,
        install_options: &InstallOptions,
    ) {
        for op in &transaction.operations {
            execute_operation(
                target_prefix,
                download_client,
                package_cache,
                install_driver,
                op.clone(),
                install_options,
            )
            .await;
        }

        install_driver
            .post_process(&transaction, target_prefix)
            .unwrap();
    }

    fn find_prefix_record<'a>(
        prefix_records: &'a [PrefixRecord],
        name: &str,
    ) -> Option<&'a PrefixRecord> {
        prefix_records
            .iter()
            .find(|r| r.repodata_record.package_record.name.as_normalized() == name)
    }

    fn test_operations() -> Vec<TransactionOperation<PrefixRecord, RepoDataRecord>> {
        let repodata_record_1 = get_repodata_record("clobber/clobber-1-0.1.0-h4616a5c_0.tar.bz2");
        let repodata_record_2 = get_repodata_record("clobber/clobber-2-0.1.0-h4616a5c_0.tar.bz2");
        let repodata_record_3 = get_repodata_record("clobber/clobber-3-0.1.0-h4616a5c_0.tar.bz2");

        vec![
            TransactionOperation::Install(repodata_record_1),
            TransactionOperation::Install(repodata_record_2),
            TransactionOperation::Install(repodata_record_3),
        ]
    }

    fn test_python_noarch_operations() -> Vec<TransactionOperation<PrefixRecord, RepoDataRecord>> {
        let repodata_record_1 =
            get_repodata_record("clobber/clobber-pynoarch-1-0.1.0-pyh4616a5c_0.tar.bz2");
        let repodata_record_2 =
            get_repodata_record("clobber/clobber-pynoarch-2-0.1.0-pyh4616a5c_0.tar.bz2");

        vec![
            TransactionOperation::Install(repodata_record_1),
            TransactionOperation::Install(repodata_record_2),
        ]
    }

    fn test_operations_nested() -> Vec<TransactionOperation<PrefixRecord, RepoDataRecord>> {
        let repodata_record_1 =
            get_repodata_record("clobber/clobber-nested-1-0.1.0-h4616a5c_0.tar.bz2");
        let repodata_record_2 =
            get_repodata_record("clobber/clobber-nested-2-0.1.0-h4616a5c_0.tar.bz2");
        let repodata_record_3 =
            get_repodata_record("clobber/clobber-nested-3-0.1.0-h4616a5c_0.tar.bz2");

        vec![
            TransactionOperation::Install(repodata_record_1),
            TransactionOperation::Install(repodata_record_2),
            TransactionOperation::Install(repodata_record_3),
        ]
    }

    fn test_operations_update() -> Vec<RepoDataRecord> {
        let repodata_record_1 = get_repodata_record("clobber/clobber-1-0.2.0-h4616a5c_0.tar.bz2");
        let repodata_record_2 = get_repodata_record("clobber/clobber-2-0.2.0-h4616a5c_0.tar.bz2");
        let repodata_record_3 = get_repodata_record("clobber/clobber-3-0.2.0-h4616a5c_0.tar.bz2");

        vec![repodata_record_1, repodata_record_2, repodata_record_3]
    }

    fn assert_check_files(target_prefix: &Path, expected_files: &[&str]) {
        let files = std::fs::read_dir(target_prefix).unwrap();
        let files = files
            .filter_map(|f| {
                let fx = f.unwrap();
                if fx.file_type().unwrap().is_file() {
                    Some(fx.path())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        println!("Files: {:?}", files);
        assert_eq!(files.len(), expected_files.len());
        println!("{:?}", files);

        for file in files {
            assert!(expected_files.contains(&file.file_name().unwrap().to_string_lossy().as_ref()));
        }
    }

    #[tokio::test]
    async fn test_transaction_with_clobber() {
        // Create a transaction
        let operations = test_operations();

        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations,
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        // execute transaction
        let target_prefix = tempfile::tempdir().unwrap();

        let packages_dir = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(packages_dir.path());

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &InstallDriver::default(),
            &InstallOptions::default(),
        )
        .await;

        // check that the files are there
        assert_check_files(
            target_prefix.path(),
            &[
                "clobber.txt",
                "clobber.txt__clobber-from-clobber-2",
                "clobber.txt__clobber-from-clobber-3",
                "another-clobber.txt",
                "another-clobber.txt__clobber-from-clobber-2",
                "another-clobber.txt__clobber-from-clobber-3",
            ],
        );

        let prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();

        let prefix_record_clobber_1 = find_prefix_record(&prefix_records, "clobber-1").unwrap();
        assert_yaml_snapshot!(prefix_record_clobber_1.files);
        assert_yaml_snapshot!(prefix_record_clobber_1.paths_data);
        let prefix_record_clobber_2 = find_prefix_record(&prefix_records, "clobber-2").unwrap();
        assert_yaml_snapshot!(prefix_record_clobber_2.files);
        assert_yaml_snapshot!(prefix_record_clobber_2.paths_data);

        assert_eq!(
            fs::read_to_string(target_prefix.path().join("clobber.txt")).unwrap(),
            "clobber-1\n"
        );

        // remove one of the clobbering files
        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations: vec![TransactionOperation::Remove(
                prefix_record_clobber_1.clone(),
            )],
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        let install_driver = InstallDriver::new(100, Some(&prefix_records));

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &install_driver,
            &InstallOptions::default(),
        )
        .await;

        assert_check_files(
            target_prefix.path(),
            &[
                "clobber.txt__clobber-from-clobber-3",
                "clobber.txt",
                "another-clobber.txt__clobber-from-clobber-3",
                "another-clobber.txt",
            ],
        );

        let prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();

        let prefix_record_clobber_1 = find_prefix_record(&prefix_records, "clobber-1");
        assert!(prefix_record_clobber_1.is_none());
        let prefix_record_clobber_2 = find_prefix_record(&prefix_records, "clobber-2").unwrap();
        assert_yaml_snapshot!(prefix_record_clobber_2.files);
        assert_yaml_snapshot!(prefix_record_clobber_2.paths_data);

        assert_eq!(
            fs::read_to_string(target_prefix.path().join("clobber.txt")).unwrap(),
            "clobber-2\n"
        );
        assert_eq!(
            fs::read_to_string(target_prefix.path().join("another-clobber.txt")).unwrap(),
            "clobber-2\n"
        );
        let prefix_record_clobber_3 = find_prefix_record(&prefix_records, "clobber-3").unwrap();
        assert!(
            prefix_record_clobber_3.files
                == vec![
                    PathBuf::from("another-clobber.txt__clobber-from-clobber-3"),
                    PathBuf::from("clobber.txt__clobber-from-clobber-3")
                ]
        );

        assert_eq!(prefix_record_clobber_3.paths_data.paths.len(), 2);
        assert_yaml_snapshot!(prefix_record_clobber_3.paths_data);
    }

    #[tokio::test]
    async fn test_random_clobber() {
        for _ in 0..3 {
            let mut operations = test_operations();
            // randomize the order of the operations
            operations.shuffle(&mut rand::thread_rng());

            let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
                operations,
                python_info: None,
                current_python_info: None,
                platform: Platform::current(),
            };

            // execute transaction
            let target_prefix = tempfile::tempdir().unwrap();

            let packages_dir = tempfile::tempdir().unwrap();
            let cache = PackageCache::new(packages_dir.path());

            execute_transaction(
                transaction,
                target_prefix.path(),
                &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
                &cache,
                &InstallDriver::default(),
                &InstallOptions::default(),
            )
            .await;

            assert_eq!(
                fs::read_to_string(target_prefix.path().join("clobber.txt")).unwrap(),
                "clobber-1\n"
            );

            // make sure that clobbers are resolved deterministically
            assert_check_files(
                target_prefix.path(),
                &[
                    "clobber.txt__clobber-from-clobber-3",
                    "clobber.txt__clobber-from-clobber-2",
                    "clobber.txt",
                    "another-clobber.txt__clobber-from-clobber-3",
                    "another-clobber.txt__clobber-from-clobber-2",
                    "another-clobber.txt",
                ],
            );

            let prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();

            for record in prefix_records {
                if record.repodata_record.package_record.name.as_normalized() == "clobber-1" {
                    assert_eq!(
                        record.files,
                        vec![
                            PathBuf::from("another-clobber.txt"),
                            PathBuf::from("clobber.txt")
                        ]
                    );
                } else if record.repodata_record.package_record.name.as_normalized() == "clobber-2"
                {
                    assert_eq!(
                        record.files,
                        vec![
                            PathBuf::from("another-clobber.txt__clobber-from-clobber-2"),
                            PathBuf::from("clobber.txt__clobber-from-clobber-2")
                        ]
                    );
                } else if record.repodata_record.package_record.name.as_normalized() == "clobber-3"
                {
                    assert_eq!(
                        record.files,
                        vec![
                            PathBuf::from("another-clobber.txt__clobber-from-clobber-3"),
                            PathBuf::from("clobber.txt__clobber-from-clobber-3")
                        ]
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn test_random_clobber_nested() {
        for _ in 0..3 {
            let mut operations = test_operations_nested();
            // randomize the order of the operations
            operations.shuffle(&mut rand::thread_rng());

            let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
                operations,
                python_info: None,
                current_python_info: None,
                platform: Platform::current(),
            };

            // execute transaction
            let target_prefix = tempfile::tempdir().unwrap();

            let packages_dir = tempfile::tempdir().unwrap();
            let cache = PackageCache::new(packages_dir.path());

            execute_transaction(
                transaction,
                target_prefix.path(),
                &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
                &cache,
                &InstallDriver::default(),
                &InstallOptions::default(),
            )
            .await;

            assert_eq!(
                fs::read_to_string(target_prefix.path().join("clobber/bobber/clobber.txt"))
                    .unwrap(),
                "clobber-2\n"
            );

            // make sure that clobbers are resolved deterministically
            assert_check_files(
                &target_prefix.path().join("clobber/bobber"),
                &[
                    "clobber.txt__clobber-from-clobber-nested-3",
                    "clobber.txt__clobber-from-clobber-nested-1",
                    "clobber.txt",
                ],
            );

            let prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();
            let prefix_record_clobber_2 =
                find_prefix_record(&prefix_records, "clobber-nested-2").unwrap();
            let prefix_record_clobber_3 =
                find_prefix_record(&prefix_records, "clobber-nested-3").unwrap();

            assert_eq!(
                prefix_record_clobber_3.files,
                vec![PathBuf::from(
                    "clobber/bobber/clobber.txt__clobber-from-clobber-nested-3"
                )]
            );

            // remove one of the clobbering files
            let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
                operations: vec![TransactionOperation::Remove(
                    prefix_record_clobber_2.clone(),
                )],
                python_info: None,
                current_python_info: None,
                platform: Platform::current(),
            };

            let install_driver = InstallDriver::new(100, Some(&prefix_records));

            execute_transaction(
                transaction,
                target_prefix.path(),
                &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
                &cache,
                &install_driver,
                &InstallOptions::default(),
            )
            .await;

            assert_check_files(
                &target_prefix.path().join("clobber/bobber"),
                &["clobber.txt__clobber-from-clobber-nested-3", "clobber.txt"],
            );

            assert_eq!(
                fs::read_to_string(target_prefix.path().join("clobber/bobber/clobber.txt"))
                    .unwrap(),
                "clobber-1\n"
            );

            let prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();
            let prefix_record_clobber_1 =
                find_prefix_record(&prefix_records, "clobber-nested-1").unwrap();

            assert_eq!(
                prefix_record_clobber_1.files,
                vec![PathBuf::from("clobber/bobber/clobber.txt")]
            );
        }
    }

    #[tokio::test]
    async fn test_clobber_update() {
        // Create a transaction
        let operations = test_operations();

        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations,
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        // execute transaction
        let target_prefix = tempfile::tempdir().unwrap();

        let packages_dir = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(packages_dir.path());

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &InstallDriver::default(),
            &InstallOptions::default(),
        )
        .await;

        // check that the files are there
        assert_check_files(
            target_prefix.path(),
            &[
                "clobber.txt",
                "clobber.txt__clobber-from-clobber-2",
                "clobber.txt__clobber-from-clobber-3",
                "another-clobber.txt",
                "another-clobber.txt__clobber-from-clobber-2",
                "another-clobber.txt__clobber-from-clobber-3",
            ],
        );

        let mut prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();
        prefix_records.sort_by(|a, b| {
            a.repodata_record
                .package_record
                .name
                .as_normalized()
                .cmp(&b.repodata_record.package_record.name.as_normalized())
        });

        let update_ops = test_operations_update();

        // remove one of the clobbering files
        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations: vec![TransactionOperation::Change {
                old: prefix_records[0].clone(),
                new: update_ops[0].clone(),
            }],
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        let install_driver = InstallDriver::new(100, Some(&prefix_records));

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &install_driver,
            &InstallOptions::default(),
        )
        .await;

        assert_check_files(
            target_prefix.path(),
            &[
                "clobber.txt",
                "clobber.txt__clobber-from-clobber-2",
                "clobber.txt__clobber-from-clobber-3",
                "another-clobber.txt__clobber-from-clobber-2",
                "another-clobber.txt__clobber-from-clobber-3",
            ],
        );

        // content of  clobber.txt
        assert_eq!(
            fs::read_to_string(target_prefix.path().join("clobber.txt")).unwrap(),
            "clobber-1 v2\n"
        );
    }

    #[tokio::test]
    async fn test_clobber_update_and_remove() {
        // Create a transaction
        let operations = test_operations();

        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations,
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        // execute transaction
        let target_prefix = tempfile::tempdir().unwrap();

        let packages_dir = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(packages_dir.path());

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &InstallDriver::default(),
            &InstallOptions::default(),
        )
        .await;

        // check that the files are there
        assert_check_files(
            target_prefix.path(),
            &[
                "clobber.txt",
                "clobber.txt__clobber-from-clobber-2",
                "clobber.txt__clobber-from-clobber-3",
                "another-clobber.txt",
                "another-clobber.txt__clobber-from-clobber-2",
                "another-clobber.txt__clobber-from-clobber-3",
            ],
        );

        let mut prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();
        prefix_records.sort_by(|a, b| {
            a.repodata_record
                .package_record
                .name
                .as_normalized()
                .cmp(&b.repodata_record.package_record.name.as_normalized())
        });

        let update_ops = test_operations_update();

        // remove one of the clobbering files
        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations: vec![
                TransactionOperation::Change {
                    old: prefix_records[2].clone(),
                    new: update_ops[2].clone(),
                },
                TransactionOperation::Remove(prefix_records[0].clone()),
                TransactionOperation::Remove(prefix_records[1].clone()),
            ],
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        let prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();
        let install_driver = InstallDriver::new(100, Some(&prefix_records));

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &install_driver,
            &InstallOptions::default(),
        )
        .await;

        assert_check_files(target_prefix.path(), &["clobber.txt"]);

        // content of  clobber.txt
        assert_eq!(
            fs::read_to_string(target_prefix.path().join("clobber.txt")).unwrap(),
            "clobber-3 v2\n"
        );

        let update_ops = test_operations_update();

        // remove one of the clobbering files
        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations: vec![TransactionOperation::Install(update_ops[0].clone())],
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        let prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();
        let install_driver = InstallDriver::new(100, Some(&prefix_records));

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &install_driver,
            &InstallOptions::default(),
        )
        .await;

        assert_check_files(
            target_prefix.path(),
            &["clobber.txt", "clobber.txt__clobber-from-clobber-3"],
        );

        // content of  clobber.txt
        assert_eq!(
            fs::read_to_string(target_prefix.path().join("clobber.txt")).unwrap(),
            "clobber-1 v2\n"
        );
    }

    #[tokio::test]
    async fn test_clobber_python_noarch() {
        // Create a transaction
        let operations = test_python_noarch_operations();

        let python_info =
            PythonInfo::from_version(&Version::from_str("3.11.0").unwrap(), Platform::current())
                .unwrap();
        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations,
            python_info: Some(python_info.clone()),
            current_python_info: Some(python_info.clone()),
            platform: Platform::current(),
        };

        // execute transaction
        let target_prefix = tempfile::tempdir().unwrap();

        let packages_dir = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(packages_dir.path());

        let mut install_options = InstallOptions::default();
        install_options.python_info = Some(python_info.clone());

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &InstallDriver::default(),
            &install_options,
        )
        .await;

        // check that the files are there
        if cfg!(unix) {
            assert_check_files(
                &target_prefix
                    .path()
                    .join("lib/python3.11/site-packages/clobber"),
                &["clobber.py", "clobber.py__clobber-from-clobber-pynoarch-2"],
            );
        } else {
            assert_check_files(
                &target_prefix.path().join("Lib/site-packages/clobber"),
                &["clobber.py", "clobber.py__clobber-from-clobber-pynoarch-2"],
            );
        }
    }

    // This used to hit an expect in the clobbering code
    #[tokio::test]
    async fn test_transaction_with_clobber_remove_all() {
        let operations = test_operations();

        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations,
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        // execute transaction
        let target_prefix = tempfile::tempdir().unwrap();

        let packages_dir = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(packages_dir.path());

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &InstallDriver::default(),
            &InstallOptions::default(),
        )
        .await;

        let prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();

        // remove one of the clobbering files
        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations: prefix_records
                .iter()
                .map(|r| TransactionOperation::Remove(r.clone()))
                .collect(),
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        let install_driver = InstallDriver::new(100, Some(&prefix_records));

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &install_driver,
            &InstallOptions::default(),
        )
        .await;

        assert_check_files(target_prefix.path(), &[]);
    }

    // This used to hit an expect in the clobbering code
    #[tokio::test]
    async fn test_dependency_clobber() {
        // Create a transaction
        let repodata_record_1 = get_repodata_record("clobber/clobber-python-0.1.0-cpython.tar.bz2");

        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations: vec![TransactionOperation::Install(repodata_record_1)],
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        // execute transaction
        let target_prefix = tempfile::tempdir().unwrap();

        let packages_dir = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(packages_dir.path());

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &InstallDriver::default(),
            &InstallOptions::default(),
        )
        .await;

        let prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();
        let repodata_record_2 = get_repodata_record("clobber/clobber-python-0.1.0-pypy.tar.bz2");
        let repodata_record_3 =
            get_repodata_record("clobber/clobber-pypy-0.1.0-h4616a5c_0.tar.bz2");

        // remove one of the clobbering files
        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations: vec![
                TransactionOperation::Remove(prefix_records[0].clone()),
                TransactionOperation::Install(repodata_record_2),
                TransactionOperation::Install(repodata_record_3),
            ],
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        let install_driver = InstallDriver::new(100, Some(&prefix_records));

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &install_driver,
            &InstallOptions::default(),
        )
        .await;

        let path = target_prefix.into_path();
        println!("{:?}", path);

        // assert_check_files(target_prefix.path(), &["bin/python"]);
        assert_check_files(&path, &["bin/python"]);
    }
}
