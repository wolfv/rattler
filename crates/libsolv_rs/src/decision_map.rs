use crate::solvable::SolvableId;
use std::cmp::Ordering;

/// Map of all available solvables
pub(crate) struct DecisionMap {
    /// = 0: undecided
    /// > 0: level of decision when installed
    /// < 0: level of decision when conflict
    map: Vec<i64>,
}

impl DecisionMap {
    pub(crate) fn new(nsolvables: usize) -> Self {
        Self {
            map: vec![0; nsolvables],
        }
    }

    pub(crate) fn reset(&mut self, solvable_id: SolvableId) {
        self.map[solvable_id.index()] = 0;
    }

    pub(crate) fn set(&mut self, solvable_id: SolvableId, value: bool, level: u32) {
        self.map[solvable_id.index()] = if value { level as i64 } else { -(level as i64) };
    }

    pub(crate) fn level(&mut self, solvable_id: SolvableId) -> u32 {
        self.map[solvable_id.index()].abs() as u32
    }

    pub(crate) fn value(&self, solvable_id: SolvableId) -> Option<bool> {
        match self.map[solvable_id.index()].cmp(&0) {
            Ordering::Less => Some(false),
            Ordering::Equal => None,
            Ordering::Greater => Some(true),
        }
    }
}
