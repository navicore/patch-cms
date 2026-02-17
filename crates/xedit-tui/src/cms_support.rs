use cms_core::filesystem::CmsFileSystem;
use cms_core::minidisk::AccessMode;
use cms_core::{CmsFs, CommandProcessor};

/// Set up a CMS environment from a base directory.
///
/// The directory layout is:
/// ```text
/// <base_path>/
///   a/    # A-disk (read-write)
///   b/    # B-disk (read-only, optional)
///   ...
///   z/    # Z-disk (read-only, optional)
/// ```
///
/// Returns a `CommandProcessor` (for CMS commands) and a `CmsFs` (for file I/O
/// through the `FileSystem` trait). Each gets its own `CmsFileSystem` instance
/// with the same disk mappings — safe because `CmsFileSystem` has no in-memory
/// cache.
pub fn setup_cms(base_path: &str) -> Result<(CommandProcessor, CmsFs), String> {
    let base = std::path::Path::new(base_path);
    if !base.is_dir() {
        return Err(format!("CMS base path is not a directory: {}", base_path));
    }

    let mut fs1 = CmsFileSystem::new();
    let mut fs2 = CmsFileSystem::new();

    // A-disk is always read-write
    let a_path = base.join("a");
    if !a_path.is_dir() {
        return Err(format!("A-disk directory not found: {}", a_path.display()));
    }
    fs1.access_disk('A', &a_path, AccessMode::ReadWrite)
        .map_err(|e| e.to_string())?;
    fs2.access_disk('A', &a_path, AccessMode::ReadWrite)
        .map_err(|e| e.to_string())?;

    // B-Z disks are read-only if they exist
    for letter in 'b'..='z' {
        let disk_path = base.join(letter.to_string());
        if disk_path.is_dir() {
            let upper = letter.to_ascii_uppercase();
            fs1.access_disk(upper, &disk_path, AccessMode::ReadOnly)
                .map_err(|e| e.to_string())?;
            fs2.access_disk(upper, &disk_path, AccessMode::ReadOnly)
                .map_err(|e| e.to_string())?;
        }
    }

    let processor = CommandProcessor::new(fs1);
    let cms_fs = CmsFs::new(fs2);

    Ok((processor, cms_fs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use xedit_core::filesystem::FileSystem;

    fn make_cms_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("a")).unwrap();
        dir
    }

    #[test]
    fn setup_cms_creates_processor_and_fs() {
        let dir = make_cms_dir();
        let (mut proc, fs) = setup_cms(dir.path().to_str().unwrap()).unwrap();

        // CmsFs can write and read files
        fs.write_file("TEST DATA A", "hello\n").unwrap();
        let content = fs.read_file("TEST DATA A").unwrap();
        assert_eq!(content, "hello\n");

        // CommandProcessor can execute STATE
        let result = proc.execute("STATE TEST DATA A");
        assert_eq!(result.rc, 0);
    }

    #[test]
    fn setup_cms_with_optional_disks() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("a")).unwrap();
        std::fs::create_dir(dir.path().join("s")).unwrap();

        let (_proc, _fs) = setup_cms(dir.path().to_str().unwrap()).unwrap();
    }

    #[test]
    fn setup_cms_missing_a_disk() {
        let dir = TempDir::new().unwrap();
        let result = setup_cms(dir.path().to_str().unwrap());
        match result {
            Err(msg) => assert!(
                msg.contains("A-disk"),
                "Expected A-disk error, got: {}",
                msg
            ),
            Ok(_) => panic!("Expected error for missing A-disk"),
        }
    }

    #[test]
    fn setup_cms_bad_path() {
        let result = setup_cms("/nonexistent/path/that/does/not/exist");
        assert!(result.is_err());
    }
}
