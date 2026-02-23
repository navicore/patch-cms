use std::fmt;
use std::io;

/// Errors from the spool subsystem.
#[derive(Debug)]
pub enum SpoolError {
    /// Reader queue is empty (RC=2)
    ReaderEmpty,
    /// Invalid parameter or option (RC=24)
    InvalidParameter(String),
    /// File not found in spool queue (RC=28)
    FileNotFound(u64),
    /// Unknown spool command (RC=24)
    UnknownCommand(String),
    /// I/O error from the backend
    Io(io::Error),
}

impl fmt::Display for SpoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpoolError::ReaderEmpty => write!(f, "DMSSPR002E No files in your reader"),
            SpoolError::InvalidParameter(s) => {
                write!(f, "DMSSPR024E Invalid parameter - {}", s)
            }
            SpoolError::FileNotFound(id) => {
                write!(f, "DMSSPR028E File {} not found", id)
            }
            SpoolError::UnknownCommand(s) => {
                write!(f, "DMSSPR024E Unknown spool command - {}", s)
            }
            SpoolError::Io(e) => write!(f, "DMSSPR100E I/O error - {}", e),
        }
    }
}

impl std::error::Error for SpoolError {}

impl From<io::Error> for SpoolError {
    fn from(e: io::Error) -> Self {
        SpoolError::Io(e)
    }
}

impl SpoolError {
    /// Return the CMS-style return code for this error.
    pub fn rc(&self) -> i32 {
        match self {
            SpoolError::ReaderEmpty => 2,
            SpoolError::InvalidParameter(_) => 24,
            SpoolError::FileNotFound(_) => 28,
            SpoolError::UnknownCommand(_) => 24,
            SpoolError::Io(_) => 100,
        }
    }
}

pub type Result<T> = std::result::Result<T, SpoolError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_reader_empty() {
        let e = SpoolError::ReaderEmpty;
        assert!(e.to_string().contains("No files in your reader"));
        assert_eq!(e.rc(), 2);
    }

    #[test]
    fn error_display_invalid_parameter() {
        let e = SpoolError::InvalidParameter("FOOBAR".to_string());
        assert!(e.to_string().contains("FOOBAR"));
        assert_eq!(e.rc(), 24);
    }

    #[test]
    fn error_display_file_not_found() {
        let e = SpoolError::FileNotFound(42);
        assert!(e.to_string().contains("42"));
        assert_eq!(e.rc(), 28);
    }

    #[test]
    fn error_from_io() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "gone");
        let e = SpoolError::from(io_err);
        assert_eq!(e.rc(), 100);
    }

    #[test]
    fn error_display_unknown_command() {
        let e = SpoolError::UnknownCommand("XYZZY".to_string());
        let msg = e.to_string();
        assert!(msg.contains("DMSSPR024E"));
        assert!(msg.contains("XYZZY"));
        assert_eq!(e.rc(), 24);
    }

    #[test]
    fn error_display_io() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        let e = SpoolError::Io(io_err);
        let msg = e.to_string();
        assert!(msg.contains("DMSSPR100E"));
        assert!(msg.contains("denied"));
    }

    #[test]
    fn error_messages_have_ibm_prefix() {
        // All error messages should start with DMSSPRnnnE or DMSSPRnnnI
        let errors = vec![
            SpoolError::ReaderEmpty,
            SpoolError::InvalidParameter("X".to_string()),
            SpoolError::FileNotFound(1),
            SpoolError::UnknownCommand("X".to_string()),
            SpoolError::Io(io::Error::new(io::ErrorKind::Other, "x")),
        ];
        for e in &errors {
            let msg = e.to_string();
            assert!(
                msg.starts_with("DMSSPR"),
                "Error '{}' missing DMSSPR prefix",
                msg
            );
        }
    }
}
