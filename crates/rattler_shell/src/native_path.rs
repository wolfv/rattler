/// Convert a path to a native path.
///
/// On Unix, this is a no-op.
///
/// On Windows, this converts the path to a unix style path using the cygpath command.
use std::fs;
use std::path::Path;
use std::process::Command;

/// Convert a path to a native path.
#[cfg(unix)]
pub fn convert_native_path(path: &Path) -> Result<String, std::io::Error> {
    Ok(fs::canonicalize(path)?.to_string_lossy().to_string())
}

/// Convert a path to a native path.
///
/// On Windows, this converts the path to a unix style path using the cygpath command.
#[cfg(windows)]
pub fn convert_native_path(path: &Path) -> Result<String, std::io::Error> {
    // Translate path from windows style to unix style
    let mut cygpath = Command::new("cygpath");
    cygpath.arg("--unix");
    cygpath.arg(path);

    let output = cygpath.output()?;

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Invalid UTF-8"))?.trim().to_owned();

    Ok(stdout)
}

mod test {
    use super::convert_native_path;
    use std::path::Path;

    #[test]
    #[cfg(unix)]
    fn test_convert_native_path_unix() {
        let path = Path::new("/tmp/foo/bar");
        let native_path = convert_native_path(path).unwrap();
        assert_eq!(native_path, "/tmp/foo/bar");
    }

    #[test]
    #[cfg(windows)]
    fn test_convert_native_path_windows() {
        let path = Path::new("C:\\tmp\\foo\\bar");
        let native_path = convert_native_path(path).unwrap();
        assert_eq!(native_path, "/tmp/foo/bar");
    }
}