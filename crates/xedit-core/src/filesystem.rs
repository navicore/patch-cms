use std::path::Path;

use crate::error::{Result, XeditError};

/// Identity components extracted from a file identifier.
///
/// For native OS files, filename comes from the path stem (uppercased)
/// and filetype from the extension (uppercased). For CMS files, these
/// map directly to the CMS fn/ft/fm components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIdentity {
    pub filename: String,
    pub filetype: String,
    pub filemode: String,
}

/// Abstraction over file I/O so xedit-core can work with native OS files
/// or CMS minidisk files without compile-time coupling.
///
/// `file_id` is an opaque string — `NativeFs` treats it as an OS path,
/// while a CMS adapter treats it as a filespec ("PROFILE EXEC A1").
pub trait FileSystem {
    fn read_file(&self, file_id: &str) -> Result<String>;
    fn write_file(&self, file_id: &str, content: &str) -> Result<()>;
    fn parse_file_id(&self, file_id: &str) -> Option<FileIdentity>;
}

/// Default filesystem implementation that delegates to `std::fs`.
pub struct NativeFs;

impl FileSystem for NativeFs {
    fn read_file(&self, file_id: &str) -> Result<String> {
        std::fs::read_to_string(file_id).map_err(|_| XeditError::FileNotFound(file_id.to_string()))
    }

    fn write_file(&self, file_id: &str, content: &str) -> Result<()> {
        std::fs::write(file_id, content)?;
        Ok(())
    }

    fn parse_file_id(&self, file_id: &str) -> Option<FileIdentity> {
        let path = Path::new(file_id);
        let filename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_uppercase();
        let filetype = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_uppercase();

        if filename.is_empty() {
            return None;
        }

        Some(FileIdentity {
            filename,
            filetype,
            filemode: "A1".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn native_fs_read_write_roundtrip() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "hello world").unwrap();
        tmp.flush().unwrap();

        let fs = NativeFs;
        let path = tmp.path().to_str().unwrap();
        let content = fs.read_file(path).unwrap();
        assert_eq!(content, "hello world");

        fs.write_file(path, "updated").unwrap();
        let content = fs.read_file(path).unwrap();
        assert_eq!(content, "updated");
    }

    #[test]
    fn native_fs_read_nonexistent() {
        let fs = NativeFs;
        let result = fs.read_file("/tmp/nonexistent_xedit_test_file_12345.txt");
        assert!(result.is_err());
    }

    #[test]
    fn parse_file_id_with_extension() {
        let fs = NativeFs;
        let id = fs.parse_file_id("/some/path/profile.exec").unwrap();
        assert_eq!(id.filename, "PROFILE");
        assert_eq!(id.filetype, "EXEC");
        assert_eq!(id.filemode, "A1");
    }

    #[test]
    fn parse_file_id_without_extension() {
        let fs = NativeFs;
        let id = fs.parse_file_id("/some/path/makefile").unwrap();
        assert_eq!(id.filename, "MAKEFILE");
        assert_eq!(id.filetype, "");
        assert_eq!(id.filemode, "A1");
    }

    #[test]
    fn parse_file_id_empty() {
        let fs = NativeFs;
        assert!(fs.parse_file_id("").is_none());
    }
}
