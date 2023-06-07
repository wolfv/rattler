use crate::pool::{MatchSpecId, Pool};
use crate::solvable::SolvableId;
use crate::solver::RuleId;

pub(crate) struct Rule {
    pub enabled: bool,
    pub n1: RuleId,
    pub n2: RuleId,
    kind: RuleKind,
}

impl Rule {
    pub fn new(kind: RuleKind) -> Self {
        Self {
            enabled: true,
            n1: RuleId::new(0),
            n2: RuleId::new(0),
            kind,
        }
    }

    pub fn solvable_id(&self) -> SolvableId {
        match &self.kind {
            RuleKind::InstallRoot => SolvableId::root(),
            RuleKind::Requires(solvable_id, _) |
            RuleKind::Constrains(solvable_id, _) => solvable_id.clone()
        }
    }

    pub fn is_assertion(&self, pool: &Pool) -> bool {
        self.solvable_and_candidate(pool).is_none()
    }

    pub fn assertion(&self, pool: &Pool) -> Option<(SolvableId, bool)> {
        match &self.kind {
            RuleKind::InstallRoot => Some((SolvableId::root(), true)),
            RuleKind::Requires(id, match_spec) | RuleKind::Constrains(id, match_spec) => {
                let value = !pool.match_spec_to_candidates[match_spec.index()]
                    .as_ref()
                    .unwrap()
                    .is_empty();
                Some((id.clone(), value))
            }
        }
    }

    pub fn solvable_and_candidate(&self, pool: &Pool) -> Option<(SolvableId, SolvableId)> {
        match &self.kind {
            RuleKind::InstallRoot => None,
            RuleKind::Requires(id, match_spec) | RuleKind::Constrains(id, match_spec) => {
                let &first_candidate = pool.match_spec_to_candidates[match_spec.index()]
                    .as_ref()
                    .unwrap()
                    .iter()
                    .next()?;
                Some((id.clone(), first_candidate))
            }
        }
    }
}

#[derive(Copy, Clone)]
pub enum RuleKind {
    InstallRoot,
    Requires(SolvableId, MatchSpecId),
    Constrains(SolvableId, MatchSpecId),
}
