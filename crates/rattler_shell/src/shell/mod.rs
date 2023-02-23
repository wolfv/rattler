use std::{
    fmt::{self, Display, Formatter},
    path::{Path, PathBuf},
};

pub trait Shell {
    fn set_env_var(&self, env_var: &str, value: &str) -> String;
    fn unset_env_var(&self, env_var: &str) -> String;
    fn extension(&self) -> String;
    fn run_script(&self, path: &Path) -> String;
    fn set_path(&self, paths: &[PathBuf]) -> String;
}

pub struct Bash;

impl Shell for Bash {
    fn set_env_var(&self, env_var: &str, value: &str) -> String {
        format!("export {}=\"{}\"", env_var, value)
    }

    fn unset_env_var(&self, env_var: &str) -> String {
        format!("unset {}", env_var)
    }

    fn run_script(&self, path: &Path) -> String {
        format!(". \"{}\"", path.to_string_lossy())
    }

    fn extension(&self) -> String {
        "sh".to_string()
    }

    fn set_path(&self, paths: &[PathBuf]) -> String {
        let path = paths
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect::<Vec<&str>>()
            .join(":");
        self.set_env_var("PATH", &path)
    }
}

pub struct Zsh;

impl Shell for Zsh {
    fn set_env_var(&self, env_var: &str, value: &str) -> String {
        format!("export {}=\"{}\"", env_var, value)
    }

    fn unset_env_var(&self, env_var: &str) -> String {
        format!("unset {}", env_var)
    }

    fn run_script(&self, path: &Path) -> String {
        format!(". \"{}\"", path.to_string_lossy())
    }

    fn extension(&self) -> String {
        "zsh".to_string()
    }

    fn set_path(&self, paths: &[PathBuf]) -> String {
        let path = paths
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect::<Vec<&str>>()
            .join(":");
        self.set_env_var("PATH", &path)
    }
}

pub struct Fish;

impl Shell for Fish {
    fn set_env_var(&self, env_var: &str, value: &str) -> String {
        format!("set -gx {} \"{}\"", env_var, value)
    }

    fn unset_env_var(&self, env_var: &str) -> String {
        format!("set -e {}", env_var)
    }

    fn run_script(&self, path: &Path) -> String {
        format!(". \"{}\"", path.to_string_lossy())
    }

    fn extension(&self) -> String {
        "fish".to_string()
    }

    fn set_path(&self, paths: &[PathBuf]) -> String {
        let path = paths
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect::<Vec<&str>>()
            .join(":");
        self.set_env_var("PATH", &path)
    }
}

pub struct CmdExe;

impl Shell for CmdExe {
    fn set_env_var(&self, env_var: &str, value: &str) -> String {
        format!("set \"{}={}\"", env_var, value)
    }

    fn unset_env_var(&self, env_var: &str) -> String {
        format!("set \"{}=\"", env_var)
    }

    fn run_script(&self, path: &Path) -> String {
        format!(". {}", path.to_string_lossy())
    }

    fn extension(&self) -> String {
        "bat".to_string()
    }

    fn set_path(&self, paths: &[PathBuf]) -> String {
        let path = paths
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect::<Vec<&str>>()
            .join(";");
        self.set_env_var("PATH", &path)
    }
}

pub struct PowerShell;

impl Shell for PowerShell {
    fn set_env_var(&self, env_var: &str, value: &str) -> String {
        format!("$env:{} = \"{}\"", env_var, value)
    }

    fn unset_env_var(&self, env_var: &str) -> String {
        format!("Remove-Item Env:{}", env_var)
    }

    fn run_script(&self, path: &Path) -> String {
        format!(". \"{}\"", path.to_string_lossy())
    }

    fn extension(&self) -> String {
        "ps1".to_string()
    }

    fn set_path(&self, paths: &[PathBuf]) -> String {
        let path = paths
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect::<Vec<&str>>()
            .join(";");
        self.set_env_var("PATH", &path)
    }
}

pub struct Xonsh;

impl Shell for Xonsh {
    fn set_env_var(&self, env_var: &str, value: &str) -> String {
        format!("setx {} \"{}\"", env_var, value)
    }

    fn unset_env_var(&self, env_var: &str) -> String {
        format!("setx {} \"\"", env_var)
    }

    fn run_script(&self, path: &Path) -> String {
        format!(". {}", path.to_string_lossy())
    }

    fn extension(&self) -> String {
        "xsh".to_string()
    }

    fn set_path(&self, paths: &[PathBuf]) -> String {
        let path = paths
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect::<Vec<&str>>()
            .join(";");
        self.set_env_var("PATH", &path)
    }
}

pub struct ShellScript {
    shell: Box<dyn Shell>,
    lines: Vec<String>,
}

impl ShellScript {
    pub fn new(shell: Box<dyn Shell>) -> ShellScript {
        ShellScript {
            shell,
            lines: Vec::new(),
        }
    }

    pub fn set_env_var(&mut self, env_var: &str, value: &str) -> &mut ShellScript {
        self.lines.push(self.shell.set_env_var(env_var, value));
        self
    }

    pub fn unset_env_var(&mut self, env_var: &str) -> &mut ShellScript {
        self.lines.push(self.shell.unset_env_var(env_var));
        self
    }

    pub fn set_path(&mut self, paths: &[PathBuf]) -> &mut ShellScript {
        self.lines.push(self.shell.set_path(paths));
        self
    }

    pub fn run_script(&mut self, path: &Path) -> &mut ShellScript {
        self.lines.push(self.shell.run_script(path));
        self
    }
}

impl Display for ShellScript {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_bash() {
        let mut script = ShellScript::new(Box::new(Bash));
        script
            .set_env_var("FOO", "bar")
            .unset_env_var("FOO")
            .run_script(&PathBuf::from_str("foo.sh").expect("blah"));

        assert_eq!(
            script.to_string(),
            "export FOO=bar\n\
             unset FOO\n\
             . foo.sh"
        );
    }
}
