use std::fmt;
use std::io;

/// Errors that can occur during CMS file system operations
#[derive(Debug)]
pub enum CmsError {
    /// Bad fn/ft/fm syntax
    InvalidFileSpec(String),
    /// File doesn't exist on any accessed disk
    FileNotFound(String),
    /// Filemode letter not mounted
    DiskNotAccessed(char),
    /// File already exists (for non-overwrite operations)
    FileExists(String),
    /// Disk is read-only
    ReadOnly(char),
    /// Underlying filesystem error
    Io(io::Error),
}

impl fmt::Display for CmsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CmsError::InvalidFileSpec(msg) => write!(f, "Invalid file specification: {}", msg),
            CmsError::FileNotFound(name) => write!(f, "File not found: {}", name),
            CmsError::DiskNotAccessed(letter) => write!(f, "Disk {} not accessed", letter),
            CmsError::FileExists(name) => write!(f, "File already exists: {}", name),
            CmsError::ReadOnly(letter) => write!(f, "Disk {} is read-only", letter),
            CmsError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for CmsError {}

impl From<io::Error> for CmsError {
    fn from(e: io::Error) -> Self {
        CmsError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, CmsError>;
