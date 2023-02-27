// #![deny(missing_docs)]

//! This crate provides helper functions to activate and deactivate virtual environments.

mod shell;

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use indexmap::IndexMap;
use shell::{Bash, CmdExe, Fish, PowerShell, ShellScript, Xonsh, Zsh};

/// Enumeration of different shell types that are recognized by rattler
#[derive(Copy, Clone, Debug)]
pub enum ShellType {
    /// The `bash` shell
    Bash,
    /// The `zsh` shell
    Zsh,
    /// The `fish` shell
    Fish,
    /// The `xonsh` shell
    Xonsh,
    /// `powershell` or `pwsh`
    Powershell,
    /// The Windows Command Prompt (cmd.exe)
    CmdExe,
}

impl ShellType {
    /// Returns the file extension for this shell type.
    fn suffix(&self) -> &'static OsStr {
        match self {
            ShellType::Bash => OsStr::new("sh"),
            ShellType::Zsh => OsStr::new("zsh"),
            ShellType::Xonsh => OsStr::new("xsh"),
            ShellType::Fish => OsStr::new("fish"),
            ShellType::Powershell => OsStr::new("ps1"),
            ShellType::CmdExe => OsStr::new("bat"),
        }
    }
}

/// An enumeration of the different operating systems that are supported by rattler
#[derive(Copy, Clone, Debug)]
pub enum OperatingSystem {
    /// The Windows operating system
    Windows,

    /// The macOS operating systems
    MacOS,

    /// The Linux family operating systems
    Linux,
}

/// A struct that holds values for the activation and deactivation
/// process of an environment, e.g. activation scripts to execute or environment variables to set.
pub struct Activator {
    /// The path to the root of the conda environment
    pub target_prefix: PathBuf,

    /// The type of shell that is being activated
    pub shell_type: ShellType,

    /// Paths that need to be added to the PATH environment variable
    pub paths: Vec<PathBuf>,

    /// A list of scripts to run when activating the environment
    pub activation_scripts: Vec<PathBuf>,

    /// A list of scripts to run when deactivating the environment
    pub deactivation_scripts: Vec<PathBuf>,

    /// A list of environment variables to set when activating the environment
    pub env_vars: IndexMap<String, String>,
}

/// Collect all script files that match a certain shell type from a given path.
/// The files are sorted by their filename.
/// If the path does not exist, an empty vector is returned.
/// If the path is not a directory, an error is returned.
///
/// # Arguments
///
/// * `path` - The path to the directory that contains the scripts
/// * `shell_type` - The type of shell that the scripts are for
///
/// # Returns
///
/// A vector of paths to the scripts
///
/// # Errors
///
/// If the path is not a directory, an error is returned.
fn collect_scripts(path: &Path, shell_type: ShellType) -> Result<Vec<PathBuf>, std::io::Error> {
    // Check if path exists
    if !path.exists() {
        return Ok(vec![]);
    }

    let paths = fs::read_dir(path)?;

    let mut scripts = paths
        .into_iter()
        .filter_map(|r| r.ok())
        .map(|r| r.path())
        .filter(|path| path.is_file() && path.extension() == Some(shell_type.suffix()))
        .collect::<Vec<_>>();

    scripts.sort();

    Ok(scripts)
}

/// Error that can occur when activating a conda environment
#[derive(thiserror::Error, Debug)]
pub enum ActivationError {
    /// An error that can occur when reading or writing files
    #[error(transparent)]
    IoError(#[from] std::io::Error),

    /// An error that can occur when parsing JSON
    #[error("Invalid json for environment vars: {0} in file {1:?}")]
    InvalidEnvVarFileJson(serde_json::Error, PathBuf),

    /// An error that can occur wiht malformed JSON when parsing files in the `env_vars.d` directory
    #[error("Malformed JSON: not a plain JSON object in file {file:?}")]
    InvalidEnvVarFileJsonNoObject {
        /// The path to the file that contains the malformed JSON
        file: PathBuf,
    },

    /// An error that can occur when `state` file is malformed
    #[error("Malformed JSON: file does not contain JSON object at key env_vars in file {file:?}")]
    InvalidEnvVarFileStateFile {
        /// The path to the file that contains the malformed JSON
        file: PathBuf,
    },
}

/// Collect all environment variables that are set in a conda environment.
/// The environment variables are collected from the `state` file and the `env_vars.d` directory in the given prefix
/// and are returned as a ordered map.
///
/// # Arguments
///
/// * `prefix` - The path to the root of the conda environment
///
/// # Returns
///
/// A map of environment variables
///
/// # Errors
///
/// If the `state` file or the `env_vars.d` directory cannot be read, an error is returned.
fn collect_env_vars(prefix: &Path) -> Result<IndexMap<String, String>, ActivationError> {
    let state_file = prefix.join("conda-meta/state");
    let pkg_env_var_dir = prefix.join("etc/conda/env_vars.d");
    let mut env_vars = IndexMap::new();

    if pkg_env_var_dir.exists() {
        let env_var_files = pkg_env_var_dir.read_dir()?;

        let mut env_var_files = env_var_files
            .into_iter()
            .filter_map(|r| r.ok())
            .map(|e| e.path())
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();

        // sort env var files to get a deterministic order
        env_var_files.sort();

        let env_var_json_files = env_var_files
            .iter()
            .map(|path| {
                fs::read_to_string(path)?
                    .parse::<serde_json::Value>()
                    .map_err(|e| ActivationError::InvalidEnvVarFileJson(e, path.to_path_buf()))
            })
            .collect::<Result<Vec<serde_json::Value>, ActivationError>>()?;

        for (env_var_json, env_var_file) in env_var_json_files.iter().zip(env_var_files.iter()) {
            let env_var_json = env_var_json.as_object().ok_or_else(|| {
                ActivationError::InvalidEnvVarFileJsonNoObject {
                    file: pkg_env_var_dir.to_path_buf(),
                }
            })?;

            for (key, value) in env_var_json {
                if let Some(value) = value.as_str() {
                    env_vars.insert(key.to_string(), value.to_string());
                } else {
                    tracing::warn!(
                        "WARNING: environment variable {key} has no string value (path: {env_var_file:?})");
                }
            }
        }
    }

    if state_file.exists() {
        let state_json = fs::read_to_string(&state_file)?;

        // load json but preserve the order of dicts - for this we use the serde preserve_order feature
        let state_json: serde_json::Value = serde_json::from_str(&state_json)
            .map_err(|e| ActivationError::InvalidEnvVarFileJson(e, state_file.to_path_buf()))?;

        let state_env_vars = state_json["env_vars"].as_object().ok_or_else(|| {
            ActivationError::InvalidEnvVarFileStateFile {
                file: state_file.to_path_buf(),
            }
        })?;

        for (key, value) in state_env_vars {
            if state_env_vars.contains_key(key) {
                tracing::warn!(
                    "WARNING: environment variable {key} already defined in packages (path: {state_file:?})");
            }

            if let Some(value) = value.as_str() {
                env_vars.insert(key.to_uppercase().to_string(), value.to_string());
            } else {
                tracing::warn!(
                    "WARNING: environment variable {key} has no string value (path: {state_file:?})");
            }
        }
    }
    Ok(env_vars)
}

/// Return a vector of path entries that are prefixed with the given path.
///
/// # Arguments
///
/// * `prefix` - The path to prefix the path entries with
/// * `operating_system` - The operating system that the path entries are for
///
/// # Returns
///
/// A vector of path entries
fn prefix_path_entries(prefix: &Path, operating_system: OperatingSystem) -> Vec<PathBuf> {
    match operating_system {
        OperatingSystem::Windows => {
            vec![
                prefix.to_path_buf(),
                prefix.join("Library/mingw-w64/bin"),
                prefix.join("Library/usr/bin"),
                prefix.join("Library/bin"),
                prefix.join("Scripts"),
                prefix.join("bin"),
            ]
        }
        OperatingSystem::MacOS | OperatingSystem::Linux => {
            vec![prefix.join("bin")]
        }
    }
}

impl Activator {
    /// Create a new activator for the given conda environment.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the root of the conda environment
    /// * `shell_type` - The shell type that the activator is for
    /// * `operating_system` - The operating system that the activator is for
    ///
    /// # Returns
    ///
    /// A new activator
    ///
    /// # Examples
    ///
    /// ```
    /// use rattler_shell_helpers::{Activator, OperatingSystem, ShellType};
    /// use std::path::PathBuf;
    ///
    /// let activator = Activator::from_path(&PathBuf::from("tests/fixtures/env_vars"), ShellType::Bash, OperatingSystem::MacOS).unwrap();
    /// assert_eq!(activator.paths.len(), 1);
    /// assert_eq!(activator.paths[0], PathBuf::from("tests/fixtures/env_vars/bin"));
    /// ```
    pub fn from_path(
        path: &Path,
        shell_type: ShellType,
        operating_system: OperatingSystem,
    ) -> Result<Activator, ActivationError> {
        let activation_scripts = collect_scripts(&path.join("etc/conda/activate.d"), shell_type)?;

        let deactivation_scripts =
            collect_scripts(&path.join("etc/conda/deactivate.d"), shell_type)?;

        let env_vars = collect_env_vars(path)?;

        let paths = prefix_path_entries(path, operating_system);

        Ok(Activator {
            target_prefix: path.to_path_buf(),
            shell_type,
            paths,
            activation_scripts,
            deactivation_scripts,
            env_vars,
        })
    }

    /// Create a activation script for a given shell
    pub fn activation_script(&self, deactivate: Option<Activator>) -> String {
        let mut script = match self.shell_type {
            ShellType::Bash => ShellScript::new(Box::new(Bash)),
            ShellType::Zsh => ShellScript::new(Box::new(Zsh)),
            ShellType::Powershell => ShellScript::new(Box::new(PowerShell)),
            ShellType::CmdExe => ShellScript::new(Box::new(CmdExe)),
            ShellType::Fish => ShellScript::new(Box::new(Fish)),
            ShellType::Xonsh => ShellScript::new(Box::new(Xonsh)),
        };

        let path = std::env::var("PATH").unwrap_or_else(|_| "".to_string());
        let mut path_elements = std::env::split_paths(&path).collect::<Vec<_>>();

        if let Some(deactivate) = deactivate {
            for (key, _) in &deactivate.env_vars {
                script.unset_env_var(key);
            }

            for s in &deactivate.deactivation_scripts {
                script.run_script(s);
            }

            path_elements.retain(|x| !deactivate.paths.contains(x));
        }

        // prepend new paths
        let path_elements = [self.paths.clone(), path_elements].concat();

        script.set_path(path_elements.as_slice());

        // deliberately not taking care of `CONDA_SHLVL` or any other complications at this point
        script.set_env_var("CONDA_PREFIX", &self.target_prefix.to_string_lossy());

        for (key, value) in &self.env_vars {
            script.set_env_var(key, value);
        }

        for s in &self.activation_scripts {
            script.run_script(s);
        }

        script.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::shell::Shell;

    use super::*;
    use tempdir::TempDir;

    #[test]
    fn test_collect_scripts() {
        let tdir = TempDir::new("test").unwrap();

        let path = tdir.path().join("etc/conda/activate.d/");
        fs::create_dir_all(&path).unwrap();

        let script1 = path.join("script1.sh");
        let script2 = path.join("aaa.sh");
        let script3 = path.join("xxx.sh");

        fs::write(&script1, "").unwrap();
        fs::write(&script2, "").unwrap();
        fs::write(&script3, "").unwrap();

        let shell_type = ShellType::Bash;

        let scripts = collect_scripts(&path, shell_type).unwrap();
        assert_eq!(scripts.len(), 3);
        assert_eq!(scripts[0], script2);
        assert_eq!(scripts[1], script1);
        assert_eq!(scripts[2], script3);

        let activator = Activator::from_path(
            &tdir.path().to_path_buf(),
            shell_type,
            OperatingSystem::MacOS,
        )
        .unwrap();
        assert_eq!(activator.activation_scripts.len(), 3);
        assert_eq!(activator.activation_scripts[0], script2);
        assert_eq!(activator.activation_scripts[1], script1);
        assert_eq!(activator.activation_scripts[2], script3);
    }

    #[test]
    fn test_collect_env_vars() {
        let tdir = TempDir::new("test").unwrap();
        let path = tdir.path().join("conda-meta/state");
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        let quotes = r#"{"env_vars": {"Hallo": "myval", "TEST": "itsatest", "AAA": "abcdef"}}"#;
        fs::write(&path, quotes).unwrap();

        let env_vars = collect_env_vars(&tdir.path().to_path_buf()).unwrap();
        assert_eq!(env_vars.len(), 3);

        assert_eq!(env_vars["HALLO"], "myval");
        assert_eq!(env_vars["TEST"], "itsatest");
        assert_eq!(env_vars["AAA"], "abcdef");
    }

    #[test]
    fn test_collect_env_vars_with_directory() {
        let tdir = TempDir::new("test").unwrap();
        let state_path = tdir.path().join("conda-meta/state");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();

        let content_pkg_1 = r#"{"VAR1": "someval", "TEST": "pkg1-test", "III": "super"}"#;
        let content_pkg_2 = r#"{"VAR1": "overwrite1", "TEST2": "pkg2-test"}"#;

        let env_var_d = tdir.path().join("etc/conda/env_vars.d");
        fs::create_dir_all(&env_var_d).expect("Could not create env vars directory");

        let pkg1 = env_var_d.join("pkg1.json");
        let pkg2 = env_var_d.join("pkg2.json");

        fs::write(&pkg1, content_pkg_1).expect("could not write file");
        fs::write(&pkg2, content_pkg_2).expect("could not write file");

        let quotes = r#"{"env_vars": {"Hallo": "myval", "TEST": "itsatest", "AAA": "abcdef"}}"#;
        fs::write(&state_path, quotes).unwrap();

        let env_vars =
            collect_env_vars(&tdir.path().to_path_buf()).expect("Could not load env vars");
        assert_eq!(env_vars.len(), 6);

        assert_eq!(env_vars["VAR1"], "overwrite1");
        assert_eq!(env_vars["TEST"], "itsatest");
        assert_eq!(env_vars["III"], "super");
        assert_eq!(env_vars["TEST2"], "pkg2-test");
        assert_eq!(env_vars["HALLO"], "myval");
        assert_eq!(env_vars["AAA"], "abcdef");

        // assert order of keys
        let mut keys = env_vars.keys();
        let key_vec = vec![
            "VAR1", // overwritten - should this be sorted down?
            "TEST", "III", "TEST2", "HALLO", "AAA",
        ];

        for key in key_vec {
            assert_eq!(keys.next().unwrap(), key);
        }
    }

    #[test]
    fn test_add_to_path() {
        let prefix = PathBuf::from_str("/opt/conda").unwrap();
        let new_paths = prefix_path_entries(&prefix, OperatingSystem::MacOS);
        assert_eq!(new_paths.len(), 1);
    }

    fn create_temp_dir() -> TempDir {
        let tempdir = TempDir::new("test").unwrap();
        let path = tempdir.path().join("etc/conda/activate.d/");
        fs::create_dir_all(&path).unwrap();

        let script1 = path.join("script1.sh");

        fs::write(&script1, "").unwrap();

        tempdir
    }

    fn get_script(shell_type: ShellType) -> String {
        let tdir = create_temp_dir();
        let old_path_var = std::env::var("PATH").unwrap();
        std::env::set_var("PATH", "/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin");

        let activator = Activator::from_path(
            &tdir.path().to_path_buf(),
            shell_type,
            OperatingSystem::MacOS,
        )
        .unwrap();

        let script = activator.activation_script(None);
        let prefix = tdir.path().to_str().unwrap();
        let script = script.replace(prefix, "__PREFIX__");

        std::env::set_var("PATH", old_path_var);
        script
    }

    #[test]
    fn test_activation_script_bash() {
        let script = get_script(ShellType::Bash);
        insta::assert_snapshot!(script);
    }

    #[test]
    fn test_activation_script_zsh() {
        let script = get_script(ShellType::Zsh);
        insta::assert_snapshot!(script);
    }

    #[test]
    fn test_activation_script_fish() {
        let script = get_script(ShellType::Fish);
        insta::assert_snapshot!(script);
    }

    #[test]
    fn test_activation_script_powershell() {
        let script = get_script(ShellType::Powershell);
        insta::assert_snapshot!(script);
    }

    #[test]
    fn test_activation_script_cmd() {
        let script = get_script(ShellType::CmdExe);
        insta::assert_snapshot!(script);
    }

    #[test]
    fn test_activation_script_xonsh() {
        let script = get_script(ShellType::Xonsh);
        insta::assert_snapshot!(script);
    }
}
