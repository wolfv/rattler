use crate::pool::{MatchSpecId, RepoId, StringId};
use rattler_conda_types::PackageRecord;

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct SolvableId(u32);

impl SolvableId {
    pub fn new(index: usize) -> Self {
        Self(index as u32)
    }

    pub fn root() -> Self {
        Self(0)
    }

    pub fn null() -> Self {
        Self(u32::MAX)
    }

    pub fn is_null(self) -> bool {
        self.0 == u32::MAX
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

pub enum Solvable {
    Root(Vec<MatchSpecId>),
    Package(PackageSolvable),
}

impl Solvable {
    pub(crate) fn debug(&self) {
        match self {
            Solvable::Root(_) => print!("root"),
            Solvable::Package(p) => print!("{} {}", p.record.name, p.record.version),
        }
    }
}

pub struct PackageSolvable {
    pub(crate) repo_id: RepoId,
    pub(crate) dependencies: Vec<MatchSpecId>,
    pub(crate) constrains: Vec<MatchSpecId>,
    pub(crate) record: &'static PackageRecord,
    pub(crate) name: StringId,
    pub version: StringId,
    pub metadata: SolvableMetadata,
}

#[derive(Default)]
pub struct SolvableMetadata {
    pub original_index: Option<usize>,
}

impl Solvable {
    pub fn new(
        repo_id: RepoId,
        name: StringId,
        version: StringId,
        record: &'static PackageRecord,
    ) -> Self {
        Solvable::Package(PackageSolvable {
            repo_id,
            record,
            name,
            version,
            dependencies: Vec::new(),
            constrains: Vec::new(),
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
