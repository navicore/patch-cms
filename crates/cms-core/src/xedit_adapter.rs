use xedit_core::error::XeditError;
use xedit_core::filesystem::{FileIdentity, FileSystem};

use crate::error::CmsError;
use crate::filespec::FileSpec;
use crate::filesystem::CmsFileSystem;

/// Adapter that implements xedit-core's `FileSystem` trait using `CmsFileSystem`.
///
/// File IDs are CMS-style filespecs: "PROFILE EXEC A1".
pub struct CmsFs {
    inner: CmsFileSystem,
}

impl CmsFs {
    pub fn new(inner: CmsFileSystem) -> Self {
        Self { inner }
    }
}

impl FileSystem for CmsFs {
    fn read_file(&self, file_id: &str) -> xedit_core::error::Result<String> {
        let spec = FileSpec::parse(file_id).map_err(cms_to_xedit_error)?;
        self.inner.read_file(&spec).map_err(cms_to_xedit_error)
    }

    fn write_file(&self, file_id: &str, content: &str) -> xedit_core::error::Result<()> {
        let spec = FileSpec::parse(file_id).map_err(cms_to_xedit_error)?;
        self.inner
            .write_file(&spec, content)
            .map_err(cms_to_xedit_error)
    }

    fn parse_file_id(&self, file_id: &str) -> Option<FileIdentity> {
        let spec = FileSpec::parse(file_id).ok()?;
        Some(FileIdentity {
            filename: spec.filename().to_string(),
            filetype: spec.filetype().to_string(),
            filemode: spec.filemode(),
        })
    }
}

fn cms_to_xedit_error(e: CmsError) -> XeditError {
    match e {
        CmsError::FileNotFound(name) => XeditError::FileNotFound(name),
        CmsError::ReadOnly(_) => XeditError::ReadOnly,
        CmsError::Io(io_err) => XeditError::Io(io_err),
        other => XeditError::InvalidCommand(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::minidisk::AccessMode;
    use tempfile::TempDir;

    fn setup_cms_fs() -> (TempDir, CmsFs) {
        let dir = TempDir::new().unwrap();
        let mut cms = CmsFileSystem::new();
        cms.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        (dir, CmsFs::new(cms))
    }

    #[test]
    fn read_write_through_trait() {
        let (_dir, fs) = setup_cms_fs();
        fs.write_file("TEST DATA A", "hello cms\n").unwrap();
        let content = fs.read_file("TEST DATA A").unwrap();
        assert_eq!(content, "hello cms\n");
    }

    #[test]
    fn file_not_found_error() {
        let (_dir, fs) = setup_cms_fs();
        let result = fs.read_file("NOFILE DATA A");
        assert!(result.is_err());
        match result.unwrap_err() {
            XeditError::FileNotFound(_) => {}
            other => panic!("Expected FileNotFound, got {:?}", other),
        }
    }

    #[test]
    fn parse_file_id_cms() {
        let (_dir, fs) = setup_cms_fs();
        let id = fs.parse_file_id("PROFILE EXEC A1").unwrap();
        assert_eq!(id.filename, "PROFILE");
        assert_eq!(id.filetype, "EXEC");
        assert_eq!(id.filemode, "A1");
    }

    #[test]
    fn readonly_error_mapping() {
        let dir = TempDir::new().unwrap();
        let mut cms = CmsFileSystem::new();
        cms.access_disk('B', dir.path().join("b"), AccessMode::ReadOnly)
            .unwrap();
        let fs = CmsFs::new(cms);
        let result = fs.write_file("TEST DATA B", "data");
        assert!(result.is_err());
        match result.unwrap_err() {
            XeditError::ReadOnly => {}
            other => panic!("Expected ReadOnly, got {:?}", other),
        }
    }
}
