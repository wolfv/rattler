//! Functions for running link scripts (pre-unlink and post-link) for a package
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use rattler_conda_types::{PackageRecord, Platform, PrefixRecord, RepoDataRecord};
use rattler_shell::shell::{Bash, CmdExe, ShellEnum};

use super::{InstallDriver, Transaction};

/// Error type for link script errors
#[derive(Debug, thiserror::Error)]
pub enum LinkScriptError {
    /// An error occurred while reading the message file
    #[error("Error reading message file: {0}")]
    MessageError(#[from] std::io::Error),
}

/// The type of link script to run
pub enum LinkScriptType {
    /// The pre-unlink script (run before the package is unlinked)
    /// This is stored in the environment as `bin/.{name}-pre-unlink.sh` or `Scripts/.{name}-pre-unlink.bat`
    PreUnlink,
    /// The post-link script (run after the package is linked)
    /// This is stored in the environment as `bin/.{name}-post-link.sh` or `Scripts/.{name}-post-link.bat`
    PostLink,
}

impl LinkScriptType {
    /// Get the path to the link script for a given package record and platform
    pub fn get_path(&self, package_record: &PackageRecord, platform: &Platform) -> String {
        let name = &package_record.name.as_normalized();
        if platform.is_windows() {
            match self {
                LinkScriptType::PreUnlink => {
                    format!("Scripts/.{name}-pre-unlink.bat")
                }
                LinkScriptType::PostLink => {
                    format!("Scripts/.{name}-post-link.bat")
                }
            }
        } else {
            match self {
                LinkScriptType::PreUnlink => {
                    format!("bin/.{name}-pre-unlink.sh")
                }
                LinkScriptType::PostLink => {
                    format!("bin/.{name}-post-link.sh")
                }
            }
        }
    }
}

impl ToString for LinkScriptType {
    fn to_string(&self) -> String {
        match self {
            LinkScriptType::PreUnlink => "pre-unlink".to_string(),
            LinkScriptType::PostLink => "post-link".to_string(),
        }
    }
}

/// Run the link scripts for a given package
pub fn run_link_scripts<'a>(
    link_script_type: LinkScriptType,
    prefix_records: impl Iterator<Item = &'a PrefixRecord>,
    target_prefix: &Path,
    platform: &Platform,
) -> Result<(), LinkScriptError> {
    let mut env = HashMap::new();
    env.insert(
        "PREFIX".to_string(),
        target_prefix.to_string_lossy().to_string(),
    );

    // prefix records are topologically sorted, so we can be sure that all dependencies are
    // installed before the package itself.
    for record in prefix_records {
        let prec = &record.repodata_record.package_record;
        let link_file = target_prefix.join(&link_script_type.get_path(prec, platform));

        if link_file.exists() {
            env.insert(
                "PKG_NAME".to_string(),
                prec.name.as_normalized().to_string(),
            );
            env.insert("PKG_VERSION".to_string(), prec.version.to_string());
            env.insert("PKG_BUILDNUM".to_string(), prec.build_number.to_string());

            let shell = if platform.is_windows() {
                ShellEnum::CmdExe(CmdExe)
            } else {
                ShellEnum::Bash(Bash)
            };

            tracing::info!(
                "Running {} script for {}",
                link_script_type.to_string(),
                prec.name.as_normalized()
            );

            match rattler_shell::run_in_environment(
                target_prefix,
                &[&link_file.to_string_lossy()],
                shell,
                &env,
            ) {
                Ok(o) if o.status.success() => {}
                Ok(o) => {
                    tracing::warn!("Error running post-link script. Status: {:?}", o.status);
                    tracing::warn!("  stdout: {}", String::from_utf8_lossy(&o.stdout));
                    tracing::warn!("  stderr: {}", String::from_utf8_lossy(&o.stderr));
                }
                Err(e) => {
                    tracing::error!("Error running post-link script: {:?}", e);
                }
            }

            let message_file = target_prefix.join(".messages.txt");
            if message_file.exists() {
                let message = std::fs::read_to_string(&message_file)?;
                tracing::info!(
                    "Message from {} for {}: {}",
                    link_script_type.to_string(),
                    prec.name.as_normalized(),
                    message
                );
                // Remove the message file
                std::fs::remove_file(&message_file)?;
            }
        }
    }

    Ok(())
}

impl InstallDriver {
    /// Run any post-link scripts that are part of the packages that are being installed.
    pub fn run_post_link_scripts(
        &self,
        transaction: &Transaction<PrefixRecord, RepoDataRecord>,
        prefix_records: &[&PrefixRecord],
        target_prefix: &Path,
    ) -> Result<(), LinkScriptError> {
        let to_install = transaction
            .installed_packages()
            .map(|r| &r.package_record.name)
            .collect::<HashSet<_>>();

        let filter_iter = prefix_records
            .iter()
            .filter(|r| to_install.contains(&r.repodata_record.package_record.name))
            .cloned();

        run_link_scripts(
            LinkScriptType::PostLink,
            filter_iter,
            target_prefix,
            &transaction.platform,
        )
    }

    /// Run any post-link scripts that are part of the packages that are being installed.
    pub fn run_pre_unlink_scripts(
        &self,
        transaction: &Transaction<PrefixRecord, RepoDataRecord>,
        target_prefix: &Path,
    ) -> Result<(), LinkScriptError> {
        run_link_scripts(
            LinkScriptType::PreUnlink,
            transaction.removed_packages(),
            target_prefix,
            &transaction.platform,
        )
    }
}

#[cfg(test)]
mod tests {
    use rattler_conda_types::{Platform, PrefixRecord, RepoDataRecord};

    use crate::{
        get_repodata_record,
        install::{
            test_utils::execute_transaction, transaction, InstallDriver, InstallOptions,
            TransactionOperation,
        },
        package_cache::PackageCache,
    };

    fn test_operations() -> Vec<TransactionOperation<PrefixRecord, RepoDataRecord>> {
        let repodata_record_1 =
            get_repodata_record("link-scripts/link-scripts-0.1.0-h4616a5c_0.tar.bz2");

        vec![TransactionOperation::Install(repodata_record_1)]
    }

    #[tokio::test]
    async fn test_run_link_scripts() {
        let target_prefix = tempfile::tempdir().unwrap();

        let operations = test_operations();

        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations,
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

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

        // check that the post-link script was run
        assert!(target_prefix.path().join("i-was-post-linked").exists());

        // unlink the package
        let prefix_records = PrefixRecord::collect_from_prefix(target_prefix.path()).unwrap();
        let transaction = transaction::Transaction::<PrefixRecord, RepoDataRecord> {
            operations: vec![TransactionOperation::Remove(prefix_records[0].clone())],
            python_info: None,
            current_python_info: None,
            platform: Platform::current(),
        };

        execute_transaction(
            transaction,
            target_prefix.path(),
            &reqwest_middleware::ClientWithMiddleware::from(reqwest::Client::new()),
            &cache,
            &InstallDriver::default(),
            &InstallOptions::default(),
        )
        .await;

        // check that the pre-unlink script was run
        assert!(!target_prefix.path().join("i-was-post-linked").exists());
    }
}
