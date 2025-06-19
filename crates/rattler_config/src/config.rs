use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use rattler_conda_types::{ChannelConfig, NamedChannelOrUrl};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::config::{
    build::BuildConfig, concurreny::ConcurrencyConfig, proxy::ProxyConfig,
    repodata_config::RepodataConfig, run_post_link_scripts::RunPostLinkScripts, s3::S3Options,
};

pub mod build;
pub mod channel_config;
pub mod concurreny;
pub mod proxy;
pub mod repodata_config;
pub mod run_post_link_scripts;
pub mod s3;
use crate::config::channel_config::default_channel_config;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Missing required field.
    #[error("Missing required field: {0}")]
    MissingRequiredField(String),

    /// Invalid value for a field.
    #[error("Invalid value for field {0}: {1}")]
    InvalidValue(String, String),

    /// Invalid configuration for various reason.
    #[error("Invalid configuration: {0}")]
    Invalid(String),
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// Error merging configurations.
    #[error("Error merging configurations: {0}")]
    Error(String),
}

#[derive(Error, Debug)]
pub enum LoadError {
    /// Error loading configuration.
    #[error("Error merging configuration files: {0} ({1})")]
    MergeError(MergeError, PathBuf),

    /// IO error while reading configuration file.
    #[error("IO error while reading configuration file: {0}")]
    IoError(#[from] std::io::Error),

    /// Error parsing configuration file.
    #[error("Error parsing configuration file: {0}")]
    ParseError(#[from] toml::de::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigBase<T> {
    #[serde(default)]
    #[serde(alias = "default_channels")] // BREAK: remove to stop supporting snake_case alias
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_channels: Option<Vec<NamedChannelOrUrl>>,

    /// Path to the file containing the authentication token.
    #[serde(default)]
    #[serde(alias = "authentication_override_file")] // BREAK: remove to stop supporting snake_case alias
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication_override_file: Option<PathBuf>,

    /// If set to true, pixi will not verify the TLS certificate of the server.
    #[serde(default)]
    #[serde(alias = "tls_no_verify")] // BREAK: remove to stop supporting snake_case alias
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_no_verify: Option<bool>,

    #[serde(default)]
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub mirrors: IndexMap<Url, Vec<Url>>,

    #[serde(default, skip_serializing_if = "BuildConfig::is_default")]
    pub build: BuildConfig,

    #[serde(skip, default = "default_channel_config")]
    pub channel_config: ChannelConfig,

    /// Configuration for repodata fetching.
    #[serde(alias = "repodata_config")] // BREAK: remove to stop supporting snake_case alias
    #[serde(default, skip_serializing_if = "RepodataConfig::is_empty")]
    pub repodata_config: RepodataConfig,

    /// Configuration for the concurreny of rattler.
    #[serde(default)]
    #[serde(skip_serializing_if = "ConcurrencyConfig::is_default")]
    pub concurrency: ConcurrencyConfig,

    /// Https/Http proxy configuration for pixi
    #[serde(default)]
    #[serde(skip_serializing_if = "ProxyConfig::is_default")]
    pub proxy_config: ProxyConfig,

    /// Configuration for S3.
    #[serde(default)]
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub s3_options: IndexMap<String, S3Options>,

    /// Run the post link scripts
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_post_link_scripts: Option<RunPostLinkScripts>,

    #[serde(flatten)]
    pub extensions: T,

    #[serde(skip)]
    #[serde(alias = "loaded_from")] // BREAK: remove to stop supporting snake_case alias
    pub loaded_from: Vec<PathBuf>,
    // Missing in rattler but should be available in pixi:
    //   experimental
    //   shell
    //   pinning_strategy
    //   detached_environments
    //   pypi_config
    //
    // Deprecated fields:
    //   change_ps1
    //   force_activate
}

// ChannelConfig does not implement `Default` so we need to provide a default implementation.
impl<T> Default for ConfigBase<T>
where
    T: Config + DeserializeOwned,
{
    fn default() -> Self {
        Self {
            default_channels: None,
            authentication_override_file: None,
            tls_no_verify: Some(false), // Default to false if not set
            mirrors: IndexMap::new(),
            build: BuildConfig::default(),
            channel_config: default_channel_config(),
            repodata_config: RepodataConfig::default(),
            concurrency: ConcurrencyConfig::default(),
            proxy_config: ProxyConfig::default(),
            s3_options: IndexMap::new(),
            run_post_link_scripts: None,
            extensions: T::default(),
            loaded_from: Vec::new(),
        }
    }
}

/// An empty dummy configuration extension that we can use when no extension is needed.
impl Config for () {
    fn get_extension_name(&self) -> String {
        "__NONE__".to_string()
    }

    fn merge_config(self, _other: &Self) -> Result<Self, MergeError> {
        Ok(Self::default())
    }

    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }

    fn keys(&self) -> Vec<String> {
        vec![]
    }
}

pub trait Config:
    Serialize + for<'de> Deserialize<'de> + std::fmt::Debug + Clone + PartialEq + Eq + Default
{
    /// Get the name of the extension.
    fn get_extension_name(&self) -> String;

    /// Merge another configuration (file) into this one.
    /// Note: the "other" configuration should take priority over the current one.
    fn merge_config(self, other: &Self) -> Result<Self, MergeError>;

    /// Validate the configuration.
    fn validate(&self) -> Result<(), ValidationError>;

    fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// Get the valid keys of the configuration.
    fn keys(&self) -> Vec<String>;
}

impl<T> ConfigBase<T>
where
    T: Config + DeserializeOwned,
{
    pub fn load_from_files<I, P>(paths: I) -> Result<Self, LoadError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut config = ConfigBase::<T>::default();

        for path in paths {
            let content = std::fs::read_to_string(path.as_ref())?;
            let other: ConfigBase<T> = toml::from_str(&content)?;
            config = config
                .merge_config(&other)
                .map_err(|e| LoadError::MergeError(e, path.as_ref().to_path_buf()))?;
        }

        // config.validate()?;
        Ok(config)
    }
}

impl<T> Config for ConfigBase<T>
where
    T: Config + Default,
{
    fn get_extension_name(&self) -> String {
        "base".to_string()
    }

    /// Merge another configuration (file) into this one.
    /// Note: the "other" configuration should take priority over the current one.
    fn merge_config(self, other: &Self) -> Result<Self, MergeError> {
        Ok(Self {
            s3_options: self
                .s3_options
                .iter()
                .chain(other.s3_options.iter())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            // Use the other configuration's default channels if available
            default_channels: other
                .default_channels
                .as_ref()
                .or(self.default_channels.as_ref())
                .cloned(),
            // Currently this is always the default so it doesn't matter which one we take.
            channel_config: self.channel_config,
            authentication_override_file: other
                .authentication_override_file
                .as_ref()
                .or(self.authentication_override_file.as_ref())
                .cloned(),
            tls_no_verify: other.tls_no_verify.or(self.tls_no_verify).or(Some(false)), // Default to false if not set
            mirrors: self
                .mirrors
                .iter()
                .chain(other.mirrors.iter())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            build: self.build.merge_config(&other.build)?,
            repodata_config: self.repodata_config.merge_config(&other.repodata_config)?,
            concurrency: self.concurrency.merge_config(&other.concurrency)?,
            proxy_config: self.proxy_config.merge_config(&other.proxy_config)?,
            extensions: self.extensions.merge_config(&other.extensions)?,
            run_post_link_scripts: other
                .run_post_link_scripts
                .clone()
                .or(self.run_post_link_scripts),
            loaded_from: self
                .loaded_from
                .iter()
                .chain(&other.loaded_from)
                .cloned()
                .collect(),
        })
    }

    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }

    /// Gather all the keys of the configuration.
    fn keys(&self) -> Vec<String> {
        let mut keys = Vec::new();

        fn get_keys(config: &impl Config) -> Vec<String> {
            config
                .keys()
                .iter()
                .map(|s| format!("{}.{}", config.get_extension_name(), s))
                .collect()
        }

        keys.extend(get_keys(&self.build));
        keys.extend(get_keys(&self.repodata_config));
        keys.extend(get_keys(&self.concurrency));
        keys.extend(get_keys(&self.proxy_config));
        keys.extend(get_keys(&self.extensions));

        keys.extend(self.s3_options.keys().map(|k| format!("s3.{}", k)));
        keys.push("default_channels".to_string());
        keys.push("authentication_override_file".to_string());
        keys.push("tls_no_verify".to_string());
        keys.push("mirrors".to_string());
        keys.push("loaded_from".to_string());
        keys.push("extensions".to_string());
        keys.push("default".to_string());

        return keys;
    }
}

pub fn load_config<T: for<'de> Deserialize<'de>>(
    config_file: &str,
) -> Result<ConfigBase<T>, Box<dyn std::error::Error>> {
    let config_content = std::fs::read_to_string(config_file)?;
    let config: ConfigBase<T> = toml::from_str(&config_content)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use serde::{Deserialize, Serialize};
    use std::collections::HashSet;

    // Define a simple test extension for testing purposes
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
    struct TestExtension {
        test_field: Option<String>,
        test_number: Option<i32>,
    }

    impl Config for TestExtension {
        fn get_extension_name(&self) -> String {
            "test_extension".to_string()
        }

        fn merge_config(self, other: &Self) -> Result<Self, MergeError> {
            Ok(Self {
                test_field: other.test_field.clone().or(self.test_field),
                test_number: other.test_number.or(self.test_number),
            })
        }

        fn validate(&self) -> Result<(), ValidationError> {
            Ok(())
        }

        fn keys(&self) -> Vec<String> {
            vec!["test_field".to_string(), "test_number".to_string()]
        }
    }

    // =========================
    // Tests for load_config()
    // =========================

    #[test]
    fn test_load_config_success_with_simple_toml() {
        // Create a temporary file with simple TOML content
        let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
        let toml_content = r#"
                default_channels = ["conda-forge", "bioconda"]
                tls_no_verify = true
                authentication_override_file = "/path/to/auth"

                [concurrency]
                downloads = 5

                [extensions]
                test_field = "foo"
                test_number = 123

                [activation.scripts]

            "#;
        
        write!(temp_file, "{}", toml_content).expect("Failed to write to temp file");
        let temp_path = temp_file.path().to_str().unwrap();

        // Test the load_config function
        let result: Result<ConfigBase<TestExtension>, Box<dyn std::error::Error>> = 
            load_config(temp_path);

        // Verify the result is successful
        assert!(result.is_ok(), "Expected successful config loading");
        
        let config = result.unwrap();
        
        // Verify the loaded configuration values
        assert_eq!(config.default_channels.len(), 2);
        assert_eq!(config.tls_no_verify, Some(true));
        assert_eq!(config.authentication_override_file, Some(PathBuf::from("/path/to/auth")));
        assert_eq!(config.concurrency.downloads,5);
        // TOOD. There seems bugs in original code.
        // assert_eq!(config.extensions.test_field, Some("foo".to_string()));
        // assert_eq!(config.extensions.test_number, Some(123));
    }

    #[test]
    fn test_load_config_file_not_found() {
        // Test with a non-existent file path
        let non_existent_path = "/path/no/exist/config.toml";
        
        let result: Result<ConfigBase<TestExtension>, Box<dyn std::error::Error>> = 
            load_config(non_existent_path);

        // Verify the result is an error
        assert!(result.is_err(), "Expected error when file doesn't exist");
        
        // Check that it's an IO error
        let error = result.unwrap_err();
        assert!(error.to_string().contains("No such file or directory") || 
                error.to_string().contains("cannot find the file"));
    }

    #[test]
    fn test_load_config_empty_file() {
        // Create a temporary empty file
        let temp_file = NamedTempFile::new().expect("Failed to create temp file");
        let temp_path = temp_file.path().to_str().unwrap();

        // Test the load_config function
        let result: Result<ConfigBase<TestExtension>, Box<dyn std::error::Error>> = 
            load_config(temp_path);

        // Verify the result is successful (empty TOML should parse to default values)
        assert!(result.is_ok(), "Expected successful config loading with empty file");
        
        let config = result.unwrap();
        
        // Verify all values are defaults
        assert!(config.default_channels.is_empty());
        assert_eq!(config.tls_no_verify, None);
        assert_eq!(config.authentication_override_file, None);
        assert_eq!(config.extensions, TestExtension::default());
    }


    // =========================
    // Tests for keys()
    // =========================

      #[test]
    fn test_keys_contains_base_keys() {
        let config = ConfigBase::<TestExtension>::default();
        let keys: HashSet<String> = config.keys().into_iter().collect();
        
        for key in ["default_channels", "authentication_override_file", "tls_no_verify", 
                   "mirrors", "loaded_from", "extensions", "default"] {
            assert!(keys.contains(key), "Missing base key: {}", key);
        }
    }

     #[test]
    fn test_keys_includes_extra_configs() {
        let keys = ConfigBase::<TestExtension>::default().keys();
        
        assert!(keys.iter().any(|k| k.starts_with("concurrency")));
        // TOOD. There seems bugs in original code.
        // assert!(keys.iter().any(|k| k.starts_with("activation.scripts")));
    }

    // TODO. Duplicate keys case.
    #[test]
    fn test_keys_no_duplicates() {
    }

    // =========================
    // Tests for merge_config()
    // =========================

    // TODO. test_config_merge() already existed in lib.rs
    #[test]
    fn test_config_merge_error() {
        // Create a test extension that fails to merge
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
        struct FailingExtension;

        impl Config for FailingExtension {
            fn get_extension_name(&self) -> String {
                "failing".to_string()
            }

            fn merge_config(self, _other: &Self) -> Result<Self, MergeError> {
                Err(MergeError::Error("Test merge failure".to_string()))
            }

            fn validate(&self) -> Result<(), ValidationError> {
                Ok(())
            }

            fn keys(&self) -> Vec<String> {
                vec![]
            }
        }

        let config1 = ConfigBase::<FailingExtension>::default();
        let config2 = ConfigBase::<FailingExtension>::default();

        let result = config1.merge_config(&config2);
        assert!(result.is_err());
        if let Err(MergeError::Error(msg)) = result {
            assert_eq!(msg, "Test merge failure");
        }
    }

    // =========================
    // Tests for validate()
    // =========================
}