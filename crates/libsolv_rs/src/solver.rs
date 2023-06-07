use crate::pool::{MatchSpecId, Pool};
use crate::rules::{Rule, RuleKind};
use crate::solvable::SolvableId;
use crate::solve_jobs::{CandidateSource, SolveJobs, SolveOperation};
use crate::solve_problem::SolveProblem;
use std::collections::{HashSet, VecDeque};
use std::fmt::{Display, Formatter};

#[derive(Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
pub struct RuleId(u32);

impl RuleId {
    pub fn new(index: usize) -> Self {
        Self(index as u32)
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum Decision {
    Decided(Decided),
    Undecided,
}

impl Decision {
    fn decided(self) -> Decided {
        match self {
            Decision::Decided(d) => d,
            Decision::Undecided => panic!("undecided!"),
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
struct Decided {
    solvable: SolvableId,
    value: bool
}

impl Decided {
    fn new(solvable: SolvableId, value: bool) -> Self {
        Self {
            solvable,
            value
        }
    }

    fn negate(mut self) -> Self {
        self.value = !self.value;
        self
    }

    fn index(self) -> usize {
        self.solvable.index()
    }
}

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

    propagate_index: usize,

    rules: Vec<Rule>,
    watches: Vec<RuleId>,

    // All assertion rules
    rule_assertions: VecDeque<RuleId>,

    decision_queue: VecDeque<Decision>,
    decision_queue_why: VecDeque<RuleId>,
    decision_queue_reason: VecDeque<u32>,

    learnt_rules_start: RuleId,

    recommendsmap: Map,
    suggestsmap: Map,
    noupdate: Map,

    /// Map of all available solvables
    ///
    /// = 0: undecided
    /// > 0: level of decision when installed
    /// < 0: level of decision when conflict
    decision_map: Vec<i64>,

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

            propagate_index: 0,

            rules: vec![Rule::new(RuleKind::InstallRoot)],
            watches: Vec::new(),
            rule_assertions: VecDeque::from([RuleId::new(0)]),
            decision_queue: VecDeque::new(),
            decision_queue_why: VecDeque::new(),
            decision_queue_reason: VecDeque::new(),

            learnt_rules_start: RuleId(0),

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

        // All new rules are learnt after this point
        self.learnt_rules_start = RuleId::new(self.rules.len());

        // Create watches chains
        self.make_watches();

        // Create assertion index. It is only used to speed up make_rule_decisions() a bit
        for (i, rule) in self.rules.iter().enumerate().skip(1) {
            if rule.is_assertion(self.pool()) {
                self.rule_assertions.push_back(RuleId::new(i));
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
                self.rules
                    .push(Rule::new(RuleKind::Requires(candidate, dep)));

                // Ensure the candidates have their rules added too
                for &dep_candidate in dep_candidates {
                    if visited.insert(dep_candidate) {
                        candidate_stack.push(dep_candidate);
                    }
                }
            }

            // Constrains
            for &dep in &solvable.constrains {
                self.rules
                    .push(Rule::new(RuleKind::Constrains(candidate, dep)));

                // It is not necessary to create rules for the packages that match a `constrains`
                // match spec, because they are not a dependency
            }
        }
    }

    fn run_sat(&mut self) {
        // This thng is true, right?
        let disable_rules = true;

        // let mut decision_queue = VecDeque::new();
        let mut level = 0;

        // What is this again?
        let mut root_level = 1;

        loop {
            // First rule decision
            if level == 0 {
                match self.make_rule_decisions() {
                    Ok(new_level) => level = new_level,
                    Err(_) => break,
                }

                if let Err(cause) = self.propagate(level) {
                    self.analyze_unsolvable(cause, false);
                    continue;
                }

                root_level = level + 1;
            }
        }
    }

    fn make_rule_decisions(&mut self) -> Result<u32, ()> {
        assert!(self.decision_queue.is_empty());

        // The root solvable is installed first
        self.decision_queue.push_back(Decision::Decided(Decided::new(SolvableId::root(), true)));
        self.decision_queue_why.push_back(RuleId::new(0));
        self.decision_queue_reason.push_back(0);
        self.decision_queue_reason.push_back(0);
        self.decision_map[SolvableId::root().index()] = 1; // Installed at level 1

        let mut have_disabled = false;

        let decision_start = 1;
        loop {
            // If we needed to re-run, back up decisions to decision_start
            while self.decision_queue.len() > decision_start {
                let decision = self.decision_queue.pop_back().unwrap();
                self.decision_queue_why.pop_back();

                // Decision becomes undecided
                self.decision_map[decision.decided().index()] = 0;
            }

            let mut visited_assertions = 0;
            for &rule_id in &self.rule_assertions {
                visited_assertions += 1;

                let rule = &mut self.rules[rule_id.index()];

                if have_disabled && rule_id >= self.learnt_rules_start {
                    // Just started with learnt rule assertions. If we have disabled some rules,
                    // adapt the learnt rule status
                    Solver::enable_disable_learnt_rules();
                    have_disabled = false;
                }

                if !rule.enabled {
                    continue;
                }

                let Some((solvable, asserted_value)) = rule.assertion(&self.pool) else {
                    // The rule is not an assertion
                    continue;
                };

                let decision = self.decision_map[solvable.index()];
                if decision == 0 {
                    // Not yet decided
                    self.decision_queue.push_back(Decision::Undecided);
                    self.decision_queue_why.push_back(rule_id);
                    self.decision_map[solvable.index()] = -1;
                    continue;
                }

                if asserted_value && decision > 0 {
                    // OK to install
                    continue;
                }

                if !asserted_value && decision < 0 {
                    // OK to not install
                    continue;
                }

                /*
                 * found a conflict!
                 *
                 * The rule we're currently processing says something
                 * different than a previous decision
                 * on this literal
                 */

                if rule_id >= self.learnt_rules_start {
                    /* conflict with a learnt rule */
                    /* can happen when packages cannot be installed for multiple reasons. */
                    /* we disable the learnt rule in this case */
                    /* (XXX: we should really do something like analyze_unsolvable_rule here!) */
                    Solver::disable_rule(rule_id);
                    continue;
                }

                // Find the decision which is opposite of the rule
                // let opposite_decision = Decision::Decided(solvable, !asserted_value);
                // let i = self
                //     .decision_queue
                //     .iter()
                //     .position(|&decision| decision == opposite_decision)
                //     .unwrap();
                // let conflicting_rule = if solvable == SolvableId::root() {
                //     RuleId::new(0)
                // } else {
                //     let ori = self.decision_queue_why[i];
                //     assert!(ori.index() > 0);
                //     ori
                // };

                Solver::record_problem(rule_id);

                // Skipped: record proof

                // Disable all problem rules
                // solver_disableproblemset(solv, oldproblemcount);
                have_disabled = true;
                break;
            }

            if visited_assertions == self.rule_assertions.len() {
                break
            } else {
                continue
            }
        }

        Ok(1)
    }

    fn propagate(&mut self, _level: u32) -> Result<(), Rule> {
        for decision in self.decision_queue.iter().skip(self.propagate_index) {
            self.propagate_index += 1;

            // TODO: what about undecided packages in the queue?
            /*
               * 'pkg' was just decided
               * negate because our watches trigger if literal goes FALSE
               */
            let pkg = decision.decided().negate();

            // Foreach rule where 'pkg' is now FALSE
            let mut rule_id = self.watches[self.pool.solvables.len() + pkg.index()];
            loop {
                if rule_id.index() == 0 {
                    break;
                }

                let rule = &self.rules[rule_id.index()];
                if !rule.enabled {
                    // rule is disabled, goto next
                    if pkg.solvable == rule.solvable_id() {
                        rule_id = rule.n1;
                    } else {
                        rule_id = rule.n2;
                    }

                    continue;
                }

                /*
                * 'pkg' was just decided, so this rule may now be unit.
	            */
                /* find the other watch */
                let Some((rule_solvable, rule_candidate)) = rule.solvable_and_candidate(&self.pool) else {
                    panic!("fingers crossed that this is unreachable?")
                };

                let (other_watch, next_rp) = if pkg.solvable == rule_solvable {
                    // Keep the watch on the candidate, advance the watch on the solvable
                    (rule_candidate, rule.n1)
                } else {
                    // Keep the watch on the solvable, advance the watch on the candidate
                    (rule_solvable, rule.n2)
                };

                /*
                   * if the other watch is true we have nothing to do
                   */
                let value_from_map = self.decision_map[other_watch.index()] > 0;
                if value_from_map {
                    continue;
                }

                /*
                   * The other literal is FALSE or UNDEF
                   *
                   */

                // TODO: continue
            }
        }

        Ok(())
    }

    fn analyze_unsolvable(&mut self, _rule: Rule, _disable_rules: bool) -> u32 {
        todo!()
    }

    fn enable_disable_learnt_rules() {
        // See enabledisablelearntrules
    }

    fn disable_rule(_rule_id: RuleId) {}

    fn record_problem(_rule_id: RuleId) {
        // See solver_recordproblem
        // TODO: this is only for error reporting, I assume?
    }

    fn make_watches(&mut self) {
        // Lower half for removals, upper half for installs
        self.watches = vec![RuleId::new(0); self.pool.solvables.len() * 2];

        for (i, rule) in self.rules.iter_mut().enumerate().skip(1).rev() {
            // Watches are only created for rules with candidates, not for assertions
            if let Some((solvable_id, first_candidate_id)) =
                rule.solvable_and_candidate(&self.pool)
            {
                let solvable_watch_index = self.pool.solvables.len() + solvable_id.index();
                rule.n1 = RuleId::new(0);
                self.watches[solvable_watch_index] = RuleId::new(i);

                let candidate_watch_index = self.pool.solvables.len() + first_candidate_id.index();
                rule.n2 = RuleId::new(0);
                self.watches[candidate_watch_index] = RuleId::new(i);
            }
        }
    }
}
