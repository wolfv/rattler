use std::collections::HashSet;
use std::ffi::CStr;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::{collections::HashMap, str::FromStr};

use crate::libsolv::wrapper::pool::Pool;
use crate::libsolv::wrapper::solve_goal::SolveGoal;
use anyhow::anyhow;
use graphlib::Graph;
use petgraph::graph::DiGraph;
use petgraph::stable_graph::DefaultIx;
use rattler_conda_types::MatchSpec;

use super::ffi;
use super::flags::SolverFlag;
use super::queue::Queue;
use super::solvable::SolvableId;
use super::transaction::Transaction;

use ffi::{
    SolverRuleinfo_SOLVER_RULE_JOB as SOLVER_RULE_JOB,
    SolverRuleinfo_SOLVER_RULE_JOB_NOTHING_PROVIDES_DEP as SOLVER_RULE_JOB_NOTHING_PROVIDES_DEP,
    SolverRuleinfo_SOLVER_RULE_JOB_UNKNOWN_PACKAGE as SOLVER_RULE_JOB_UNKNOWN_PACKAGE,
    SolverRuleinfo_SOLVER_RULE_PKG as SOLVER_RULE_PKG,
    SolverRuleinfo_SOLVER_RULE_PKG_CONFLICTS as SOLVER_RULE_SOLVER_RULE_PKG_CONFLICTS,
    SolverRuleinfo_SOLVER_RULE_PKG_CONSTRAINS as SOLVER_RULE_PKG_CONSTRAINS,
    SolverRuleinfo_SOLVER_RULE_PKG_NOTHING_PROVIDES_DEP as SOLVER_RULE_SOLVER_RULE_PKG_NOTHING_PROVIDES_DEP,
    SolverRuleinfo_SOLVER_RULE_PKG_REQUIRES as SOLVER_RULE_PKG_REQUIRES,
    SolverRuleinfo_SOLVER_RULE_PKG_SAME_NAME as SOLVER_RULE_SOLVER_RULE_PKG_SAME_NAME,
    SolverRuleinfo_SOLVER_RULE_UPDATE as SOLVER_RULE_SOLVER_RULE_UPDATE,
};

/// Wrapper for libsolv solver, which is used to drive dependency resolution
///
/// The wrapper functions as an owned pointer, guaranteed to be non-null and freed
/// when the Solver is dropped
pub struct Solver<'pool>(NonNull<ffi::Solver>, PhantomData<&'pool Pool>);

impl<'pool> Drop for Solver<'pool> {
    fn drop(&mut self) {
        unsafe { ffi::solver_free(self.0.as_mut()) }
    }
}

#[derive(Debug)]
pub struct SolverProblem {
    pub problem_type: ffi::SolverRuleinfo,
    pub source_id: ffi::Id,
    pub target_id: ffi::Id,
    pub dep_id: ffi::Id,
    pub source: Option<SolvableId>,
    pub target: Option<SolvableId>,
    pub dep: Option<String>,
}
impl SolverProblem {
    pub(crate) fn from_raw(
        solver: &Solver,
        problem_type: ffi::SolverRuleinfo,
        source_id: i32,
        target_id: i32,
        dep_id: i32,
    ) -> Self {
        let pool = unsafe { (*solver.0.as_ptr()).pool as *mut ffi::Pool };

        let nsolvables = unsafe { (*pool).nsolvables };

        let target = if target_id < 0 || target_id >= nsolvables as i32 {
            None
        } else {
            Some(SolvableId(target_id))
        };

        let source = if source_id < 0 || source_id >= nsolvables as i32 {
            None
        } else {
            Some(SolvableId(target_id))
        };

        let dep = if dep_id < 0 || dep_id >= nsolvables as i32 {
            None
        } else {
            let dep = unsafe { ffi::pool_dep2str(pool, dep_id.into()) };
            let dep = unsafe { CStr::from_ptr(dep) };
            let dep = dep.to_str().unwrap().to_string();
            Some(dep)
        };

        return SolverProblem {
            problem_type: problem_type.try_into().unwrap(),
            source_id: source_id.into(),
            target_id: target_id.into(),
            dep_id: dep_id.into(),
            source,
            target,
            dep,
        };
    }
}

impl Solver<'_> {
    /// Constructs a new Solver from the provided libsolv pointer. It is the responsibility of the
    /// caller to ensure the pointer is actually valid.
    pub(super) unsafe fn new(_pool: &Pool, ptr: NonNull<ffi::Solver>) -> Solver {
        Solver(ptr, PhantomData::default())
    }

    /// Returns a raw pointer to the wrapped `ffi::Solver`, to be used for calling ffi functions
    /// that require access to the pool (and for nothing else)
    fn raw_ptr(&self) -> *mut ffi::Solver {
        self.0.as_ptr()
    }

    /// Returns the amount of problems that are yet to be solved
    fn problem_count(&self) -> u32 {
        unsafe { ffi::solver_problem_count(self.raw_ptr()) }
    }

    /// Returns a user-friendly representation of a problem
    ///
    /// Safety: the caller must ensure the id is valid
    unsafe fn problem2str(&self, id: ffi::Id) -> &CStr {
        let problem = ffi::solver_problem2str(self.raw_ptr(), id);
        CStr::from_ptr(problem)
    }

    /// Creates a string of 'problems' that the solver still has which it encountered while solving
    /// the matchspecs. Use this function to print the existing problems to string.
    fn solver_problems(&self) -> String {
        let mut output = String::default();

        let count = self.problem_count();
        for i in 1..=count {
            // Safe because the id valid (between [1, count])
            let problem = unsafe { self.problem2str(i as ffi::Id) };

            output.push_str(" - ");
            output.push_str(problem.to_str().expect("string is invalid UTF8"));
            output.push('\n');
        }
        output
    }

    pub fn all_solver_problems(&self) -> Vec<SolverProblem> {
        let mut problems = Vec::new();
        let mut problem_rules = Queue::<ffi::Id>::default();

        let count = self.problem_count();
        for i in 1..=count {
            unsafe {
                ffi::solver_findallproblemrules(
                    self.raw_ptr(),
                    i.try_into().unwrap(),
                    problem_rules.raw_ptr(),
                )
            };
            for r in problem_rules.id_iter() {
                if r != 0 {
                    let mut source = 0;
                    let mut target = 0;
                    let mut dep = 0;
                    let problem_type = unsafe {
                        ffi::solver_ruleinfo(self.raw_ptr(), r, &mut source, &mut target, &mut dep)
                    };

                    problems.push(SolverProblem::from_raw(
                        self,
                        problem_type,
                        source,
                        target,
                        dep,
                    ));
                }
            }
        }
        return problems;
    }

    /// Sets a solver flag
    pub fn set_flag(&self, flag: SolverFlag, value: bool) {
        unsafe { ffi::solver_set_flag(self.raw_ptr(), flag.inner(), i32::from(value)) };
    }

    /// Solves all the problems in the `queue` and returns a transaction from the found solution.
    /// Returns an error if problems remain unsolved.
    pub fn solve(&mut self, queue: &mut SolveGoal) -> anyhow::Result<Transaction> {
        let result = unsafe {
            // Run the solve method
            ffi::solver_solve(self.raw_ptr(), queue.raw_ptr());
            // If there are no problems left then the solver is done
            ffi::solver_problem_count(self.raw_ptr()) == 0
        };
        if result {
            let transaction =
                NonNull::new(unsafe { ffi::solver_create_transaction(self.raw_ptr()) })
                    .expect("solver_create_transaction returned a nullptr");

            // Safe because we know the `transaction` ptr is valid
            Ok(unsafe { Transaction::new(self, transaction) })
        } else {
            let problems = self.all_solver_problems();

            problems_to_graph(self, problems);

            Err(anyhow!(
                "encountered problems while solving:\n {}",
                self.solver_problems()
            ))
        }
    }
}

pub struct PackageNode {
    pub id: SolvableId,
    pub problem_type: Option<ffi::SolverRuleinfo>,
}



fn problems_to_graph(
    arg: &mut Solver,
    problems: Vec<SolverProblem>,
) -> DiGraph<SolvableId, MatchSpec> {
    let mut graph = DiGraph::<PackageNode, Option<MatchSpec>>::new();
    let conflicts = HashMap::<_, HashSet<_>>::new();

    let mut add_conflict = |src: _, tgt: _| {
        let entry = conflicts.entry(src).or_insert_with(HashSet::new);
        entry.insert(tgt);
        let entry = conflicts.entry(tgt).or_insert_with(HashSet::new);
        entry.insert(src);
    };

    for problem in problems {
        let source = problem.source;
        let target = problem.target;
        let dep = problem
            .dep
            .filter(|d| d != "<NONE>")
            .map(|d| MatchSpec::from_str(&d).unwrap());

        match problem.problem_type {
            SOLVER_RULE_PKG_CONSTRAINS => {
                if source.is_some() && target.is_some() {
                    let v1 = graph.add_node(PackageNode {
                        id: source.unwrap(),
                        problem_type: None,
                    });
                    let v2 = graph.add_node(PackageNode {
                        id: source.unwrap(),
                        problem_type: Some(problem.problem_type),
                    });
                    add_conflict(v1, v2);
                    graph.add_edge(v1, v2, dep);
                }
            }
            SOLVER_RULE_PKG_REQUIRES => {
                assert!(dep.is_some() && source.is_some());
                let v1 = graph.add_node(PackageNode {
                    id: source.unwrap(),
                    problem_type: None,
                });
                let dep = dep.unwrap();
                let added = add_expanded_deps_edges(solver, v1, problem.dep_id, dep);
            }
            SOLVER_RULE_JOB | SOLVER_RULE_PKG => {
                // A top level requirement.
                // The difference between JOB and PKG is unknown (possibly unused).
                let dep = dep.unwrap();
                // bool added = add_expanded_deps_edges(m_root_node, problem.dep_id, edge);
            }
            SOLVER_RULE_JOB_NOTHING_PROVIDES_DEP | SOLVER_RULE_JOB_UNKNOWN_PACKAGE => {
                // A top level dependency does not exist.
                // Could be a wrong name or missing channel.
                let dep = dep.unwrap();
                // node_id dep_id = add_solvable(
                //     problem.dep_id,
                //     UnresolvedDependencyNode{ std::move(dep).value(), type }
                // );
                // m_graph.add_edge(m_root_node, dep_id, std::move(edge));
            }
            SOLVER_RULE_PKG_NOTHING_PROVIDES_DEP => {
                // A package dependency does not exist. Could be a wrong name or missing channel.
                // This is a partial exaplanation of why a specific solvable (could be any
                // of the parent) cannot be installed.
                let edge = dep.unwrap();
                // node_id src_id = add_solvable(
                //     problem.source_id,
                //     PackageNode{ std::move(source).value(), std::nullopt }
                // );
                // node_id dep_id = add_solvable(
                //     problem.dep_id,
                //     UnresolvedDependencyNode{ std::move(dep).value(), type }
                // );
                // m_graph.add_edge(src_id, dep_id, std::move(edge));
            }
            SOLVER_RULE_PKG_CONFLICTS | SOLVER_RULE_PKG_SAME_NAME => {
                // Looking for a valid solution to the installation satisfiability expand to
                // two solvables of same package that cannot be installed together. This is
                // a partial exaplanation of why one of the solvables (could be any of the
                // parent) cannot be installed.
                // if (!source || !target)
                // {
                //     warn_unexpected_problem(problem);
                //     break;
                // }
                // node_id src_id = add_solvable(
                //     problem.source_id,
                //     PackageNode{ std::move(source).value(), { type } }
                // );
                // node_id tgt_id = add_solvable(
                //     problem.target_id,
                //     PackageNode{ std::move(target).value(), { type } }
                // );
                // add_conflict(src_id, tgt_id);
                // break;
            }
            SOLVER_RULE_UPDATE => {
                // Encounterd in the problems list from libsolv but unknown.
                // Explicitly ignored until we do something with it.
            }
            _ => {
                panic!("Unknown problem type: {:?}", problem.problem_type);
            }
        }

        // if source.is_some() && target.is_some() {
        //     let v1 = graph.add_node(source.unwrap());
        //     let v2 = graph.add_node(target.unwrap());
        //     let s = dep.unwrap_or("NONE".to_string());
        //     graph.add_edge(v1, v2, s);
        // }
    }

    // println!("digraph {:?}", graph);
    // let mut dot = String::new();
    // dot::render(&graph, &mut dot).unwrap();
    return graph;
}

fn add_expanded_deps_edges(solver: &Solver, v1: petgraph::stable_graph::NodeIndex, dep_id: i32, dep: MatchSpec) -> _ {
    let pool = unsafe { (*solver.0.as_ptr()).pool as *mut ffi::Pool };
    pool.select_by_id(dep_id);
}
