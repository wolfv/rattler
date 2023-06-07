//! Contains business logic to retrieve the results from libsolv after attempting to resolve a conda
//! environment

use libsolv_rs::pool::{Pool, RepoId};
use libsolv_rs::solvable::SolvableId;
use libsolv_rs::solver::{Transaction, TransactionKind};
use rattler_conda_types::RepoDataRecord;
use std::collections::HashMap;

/// Returns which packages should be installed in the environment
///
/// If the transaction contains libsolv operations that are not "install" an error is returned
/// containing their ids.
pub fn get_required_packages(
    pool: &Pool,
    repo_mapping: &HashMap<RepoId, usize>,
    transaction: &Transaction,
    repodata_records: &[&[RepoDataRecord]],
) -> Result<Vec<RepoDataRecord>, Vec<TransactionKind>> {
    let mut required_packages = Vec::new();
    let mut unsupported_operations = Vec::new();

    for &(id, kind) in &transaction.steps {
        // Retrieve the repodata record corresponding to this solvable
        let (repo_index, solvable_index) = get_solvable_indexes(pool, repo_mapping, id);
        let repodata_record = &repodata_records[repo_index][solvable_index];

        match kind {
            TransactionKind::Install => {
                required_packages.push(repodata_record.clone());
            }
            _ => {
                unsupported_operations.push(kind);
            }
        }
    }

    if !unsupported_operations.is_empty() {
        return Err(unsupported_operations);
    }

    Ok(required_packages)
}

fn get_solvable_indexes(
    pool: &Pool,
    repo_mapping: &HashMap<RepoId, usize>,
    id: SolvableId,
) -> (usize, usize) {
    let solvable = pool.resolve_solvable(id);
    let solvable_index = solvable.package().metadata.original_index.unwrap();

    let repo_id = solvable.repo_id();
    let repo_index = repo_mapping[&repo_id];

    (repo_index, solvable_index)
}
