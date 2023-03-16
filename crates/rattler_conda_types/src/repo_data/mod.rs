//! Defines [`RepoData`]. `RepoData` stores information of all packages present in a subdirectory
//! of a channel. It provides indexing functionality.

pub mod patches;

use std::fmt::{Display, Formatter};
use std::path::Path;

use fxhash::{FxHashMap, FxHashSet};

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none, DisplayFromStr, OneOrMany};

use crate::package::IndexJson;
use crate::{Channel, NoArchType, RepoDataRecord, Version};

fn sort_map_alphabetically<T: Serialize, S: serde::Serializer>(
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let value = serde_json::to_value(value).map_err(serde::ser::Error::custom)?;
    value.serialize(serializer)
}

fn sort_set_alphabetically<S: serde::Serializer>(
    value: &FxHashSet<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut value = Vec::from_iter(value.iter().cloned());
    value.sort();
    value.serialize(serializer)
}

/// [`RepoData`] is an index of package binaries available on in a subdirectory of a Conda channel.
#[derive(Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct RepoData {
    /// The version of the repodata format
    #[serde(rename = "repodata_version")]
    pub version: Option<u64>,

    /// The channel information contained in the repodata.json file
    pub info: Option<ChannelInfo>,

    /// The tar.bz2 packages contained in the repodata.json file
    #[serde(serialize_with = "sort_map_alphabetically")]
    pub packages: FxHashMap<String, PackageRecord>,

    /// The conda packages contained in the repodata.json file (under a different key for
    /// backwards compatibility with previous conda versions)
    #[serde(rename = "packages.conda", serialize_with = "sort_map_alphabetically")]
    pub conda_packages: FxHashMap<String, PackageRecord>,

    /// removed packages (files are still accessible, but they are not installable like regular packages)
    #[serde(default, serialize_with = "sort_set_alphabetically")]
    pub removed: FxHashSet<String>,
}

/// Information about subdirectory of channel in the Conda [`RepoData`]
#[derive(Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ChannelInfo {
    /// The channel's subdirectory
    pub subdir: String,
}

/// A single record in the Conda repodata. A single record refers to a single binary distribution
/// of a package on a Conda channel.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd, Clone)]
pub struct PackageRecord {
    /// The name of the package
    pub name: String,

    /// The version of the package
    #[serde_as(as = "DisplayFromStr")]
    pub version: Version,

    /// The build string of the package
    pub build: String,

    /// The build number of the package
    pub build_number: u64,

    /// The subdirectory where the package can be found
    #[serde(default)]
    pub subdir: String,

    /// Optionally a MD5 hash of the package archive
    pub md5: Option<String>,

    /// Optionally a SHA256 hash of the package archive
    pub sha256: Option<String>,

    /// A deprecated md5 hash
    pub legacy_bz2_md5: Option<String>,

    /// A deprecated package archive size.
    pub legacy_bz2_size: Option<u64>,

    /// Optionally the size of the package archive in bytes
    pub size: Option<u64>,

    /// Optionally the architecture the package supports
    pub arch: Option<String>,

    /// Optionally the platform the package supports
    pub platform: Option<String>, // Note that this does not match the [`Platform`] enum..

    /// Specification of packages this package depends on
    #[serde(default)]
    pub depends: Vec<String>,

    /// Additional constraints on packages. `constrains` are different from `depends` in that packages
    /// specified in `depends` must be installed next to this package, whereas packages specified in
    /// `constrains` are not required to be installed, but if they are installed they must follow these
    /// constraints.
    #[serde(default)]
    pub constrains: Vec<String>,

    /// Track features are nowadays only used to downweight packages (ie. give them less priority). To
    /// that effect, the number of track features is counted (number of commas) and the package is downweighted
    /// by the number of track_features.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[serde_as(as = "OneOrMany<_>")]
    pub track_features: Vec<String>,

    /// Features are a deprecated way to specify different feature sets for the conda solver. This is not
    /// supported anymore and should not be used. Instead, `mutex` packages should be used to specify
    /// mutually exclusive features.
    pub features: Option<String>,

    /// If this package is independent of architecture this field specifies in what way. See
    /// [`NoArchType`] for more information.
    #[serde(skip_serializing_if = "NoArchType::is_none")]
    pub noarch: NoArchType,

    /// The specific license of the package
    pub license: Option<String>,

    /// The license family
    pub license_family: Option<String>,

    /// The UNIX Epoch timestamp when this package was created. Note that sometimes this is specified in
    /// seconds and sometimes in milliseconds.
    pub timestamp: Option<u64>,
    // Looking at the `PackageRecord` class in the Conda source code a record can also include all
    // these fields. However, I have no idea if or how they are used so I left them out.
    //pub preferred_env: Option<String>,
    //pub date: Option<String>,
    //pub package_type: ?
}

impl Display for PackageRecord {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}={}={}", self.name, self.version, self.build)
    }
}

impl RepoData {
    /// Parses [`RepoData`] from a file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let contents = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&contents)?)
    }

    /// Builds a [`Vec<RepoDataRecord>`] from the packages in a [`RepoData`] given the source of the
    /// data.
    pub fn into_repo_data_records(self, channel: &Channel) -> Vec<RepoDataRecord> {
        let mut records = Vec::with_capacity(self.packages.len() + self.conda_packages.len());
        let channel_name = channel.canonical_name();
        for (filename, package_record) in self.packages.into_iter().chain(self.conda_packages) {
            records.push(RepoDataRecord {
                url: channel
                    .base_url()
                    .join(&format!("{}/{}", &package_record.subdir, &filename))
                    .expect("failed to build a url from channel and package record"),
                channel: channel_name.clone(),
                package_record,
                file_name: filename,
            })
        }
        records
    }
}

impl PackageRecord {
    /// Creates a new [`PackageRecord`] from a [`IndexJson`] and optionally a size, SHA256 and MD5 hash.
    pub fn from_index_json(
        index: IndexJson,
        size: Option<u64>,
        sha256: Option<String>,
        md5: Option<String>,
    ) -> Self {
        Self {
            name: index.name,
            version: index.version,
            build: index.build,
            build_number: index.build_number,
            subdir: index.subdir.expect("subdir is not set in index.json"),
            md5,
            sha256,
            size,
            arch: index.arch,
            platform: index.platform,
            depends: index.depends,
            constrains: index.constrains,
            track_features: index.track_features,
            features: index.features,
            noarch: index.noarch,
            license: index.license,
            license_family: index.license_family,
            timestamp: index.timestamp,
            legacy_bz2_md5: None,
            legacy_bz2_size: None,
        }
    }
}
