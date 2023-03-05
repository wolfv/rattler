use crate::libsolv::input::{add_repodata_records, add_virtual_packages};
use crate::libsolv::output::get_required_packages;
use crate::libsolv::wrapper::repo::Repo;
use crate::{SolveError, SolverBackend, SolverProblem};
use rattler_conda_types::RepoDataRecord;
use std::collections::HashMap;
use wrapper::{
    flags::SolverFlag,
    pool::{Pool, Verbosity},
    solve_goal::SolveGoal,
};

mod input;
mod output;
mod wrapper;

/// A [`SolverBackend`] implemented using the `libsolv` library
pub struct LibsolvBackend;

impl SolverBackend for LibsolvBackend {
    fn solve(&mut self, problem: SolverProblem) -> Result<Vec<RepoDataRecord>, SolveError> {
        // Construct a default libsolv pool
        let pool = Pool::default();

        // Setup proper logging for the pool
        pool.set_debug_callback(|msg, flags| {
            tracing::event!(tracing::Level::DEBUG, flags, "{}", msg);
        });
        pool.set_debug_level(Verbosity::Low);

        // Add virtual packages
        let repo = Repo::new(&pool, "virtual_packages");
        add_virtual_packages(&pool, &repo, &problem.virtual_packages)
            .map_err(SolveError::ErrorAddingInstalledPackages)?;

        // Mark the virtual packages as installed.
        pool.set_installed(&repo);

        // Create repos for all channels
        let mut repo_mapping = HashMap::with_capacity(problem.available_packages.len() + 1);
        let mut all_repodata_records = Vec::with_capacity(repo_mapping.len());
        for repodata_records in &problem.available_packages {
            if repodata_records.is_empty() {
                continue;
            }

            let channel_name = &repodata_records[0].channel;
            let repo = Repo::new(&pool, channel_name);
            add_repodata_records(&pool, &repo, repodata_records)
                .map_err(SolveError::ErrorAddingRepodata)?;

            // Keep our own info about repodata_records
            repo_mapping.insert(repo.id(), repo_mapping.len());
            all_repodata_records.push(repodata_records.as_slice());

            // We dont want to drop the Repo, its stored in the pool anyway, so just forget it.
            std::mem::forget(repo);
        }

        // Create a special pool for records that are already installed or locked.
        let repo = Repo::new(&pool, "locked");
        let installed_solvables = add_repodata_records(&pool, &repo, &problem.locked_packages)
            .map_err(SolveError::ErrorAddingRepodata)?;

        // Also add the installed records to the repodata
        repo_mapping.insert(repo.id(), repo_mapping.len());
        all_repodata_records.push(problem.locked_packages.as_slice());

        // Create a special pool for records that are pinned and cannot be changed.
        let repo = Repo::new(&pool, "pinned");
        let pinned_solvables = add_repodata_records(&pool, &repo, &problem.pinned_packages)
            .map_err(SolveError::ErrorAddingRepodata)?;

        // Also add the installed records to the repodata
        repo_mapping.insert(repo.id(), repo_mapping.len());
        all_repodata_records.push(problem.pinned_packages.as_slice());

        // Create datastructures for solving
        pool.create_whatprovides();

        // Add matchspec to the queue
        let mut goal = SolveGoal::default();

        // Favor the currently installed packages
        for favor_solvable in installed_solvables {
            goal.favor(favor_solvable);
        }

        // Lock the currently pinned packages
        for locked_solvable in pinned_solvables {
            goal.lock(locked_solvable);
        }

        // Specify the matchspec requests
        for spec in problem.specs {
            let id = pool.intern_matchspec(&spec);
            goal.install(id, false)
        }

        // Construct a solver and solve the problems in the queue
        let mut solver = pool.create_solver();
        solver.set_flag(SolverFlag::allow_uninstall(), true);
        solver.set_flag(SolverFlag::allow_downgrade(), true);

        let transaction = match solver.solve(&mut goal) {
            Ok(transaction) => transaction,
            Err(_) => {
                return Err(SolveError::Unsolvable);
            }
        };

        let required_records =
            get_required_packages(&pool, &repo_mapping, &transaction, &all_repodata_records)
                .map_err(|unsupported_operation_ids| {
                    SolveError::UnsupportedOperations(
                        unsupported_operation_ids
                            .into_iter()
                            .map(|id| format!("libsolv operation {id}"))
                            .collect(),
                    )
                })?;

        Ok(required_records)
    }
}
