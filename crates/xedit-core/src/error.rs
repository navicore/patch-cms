use std::fmt;
use std::io;

/// Errors that can occur during XEDIT operations
#[derive(Debug)]
pub enum XeditError {
    FileNotFound(String),
    Io(io::Error),
    InvalidCommand(String),
    TargetNotFound(String),
    InvalidTarget(String),
    PrefixError(String),
    FileModified,
    ReadOnly,
    NoFile,
}

impl fmt::Display for XeditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XeditError::FileNotFound(name) => write!(f, "File not found: {}", name),
            XeditError::Io(e) => write!(f, "I/O error: {}", e),
            XeditError::InvalidCommand(msg) => write!(f, "{}", msg),
            XeditError::TargetNotFound(msg) => write!(f, "{}", msg),
            XeditError::InvalidTarget(msg) => write!(f, "Invalid target: {}", msg),
            XeditError::PrefixError(msg) => write!(f, "Prefix error: {}", msg),
            XeditError::FileModified => {
                write!(f, "File has been modified; use QQUIT to quit anyway")
            }
            XeditError::ReadOnly => write!(f, "File is read-only"),
            XeditError::NoFile => write!(f, "No file in ring"),
        }
    }
}

impl std::error::Error for XeditError {}

impl From<io::Error> for XeditError {
    fn from(e: io::Error) -> Self {
        XeditError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, XeditError>;
