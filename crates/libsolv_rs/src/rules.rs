use crate::pool::MatchSpecId;
use crate::solvable::SolvableId;

pub enum Rule {
    Requires(SolvableId, MatchSpecId),
    Constrains(SolvableId, MatchSpecId),
}
