use crate::pool::{MatchSpecId, Pool};
use crate::rules::Rule;
use crate::solvable::SolvableId;
use crate::solve_problem::SolveProblem;
use rattler_conda_types::MatchSpec;
use std::collections::{HashSet, VecDeque};
use std::fmt::{Display, Formatter};
use crate::solve_jobs::{CandidateSource, SolveJobs, SolveOperation};

pub struct Transaction {
    pub steps: Vec<(SolvableId, TransactionKind)>,
}

#[derive(Copy, Clone, Debug)]
pub enum TransactionKind {
    Install,
}

impl Display for TransactionKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub struct Config {
    pub allow_name_change: bool,
    pub allow_uninstall: bool,
    pub allow_downgrade: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            allow_name_change: true,
            allow_uninstall: false,
            allow_downgrade: false,
        }
    }
}

// Not sure what this is... Maybe a bitmap?
struct Map {}

impl Map {
    fn with_capacity(n: usize) -> Self {
        Map {}
    }
}

pub struct Solver {
    config: Config,
    pool: Pool,

    // The job we are solving
    job: VecDeque<()>,

    // All rules
    rules: Vec<Rule>,

    // All assertion rules
    rule_assertions: VecDeque<()>,

    recommendsmap: Map,
    suggestsmap: Map,
    noupdate: Map,

    /// Map of all available solvables
    ///
    /// = 0: undecided
    /// > 0: level of decision when installed
    /// < 0: level of decision when conflict
    decision_map: Vec<u64>,

    /* list of lists of conflicting rules, < 0 for job rules */
    problems: VecDeque<()>,
}

pub mod flags {
    pub const SOLVER_SOLVABLE: u32 = 1;
    pub const SOLVER_SOLVABLE_PROVIDES: u32 = 3;
    pub const SOLVER_TRANSACTION_INSTALL: u32 = 32;
    pub const SOLVER_INSTALL: u32 = 256;
    pub const SOLVER_ERASE: u32 = 512;
    pub const SOLVER_UPDATE: u32 = 768;
    pub const SOLVER_FAVOR: u32 = 3072;
    pub const SOLVER_DISFAVOR: u32 = 3328;
    pub const SOLVER_LOCK: u32 = 1536;
}

// Solver flags used
// * SOLVER_FLAG_ALLOW_UNINSTALL
// * SOLVER_FLAG_ALLOW_DOWNGRADE
// -> All other flags are unused

impl Solver {
    /// Create a solver, using the provided pool
    pub fn create(pool: Pool) -> Self {
        // TODO: the solver is initialized with the `learnt_pool` with a single element 0: "so that 0 does not describe a proof"

        Solver {
            job: VecDeque::new(),
            rules: Vec::new(),
            rule_assertions: VecDeque::new(),
            recommendsmap: Map::with_capacity(pool.nsolvables()),
            suggestsmap: Map::with_capacity(pool.nsolvables()),
            noupdate: Map::with_capacity(42),
            decision_map: vec![0; pool.nsolvables()],
            problems: VecDeque::new(),

            config: Config::default(),
            pool,
        }
    }

    /// Retrieve the solver's configuration in order to modify it
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    fn problem_count(&self) -> usize {
        self.problems.len() / 2
    }

    /// Creates a string for each 'problem' that the solver still has which it encountered while
    /// solving the matchspecs. Use this function to print the existing problems to string.
    fn solver_problems(&self) -> Vec<String> {
        let mut output = Vec::new();

        // See solver_problem_count
        let count = self.problem_count();
        for _ in 1..=count {
            // Safe because the id valid (between [1, count])
            // let problem = unsafe { self.problem2str(i as ffi::Id) };

            output.push("TODO".to_string());
        }
        output
    }

    pub fn all_solver_problems(&self) -> Vec<SolveProblem> {
        todo!()

        // let mut problems = Vec::new();
        // let mut problem_rules = VecDeque::<Id>::default();
        // let count = self.problem_count();
        // for i in 1..=count {
        //     unsafe {
        //         ffi::solver_findallproblemrules(
        //             self.raw_ptr(),
        //             i.try_into().unwrap(),
        //             problem_rules.raw_ptr(),
        //         )
        //     };
        //     for r in problem_rules.id_iter() {
        //         if r != 0 {
        //             let mut source_id = 0;
        //             let mut target_id = 0;
        //             let mut dep_id = 0;
        //
        //             let problem_type = unsafe {
        //                 ffi::solver_ruleinfo(
        //                     self.raw_ptr(),
        //                     r,
        //                     &mut source_id,
        //                     &mut target_id,
        //                     &mut dep_id,
        //                 )
        //             };
        //
        //             let pool = unsafe { (*self.0.as_ptr()).pool as *mut ffi::Pool };
        //
        //             let nsolvables = unsafe { (*pool).nsolvables };
        //
        //             let target = if target_id < 0 || target_id >= nsolvables as i32 {
        //                 None
        //             } else {
        //                 Some(SolvableId(target_id))
        //             };
        //
        //             let source = if source_id < 0 || source_id >= nsolvables as i32 {
        //                 None
        //             } else {
        //                 Some(SolvableId(target_id))
        //             };
        //
        //             let dep = if dep_id == 0 {
        //                 None
        //             } else {
        //                 let dep = unsafe { ffi::pool_dep2str(pool, dep_id) };
        //                 let dep = unsafe { CStr::from_ptr(dep) };
        //                 let dep = dep.to_str().expect("Invalid UTF8 value").to_string();
        //                 Some(dep)
        //             };
        //
        //             problems.push(SolveProblem::from_raw(problem_type, dep, source, target));
        //         }
        //     }
        // }
        // problems
    }

    /// Solves all the problems in the `queue` and returns a transaction from the found solution.
    /// Returns an error if problems remain unsolved.
    pub fn solve(&mut self, jobs: SolveJobs) -> Result<Transaction, Vec<String>> {
        let mut visited_solvables = HashSet::default();

        // Initialize the root solvable with the requested packages as dependencies
        self.pool.root_solvable_mut().clear();
        for (source, op) in jobs.items {
            if let (SolveOperation::Install, CandidateSource::MatchSpec(match_spec)) = (op, source)
            {
                let match_spec_id = self.pool.intern_matchspec(match_spec.to_string());
                let root_solvable = self.pool.root_solvable_mut();
                root_solvable.push(match_spec_id);

                // Recursively add rules for the current dep
                self.add_rules_for_root_dep(&mut visited_solvables, match_spec_id);
            } else {
                panic!("Unsupported operation or candidate source")
            }
        }

        // Run SAT
        self.run_sat();

        if self.problem_count() == 0 {
            Ok(Transaction { steps: Vec::new() })
        } else {
            Err(self.solver_problems())
        }
    }

    // fn add_pkg_rules_for_solvable(&mut self, solvable: SolvableId) {
    //     let match_spec = match self.pool.resolve_solvable(solvable) {
    //         Solvable::Package(_) => panic!("only match spec solvables generate rules (?)"),
    //         Solvable::MatchSpec(match_spec) => match_spec.clone(),
    //     };
    //
    //     let candidates = self.pool.get_candidates(&match_spec);
    //     if candidates.is_empty() {
    //         panic!("No candidates for match spec: {match_spec}!");
    //     }
    //
    //     self.add_requires_rule(solvable, candidates);
    //
    //     // Add constrains when adding rules for package
    // }

    fn add_rules_for_root_dep(&mut self, visited: &mut HashSet<SolvableId>, dep: MatchSpecId) {
        let mut candidate_stack = Vec::new();

        // Initialize candidate stack
        {
            let candidates = Pool::get_candidates(
                &self.pool.match_specs,
                &self.pool.strings_to_ids,
                &self.pool.solvables,
                &self.pool.packages_by_name,
                &mut self.pool.match_spec_to_candidates,
                dep,
            );
            for &candidate in candidates {
                if visited.insert(candidate) {
                    candidate_stack.push(candidate);
                }
            }
        }

        // Process candidates, adding them recursively
        while let Some(candidate) = candidate_stack.pop() {
            let solvable = self.pool.solvables[candidate.index()].package();

            // Requires
            for &dep in &solvable.dependencies {
                let dep_candidates = Pool::get_candidates(
                    &self.pool.match_specs,
                    &self.pool.strings_to_ids,
                    &self.pool.solvables,
                    &self.pool.packages_by_name,
                    &mut self.pool.match_spec_to_candidates,
                    dep,
                );

                // Create requires rule
                self.rules.push(Rule::Requires(candidate, dep));

                // Ensure the candidates have their rules added too
                for &dep_candidate in dep_candidates {
                    if visited.insert(dep_candidate) {
                        candidate_stack.push(dep_candidate);
                    }
                }
            }

            // Constrains
            for &dep in &solvable.constrains {
                self.rules.push(Rule::Constrains(candidate, dep));

                // It is not necessary to create rules for the packages that match a `constrains`
                // match spec, because they are not a dependency
            }
        }
    }

    fn run_sat(&mut self) {}
}
