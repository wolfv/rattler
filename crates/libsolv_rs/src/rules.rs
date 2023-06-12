use crate::pool::{MatchSpecId, Pool};
use crate::solvable::SolvableId;
use crate::solver::{DecisionMap, RuleId};

#[derive(Clone)]
pub(crate) struct Rule {
    pub enabled: bool,
    pub w1: SolvableId,
    pub w2: SolvableId,
    pub n1: RuleId,
    pub n2: RuleId,
    pub kind: RuleKind,
}

impl Rule {
    pub fn new(kind: RuleKind, pool: &Pool) -> Self {
        let (w1, w2) = kind
            .initial_watches(pool)
            .unwrap_or((SolvableId::null(), SolvableId::null()));
        Self {
            enabled: true,
            w1,
            w2,
            n1: RuleId::null(),
            n2: RuleId::null(),
            kind,
        }
    }

    pub fn has_watches(&self) -> bool {
        // If w1 is not null, w2 won't be either
        !self.w1.is_null()
    }

    pub fn watched_literals(&self) -> (Literal, Literal) {
        match self.kind {
            RuleKind::InstallRoot => unreachable!(),
            RuleKind::Requires(solvable_id, _) => {
                if self.w1 == solvable_id {
                    (
                        Literal {
                            solvable_id: self.w1,
                            negate: true,
                        },
                        Literal {
                            solvable_id: self.w2,
                            negate: false,
                        },
                    )
                } else if self.w2 == solvable_id {
                    (
                        Literal {
                            solvable_id: self.w1,
                            negate: false,
                        },
                        Literal {
                            solvable_id: self.w2,
                            negate: true,
                        },
                    )
                } else {
                    (
                        Literal {
                            solvable_id: self.w1,
                            negate: false,
                        },
                        Literal {
                            solvable_id: self.w2,
                            negate: false,
                        },
                    )
                }
            }
            RuleKind::Constrains(_, _) => (
                Literal {
                    solvable_id: self.w1,
                    negate: true,
                },
                Literal {
                    solvable_id: self.w2,
                    negate: true,
                },
            ),
        }
    }

    pub fn next_unwatched_variable(
        &self,
        pool: &Pool,
        decision_map: &DecisionMap,
    ) -> Option<SolvableId> {
        // The next unwatched variable (if available), is a variable that is:
        // * Not already being watched
        // * Not yet decided, or decided in such a way that the literal yields true
        let can_watch = |solvable_lit: Literal| {
            self.w1 != solvable_lit.solvable_id
                && self.w2 != solvable_lit.solvable_id
                && solvable_lit.eval(decision_map).unwrap_or(true)
        };

        match self.kind {
            RuleKind::InstallRoot => unreachable!(),
            RuleKind::Requires(solvable_id, match_spec_id) => {
                // The solvable that added this rule
                let solvable_lit = Literal {
                    solvable_id: solvable_id.clone(),
                    negate: true,
                };
                if can_watch(solvable_lit) {
                    return Some(solvable_id);
                }

                // The available candidates
                for &candidate in pool.match_spec_to_candidates[match_spec_id.index()]
                    .as_deref()
                    .unwrap()
                {
                    let lit = Literal {
                        solvable_id: candidate,
                        negate: false,
                    };
                    if can_watch(lit) {
                        return Some(candidate);
                    }
                }

                // No solvable available to watch
                None
            }
            RuleKind::Constrains(solvable_id, match_spec_id) => {
                // The solvable that added this rule
                let solvable_lit = Literal {
                    solvable_id: solvable_id.clone(),
                    negate: true,
                };
                if can_watch(solvable_lit) {
                    return Some(solvable_id);
                }

                // The forbidden candidates
                for &forbidden in pool.match_spec_to_forbidden[match_spec_id.index()]
                    .as_deref()
                    .unwrap()
                {
                    let lit = Literal {
                        solvable_id: forbidden,
                        negate: true,
                    };
                    if can_watch(lit) {
                        return Some(forbidden);
                    }
                }

                // No solvable available to watch
                None
            }
        }
    }
}

#[derive(Copy, Clone)]
pub struct Literal {
    pub solvable_id: SolvableId,
    pub negate: bool,
}

impl Literal {
    pub(crate) fn satisfying_value(self) -> bool {
        !self.negate
    }

    pub(crate) fn eval(self, decision_map: &DecisionMap) -> Option<bool> {
        decision_map
            .value(self.solvable_id)
            .map(|value| self.eval_inner(value))
    }

    fn eval_inner(self, solvable_value: bool) -> bool {
        if self.negate {
            !solvable_value
        } else {
            solvable_value
        }
    }
}

#[derive(Copy, Clone)]
pub enum RuleKind {
    InstallRoot,
    Requires(SolvableId, MatchSpecId),
    Constrains(SolvableId, MatchSpecId),
}

impl RuleKind {
    fn initial_watches(&self, pool: &Pool) -> Option<(SolvableId, SolvableId)> {
        match self {
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

#[test]
fn test_literal_satisfying_value() {
    let lit = Literal {
        solvable_id: SolvableId::root(),
        negate: true,
    };
    assert_eq!(lit.satisfying_value(), false);

    let lit = Literal {
        solvable_id: SolvableId::root(),
        negate: false,
    };
    assert_eq!(lit.satisfying_value(), true);
}

#[test]
fn test_literal_eval() {
    let mut decision_map = DecisionMap::new(10);

    let literal = Literal {
        solvable_id: SolvableId::root(),
        negate: false,
    };
    let negated_literal = Literal {
        solvable_id: SolvableId::root(),
        negate: true,
    };

    // Undecided
    assert_eq!(literal.eval(&decision_map), None);
    assert_eq!(negated_literal.eval(&decision_map), None);

    // Decided
    decision_map.set(SolvableId::root(), true, 1);
    assert_eq!(literal.eval(&decision_map), Some(true));
    assert_eq!(negated_literal.eval(&decision_map), Some(false));

    decision_map.set(SolvableId::root(), false, 1);
    assert_eq!(literal.eval(&decision_map), Some(false));
    assert_eq!(negated_literal.eval(&decision_map), Some(true));
}
