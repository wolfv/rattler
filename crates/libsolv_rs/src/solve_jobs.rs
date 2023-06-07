use crate::solvable::SolvableId;
use rattler_conda_types::MatchSpec;
use std::collections::VecDeque;

pub enum CandidateSource {
    MatchSpec(MatchSpec),
    // Package(PackageRecord),
}

#[derive(Default)]
pub struct SolveJobs {
    pub items: VecDeque<(CandidateSource, SolveOperation)>,
}

#[derive(Copy, Clone)]
pub enum SolveOperation {
    Install,  // SOLVER_INSTALL | SOLVER_SOLVABLE_PROVIDES
    Erase,    // SOLVER_ERASE | SOLVER_SOLVABLE_PROVIDES
    Update,   // SOLVER_UPDATE | SOLVER_SOLVABLE_PROVIDES
    Favor,    // SOLVER_SOLVABLE | SOLVER_FAVOR
    Lock,     // SOLVER_SOLVABLE | SOLVER_LOCK
    Disfavor, // SOLVER_SOLVABLE | SOLVER_DISFAVOR
}

impl SolveJobs {
    /// The specified spec must be installed
    pub fn install(&mut self, match_spec: MatchSpec) {
        self.items.push_back((
            CandidateSource::MatchSpec(match_spec),
            SolveOperation::Install,
        ));
    }

    /// The specified spec must not be installed.
    pub fn erase(&mut self, match_spec: MatchSpec) {
        self.items.push_back((
            CandidateSource::MatchSpec(match_spec),
            SolveOperation::Erase,
        ));
    }

    /// The highest possible spec must be installed
    pub fn update(&mut self, match_spec: MatchSpec) {
        self.items.push_back((
            CandidateSource::MatchSpec(match_spec),
            SolveOperation::Update,
        ));
    }

    /// Favor the specified solvable over other variants. This doesnt mean this variant will be
    /// used. To guarantee a solvable is used (if selected) use the `Self::lock` function.
    pub fn favor(&mut self, id: SolvableId) {
        todo!()
        // self.items.push_back((Id::solvable(id), SolveOperation::Update));
    }

    /// Lock the specified solvable over other variants. This implies that not other variant will
    /// ever be considered.
    pub fn lock(&mut self, id: SolvableId) {
        todo!()
        // self.items.push_back((Id::solvable(id), SolveOperation::Lock));
    }

    /// Disfavor the specified variant over other variants. This does not mean it will never be
    /// selected, but other variants are considered first.
    pub fn disfavor(&mut self, id: SolvableId) {
        todo!()
        // self.items.push_back((Id::solvable(id), SolveOperation::Disfavor));
    }
}
