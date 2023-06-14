use crate::solvable::{Solvable, SolvableId};
use rattler_conda_types::{MatchSpec, PackageRecord};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct RepoId(u32);

impl RepoId {
    fn new(id: u32) -> Self {
        Self(id)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct StringId {
    value: u32,
}

impl StringId {
    pub fn name(index: usize) -> Self {
        Self {
            value: index as u32,
        }
    }

    pub fn index(self) -> usize {
        self.value as usize
    }

    pub fn max() -> Self {
        Self { value: u32::MAX }
    }

    pub fn is_max(self) -> bool {
        self.value == u32::MAX
    }
}

#[derive(Clone, Copy)]
pub struct MatchSpecId(u32);

impl MatchSpecId {
    fn new(index: usize) -> Self {
        Self(index as u32)
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

pub struct Pool {
    pub(crate) solvables: Vec<Solvable>,

    /// The total amount of registered repos
    total_repos: u32,

    /// The id of the repo that contains packages considered to be installed
    pub(crate) installed: Option<RepoId>,

    /// Interned strings
    pub(crate) strings_to_ids: HashMap<String, StringId>,
    strings: HashMap<StringId, String>,

    /// Interned match specs
    match_specs_to_ids: HashMap<String, MatchSpecId>,
    pub(crate) match_specs: Vec<MatchSpec>,

    /// Cached candidates for each match spec, indexed by their MatchSpecId
    pub(crate) match_spec_to_candidates: Vec<Option<Vec<SolvableId>>>,

    pub(crate) match_spec_to_forbidden: Vec<Option<Vec<SolvableId>>>,

    // TODO: eventually we could turn this into a Vec, making sure we have a separate interning
    // scheme for package names
    pub(crate) packages_by_name: HashMap<StringId, Vec<SolvableId>>,
}

impl Pool {
    pub fn new() -> Self {
        Pool {
            solvables: vec![Solvable::Root(Vec::new())],
            total_repos: 0,

            installed: None,

            strings_to_ids: HashMap::new(),
            strings: HashMap::new(),

            packages_by_name: HashMap::default(),

            match_specs_to_ids: HashMap::default(),
            match_specs: Vec::new(),
            match_spec_to_candidates: Vec::new(),
            match_spec_to_forbidden: Vec::new(),
        }
    }

    pub fn new_repo(&mut self, _url: impl AsRef<str>) -> RepoId {
        let id = RepoId::new(self.total_repos);
        self.total_repos += 1;
        id
    }

    /// Adds a new solvable to a repo
    pub fn add_package(&mut self, repo_id: RepoId, record: &'static PackageRecord) -> SolvableId {
        let name = self.intern_str(&record.name);
        let version = self.intern_str(record.version.to_string());

        let solvable_id = SolvableId::new(self.solvables.len());
        self.solvables
            .push(Solvable::new(repo_id, name, version, record));

        assert!(repo_id.0 < self.total_repos);

        self.packages_by_name
            .entry(name)
            .or_insert(Vec::new())
            .push(solvable_id);

        solvable_id
    }

    pub fn intern_matchspec(&mut self, match_spec: String) -> MatchSpecId {
        let next_index = self.match_specs.len();
        match self.match_specs_to_ids.entry(match_spec) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                self.match_specs
                    .push(MatchSpec::from_str(entry.key()).unwrap());
                self.match_spec_to_candidates.push(None);

                // Update the entry
                let id = MatchSpecId::new(next_index);
                entry.insert(id);

                id
            }
        }
    }

    pub fn reset_package(
        &mut self,
        repo_id: RepoId,
        solvable_id: SolvableId,
        record: &'static PackageRecord,
    ) {
        let name = self.intern_str(&record.name);
        let version = self.intern_str(&record.version.to_string());
        self.solvables[solvable_id.index()] = Solvable::new(repo_id, name, version, record);
    }

    // This function does not take `self`, because otherwise we run into problems with borrowing
    // when we want to use it together with other pool functions
    pub fn get_candidates<'a>(
        match_specs: &'a [MatchSpec],
        strings_to_ids: &'a HashMap<String, StringId>,
        solvables: &'a [Solvable],
        packages_by_name: &'a HashMap<StringId, Vec<SolvableId>>,
        match_spec_to_candidates: &'a mut [Option<Vec<SolvableId>>],
        match_spec_id: MatchSpecId,
    ) -> &'a [SolvableId] {
        let candidates = match_spec_to_candidates[match_spec_id.index()].get_or_insert_with(|| {
            let match_spec = &match_specs[match_spec_id.index()];
            let match_spec_name = match_spec
                .name
                .as_deref()
                .expect("match spec without name!");
            let name_id = match strings_to_ids.get(match_spec_name) {
                None => return Vec::new(),
                Some(name_id) => name_id,
            };

            packages_by_name[&name_id]
                .iter()
                .cloned()
                .filter(|solvable| match_spec.matches(solvables[solvable.index()].package().record))
                .collect()
        });

        candidates.as_slice()
    }

    // This function does not take `self`, because otherwise we run into problems with borrowing
    // when we want to use it together with other pool functions
    pub fn get_forbidden<'a>(
        match_specs: &'a [MatchSpec],
        strings_to_ids: &'a HashMap<String, StringId>,
        solvables: &'a [Solvable],
        packages_by_name: &'a HashMap<StringId, Vec<SolvableId>>,
        match_spec_to_forbidden: &'a mut [Option<Vec<SolvableId>>],
        match_spec_id: MatchSpecId,
    ) -> &'a [SolvableId] {
        let candidates = match_spec_to_forbidden[match_spec_id.index()].get_or_insert_with(|| {
            let match_spec = &match_specs[match_spec_id.index()];
            let match_spec_name = match_spec
                .name
                .as_deref()
                .expect("match spec without name!");
            let name_id = match strings_to_ids.get(match_spec_name) {
                None => return Vec::new(),
                Some(name_id) => name_id,
            };

            packages_by_name[&name_id]
                .iter()
                .cloned()
                .filter(|solvable| {
                    !match_spec.matches(solvables[solvable.index()].package().record)
                })
                .collect()
        });

        candidates.as_slice()
    }

    pub fn add_dependency(&mut self, solvable_id: SolvableId, match_spec: String) {
        let match_spec_id = self.intern_matchspec(match_spec);
        let solvable = self.solvables[solvable_id.index()].package_mut();
        solvable.dependencies.push(match_spec_id);
    }

    pub fn add_constrains(&mut self, solvable_id: SolvableId, match_spec: String) {
        let match_spec_id = self.intern_matchspec(match_spec);
        let solvable = self.solvables[solvable_id.index()].package_mut();
        solvable.constrains.push(match_spec_id);
    }

    pub fn nsolvables(&self) -> usize {
        self.solvables.len()
    }

    /// Set the provided repo to be considered as a source of installed packages
    ///
    /// Panics if the repo does not belong to this pool
    pub fn set_installed(&mut self, repo_id: RepoId) {
        self.installed = Some(repo_id);
    }

    /// Interns string like types into a `Pool` returning an `Id`
    pub fn intern_str<T: Into<String>>(&mut self, str: T) -> StringId {
        let next_id = StringId::name(self.strings_to_ids.len());
        match self.strings_to_ids.entry(str.into()) {
            Entry::Occupied(e) => e.get().clone(),
            Entry::Vacant(e) => {
                self.strings.insert(next_id, e.key().clone());
                e.insert(next_id);
                next_id
            }
        }
    }

    /// Finds a previously interned string or returns `None` if it wasn't found
    pub fn find_interned_str<T: AsRef<str>>(&self, str: T) -> Option<StringId> {
        self.strings_to_ids.get(str.as_ref()).cloned()
    }

    pub fn resolve_string(&self, string_id: StringId) -> &str {
        &self.strings[&string_id]
    }

    /// Returns a string describing the last error associated to this pool, or "no error" if there
    /// were no errors
    pub fn last_error(&self) -> String {
        // See pool_errstr
        "no error".to_string()
    }

    /// Resolves the id to a solvable
    ///
    /// Panics if the solvable is not found in the pool
    pub fn resolve_solvable(&self, id: SolvableId) -> &Solvable {
        if id.index() < self.solvables.len() {
            &self.solvables[id.index()]
        } else {
            panic!("invalid solvable id!")
        }
    }

    pub fn resolve_match_spec(&self, id: MatchSpecId) -> &MatchSpec {
        &self.match_specs[id.index()]
    }

    pub fn root_solvable_mut(&mut self) -> &mut Vec<MatchSpecId> {
        match &mut self.solvables[0] {
            Solvable::Root(ids) => ids,
            Solvable::Package(_) => panic!("root solvable not registered!"),
        }
    }

    /// Resolves the id to a solvable
    ///
    /// Panics if the solvable is not found in the pool
    pub fn resolve_solvable_mut(&mut self, id: SolvableId) -> &mut Solvable {
        if id.index() < self.solvables.len() {
            &mut self.solvables[id.index()]
        } else {
            panic!("invalid solvable id!")
        }
    }
}
