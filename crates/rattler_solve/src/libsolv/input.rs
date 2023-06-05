//! Contains business logic that loads information into libsolv in order to solve a conda
//! environment

use crate::libsolv::conda_version;
use libsolv_rs::pool::keys::*;
use libsolv_rs::pool::{Id, Pool};
use rattler_conda_types::package::ArchiveType;
use rattler_conda_types::{GenericVirtualPackage, MatchSpec, RepoDataRecord};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::str::FromStr;

/// Adds [`RepoDataRecord`] to `repo`
///
/// Panics if the repo does not belong to the pool
pub fn add_repodata_records(
    pool: &mut Pool,
    repo_id: Id,
    repo_datas: &[RepoDataRecord],
) -> Vec<Id> {
    // Get all the IDs (these strings are internal to libsolv and always present, so we can
    // unwrap them at will)
    let solvable_buildflavor_id = pool.find_interned_str(SOLVABLE_BUILDFLAVOR).unwrap();
    let solvable_buildtime_id = pool.find_interned_str(SOLVABLE_BUILDTIME).unwrap();
    let solvable_buildversion_id = pool.find_interned_str(SOLVABLE_BUILDVERSION).unwrap();
    let solvable_constraints = pool.find_interned_str(SOLVABLE_CONSTRAINS).unwrap();
    let solvable_download_size_id = pool.find_interned_str(SOLVABLE_DOWNLOADSIZE).unwrap();
    let solvable_license_id = pool.find_interned_str(SOLVABLE_LICENSE).unwrap();
    let solvable_pkg_id = pool.find_interned_str(SOLVABLE_PKGID).unwrap();
    let solvable_checksum = pool.find_interned_str(SOLVABLE_CHECKSUM).unwrap();
    let solvable_track_features = pool.find_interned_str(SOLVABLE_TRACK_FEATURES).unwrap();

    // Keeps a mapping from packages added to the repo to the type and solvable
    let mut package_to_type: HashMap<&str, (ArchiveType, Id)> = HashMap::new();

    let mut solvable_ids = Vec::new();
    for (repo_data_index, repo_data) in repo_datas.iter().enumerate() {
        // Create a solvable for the package
        let solvable_id =
            match add_or_reuse_solvable(pool, repo_id, &mut package_to_type, repo_data) {
                Some(id) => id,
                None => continue,
            };

        // Store the current index so we can retrieve the original repo data record
        // from the final transaction
        pool.resolve_solvable_mut(solvable_id)
            .metadata
            .original_index = Some(repo_data_index);

        let record = &repo_data.package_record;

        // Intern stuff
        let build_str_id = pool.intern_str(&record.build);

        // Name and version
        let name = pool.intern_str(record.name.as_str()).into();
        let evr = pool.intern_str(record.version.to_string()).into();
        let rel_eq = pool.rel_eq(name, evr);
        {
            let solvable = pool.resolve_solvable_mut(solvable_id);
            solvable.name = name;
            solvable.evr = evr;
            pool.add_provides(repo_id, solvable_id, rel_eq);
        }

        // Dependencies
        for match_spec in record.depends.iter() {
            // Create a reldep id from a matchspec
            let match_spec = MatchSpec::from_str(&match_spec).unwrap();
            let match_spec_id =
                pool.conda_matchspec(&match_spec.name.unwrap(), conda_version(match_spec.version));

            // Add it to the list of requirements of this solvable
            pool.add_requires(repo_id, solvable_id, match_spec_id);
        }

        // Constraints
        for match_spec in record.constrains.iter() {
            // Create a reldep id from a matchspec
            let match_spec = MatchSpec::from_str(&match_spec).unwrap();
            let match_spec_id =
                pool.conda_matchspec(&match_spec.name.unwrap(), conda_version(match_spec.version));

            // Add it to the list of constraints of this solvable
            pool.repo_mut(repo_id).repodata_mut().add_idarray(
                solvable_id,
                solvable_constraints,
                match_spec_id,
            );
        }

        // Track features
        for track_features in record.track_features.iter() {
            let track_feature = track_features.trim();
            if !track_feature.is_empty() {
                let string_id = pool.intern_str(track_features.trim()).into();
                pool.repo_mut(repo_id).repodata_mut().add_idarray(
                    solvable_id,
                    solvable_track_features,
                    string_id,
                );
            }
        }

        // License
        if let Some(license) = record.license.as_ref() {
            let license_id = pool.intern_str(license);
            pool.repo_mut(repo_id).repodata_mut().add_idarray(
                solvable_id,
                solvable_license_id,
                license_id,
            );
        }

        // Timestamp
        let data = pool.repo_mut(repo_id).repodata_mut();
        if let Some(timestamp) = record.timestamp {
            data.set_num(
                solvable_id,
                solvable_buildtime_id,
                timestamp.timestamp() as u64,
            );
        }

        // Size
        if let Some(size) = record.size {
            data.set_num(solvable_id, solvable_download_size_id, size);
        }

        // Build string
        data.add_idarray(solvable_id, solvable_buildflavor_id, build_str_id);

        // Build number
        data.set_str(
            solvable_id,
            solvable_buildversion_id,
            &record.build_number.to_string(),
        );

        // MD5 hash
        if let Some(md5) = record.md5.as_ref() {
            data.set_checksum(solvable_id, solvable_pkg_id, &format!("{:x}", md5));
        }

        // Sha256 hash
        if let Some(sha256) = record.sha256.as_ref() {
            data.set_checksum(solvable_id, solvable_checksum, &format!("{:x}", sha256));
        }

        solvable_ids.push(solvable_id)
    }

    pool.repo_mut(repo_id).internalize();

    solvable_ids
}

/// When adding packages, we want to make sure that `.conda` packages have preference over `.tar.bz`
/// packages. For that reason, when adding a solvable we check first if a `.conda` version of the
/// package has already been added, in which case we forgo adding its `.tar.bz` version (and return
/// `None`). If no `.conda` version has been added, we create a new solvable (replacing any existing
/// solvable for the `.tar.bz` version of the package).
fn add_or_reuse_solvable<'a>(
    pool: &mut Pool,
    repo_id: Id,
    package_to_type: &mut HashMap<&'a str, (ArchiveType, Id)>,
    repo_data: &'a RepoDataRecord,
) -> Option<Id> {
    // Sometimes we can reuse an existing solvable
    if let Some((filename, archive_type)) = ArchiveType::split_str(&repo_data.file_name) {
        if let Some(&(other_package_type, old_solvable_id)) = package_to_type.get(filename) {
            match archive_type.cmp(&other_package_type) {
                Ordering::Less => {
                    // A previous package that we already stored is actually a package of a better
                    // "type" so we'll just use that instead (.conda > .tar.bz)
                    return None;
                }
                Ordering::Greater => {
                    // A previous package has a worse package "type", we'll reuse the handle but
                    // overwrite its attributes

                    // Update the package to the new type mapping
                    package_to_type.insert(filename, (archive_type, old_solvable_id));

                    // Reset and reuse the old solvable
                    reset_solvable(pool, repo_id, old_solvable_id);
                    return Some(old_solvable_id);
                }
                Ordering::Equal => {
                    unreachable!("found a duplicate package")
                }
            }
        } else {
            let solvable_id = pool.add_solvable(repo_id);
            package_to_type.insert(filename, (archive_type, solvable_id));
            return Some(solvable_id);
        }
    } else {
        tracing::warn!("unknown package extension: {}", &repo_data.file_name);
    }

    Some(pool.add_solvable(repo_id))
}

pub fn add_virtual_packages(pool: &mut Pool, repo_id: Id, packages: &[GenericVirtualPackage]) {
    let solvable_buildflavor_id = pool.find_interned_str(SOLVABLE_BUILDFLAVOR).unwrap();

    for package in packages {
        let name = pool.intern_str(package.name.as_str()).into();
        let evr = pool.intern_str(package.version.to_string()).into();
        let rel_eq = pool.rel_eq(name, evr);

        // Create a solvable for the package
        let solvable_id = pool.add_solvable(repo_id);
        let solvable = pool.resolve_solvable_mut(solvable_id);

        // Name and version
        solvable.name = name;
        solvable.evr = evr;
        pool.add_provides(repo_id, solvable_id, rel_eq);

        // Build string
        let build_str_id = pool.intern_str(&package.build_string);
        pool.repo_mut(repo_id).repodata_mut().add_idarray(
            solvable_id,
            solvable_buildflavor_id,
            build_str_id,
        );
    }
}

fn reset_solvable(pool: &mut Pool, repo_id: Id, solvable_id: Id) {
    let blank_solvable = pool.add_solvable(repo_id);

    // Replace the existing solvable with the blank one
    pool.swap_solvables(blank_solvable, solvable_id);
    pool.repo_mut(repo_id)
        .repodata_mut()
        .swap_attrs(blank_solvable, solvable_id);

    // Remove the highest solvable, which contains stale data
    pool.pop_solvable(repo_id);
}
