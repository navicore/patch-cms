use std::path::{Path, PathBuf};

/// Access mode for a minidisk, derived from the filemode digit.
///
/// Digits 0-1 grant read-write access; digits 2-6 grant read-only access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    ReadWrite,
    ReadOnly,
}

impl AccessMode {
    /// Determine access mode from a filemode digit (0-6).
    pub fn from_digit(digit: u8) -> Self {
        match digit {
            0 | 1 => AccessMode::ReadWrite,
            _ => AccessMode::ReadOnly,
        }
    }
}

/// A minidisk maps a filemode letter (A-Z) to a real directory on the host
/// filesystem, with an associated access mode.
#[derive(Debug, Clone)]
pub struct Minidisk {
    letter: char,
    path: PathBuf,
    access: AccessMode,
}

impl Minidisk {
    pub fn new(letter: char, path: PathBuf, access: AccessMode) -> Self {
        Minidisk {
            letter: letter.to_ascii_uppercase(),
            path,
            access,
        }
    }

    pub fn letter(&self) -> char {
        self.letter
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn access(&self) -> AccessMode {
        self.access
    }

    pub fn is_writable(&self) -> bool {
        self.access == AccessMode::ReadWrite
    }

    /// Resolve a filename and filetype to a full path on this disk.
    /// Files are stored as `{disk_path}/{fn}.{ft}` in lowercase.
    pub fn file_path(&self, filename: &str, filetype: &str) -> PathBuf {
        let name = format!("{}.{}", filename.to_lowercase(), filetype.to_lowercase());
        self.path.join(name)
    }

    /// Check whether a regular file (not a symlink) exists on this disk.
    pub fn file_exists(&self, filename: &str, filetype: &str) -> bool {
        let path = self.file_path(filename, filetype);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta.file_type().is_file(),
            Err(_) => false,
        }
    }

    /// Create the disk directory if it doesn't exist.
    pub fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_path_construction() {
        let disk = Minidisk::new('A', PathBuf::from("/tmp/cms/a"), AccessMode::ReadWrite);
        let path = disk.file_path("PROFILE", "EXEC");
        assert_eq!(path, PathBuf::from("/tmp/cms/a/profile.exec"));
    }

    #[test]
    fn file_exists_false() {
        let disk = Minidisk::new(
            'A',
            PathBuf::from("/tmp/nonexistent"),
            AccessMode::ReadWrite,
        );
        assert!(!disk.file_exists("NOFILE", "NOEXT"));
    }

    #[test]
    fn file_exists_true() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.data"), "hello").unwrap();
        let disk = Minidisk::new('A', dir.path().to_path_buf(), AccessMode::ReadWrite);
        assert!(disk.file_exists("TEST", "DATA"));
    }

    #[test]
    fn access_mode_from_digit() {
        assert_eq!(AccessMode::from_digit(0), AccessMode::ReadWrite);
        assert_eq!(AccessMode::from_digit(1), AccessMode::ReadWrite);
        assert_eq!(AccessMode::from_digit(2), AccessMode::ReadOnly);
        assert_eq!(AccessMode::from_digit(3), AccessMode::ReadOnly);
        assert_eq!(AccessMode::from_digit(6), AccessMode::ReadOnly);
    }

    #[test]
    fn writable_check() {
        let rw = Minidisk::new('A', PathBuf::from("/tmp"), AccessMode::ReadWrite);
        let ro = Minidisk::new('B', PathBuf::from("/tmp"), AccessMode::ReadOnly);
        assert!(rw.is_writable());
        assert!(!ro.is_writable());
    }
}
