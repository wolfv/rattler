use crate::pool::{MatchSpecId, RepoId, StringId};
use rattler_conda_types::PackageRecord;

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct SolvableId(u32);

impl SolvableId {
    pub fn new(index: usize) -> Self {
        Self(index as u32)
    }

    pub fn root() -> Self {
        Self(0)
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

pub enum Solvable {
    Root(Vec<MatchSpecId>),
    Package(PackageSolvable),
}

pub struct PackageSolvable {
    pub(crate) repo_id: RepoId,
    // TODO: it might be unnecessary to keep dependencies and constrains stored, since they are only
    // needed when generating rules
    pub(crate) dependencies: Vec<MatchSpecId>,
    pub(crate) constrains: Vec<MatchSpecId>,
    pub(crate) record: &'static PackageRecord,
    pub(crate) name: StringId,
    pub evr: StringId,
    pub metadata: SolvableMetadata,
}

#[derive(Default)]
pub struct SolvableMetadata {
    pub original_index: Option<usize>,
}

impl Solvable {
    pub fn new(repo_id: RepoId, name: StringId, record: &'static PackageRecord) -> Self {
        Solvable::Package(PackageSolvable {
            repo_id,
            record,
            name,
            dependencies: Vec::new(),
            constrains: Vec::new(),
            evr: StringId::max(),
            metadata: SolvableMetadata::default(),
        })
    }

    pub fn repo_id(&self) -> RepoId {
        match self {
            Solvable::Package(p) => p.repo_id,
            Solvable::Root(_) => panic!("root solvable has no repo"),
        }
    }

    pub fn package(&self) -> &PackageSolvable {
        match self {
            Solvable::Root(_) => panic!("unexpected root solvable"),
            Solvable::Package(p) => p,
        }
    }

    pub fn package_mut(&mut self) -> &mut PackageSolvable {
        match self {
            Solvable::Root(_) => panic!("unexpected root solvable"),
            Solvable::Package(p) => p,
        }
    }
}
