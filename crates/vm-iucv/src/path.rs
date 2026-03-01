use crate::error::{IucvError, Result};
use std::fmt;

/// Maximum IUCV buffer size (VM/CMS IUCV data limit).
const IUCV_MAX_BUFFER_LEN: usize = 65535;

/// Unique identifier for an IUCV path between two machines.
///
/// Generated internally by the supervisor via an `AtomicU32` counter.
/// Not user-constructible — use [`Supervisor::connect`] to obtain paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathId(pub(crate) u32);

impl PathId {
    /// Return the raw numeric identifier.
    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for PathId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PATH({})", self.0)
    }
}

/// Buffer for IUCV SEND/RECEIVE data.
///
/// Enforces the 65535-byte VM/CMS IUCV data limit at construction time.
#[derive(Debug, Clone)]
pub struct IucvBuffer {
    data: Vec<u8>,
}

impl IucvBuffer {
    /// Create a new IUCV buffer, validating the 65535-byte limit.
    pub fn new(data: Vec<u8>) -> Result<Self> {
        if data.len() > IUCV_MAX_BUFFER_LEN {
            return Err(IucvError::InvalidParameter(format!(
                "IUCV buffer exceeds {} bytes",
                IUCV_MAX_BUFFER_LEN
            )));
        }
        Ok(IucvBuffer { data })
    }

    /// Return the buffer contents as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Return the buffer length in bytes.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Return true if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_id_display() {
        let id = PathId(42);
        assert_eq!(id.to_string(), "PATH(42)");
        assert_eq!(id.as_u32(), 42);
    }

    #[test]
    fn path_id_eq_and_copy() {
        let a = PathId(1);
        let b = a; // Copy
        assert_eq!(a, b);
        assert_ne!(a, PathId(2));
    }

    #[test]
    fn iucv_buffer_valid() {
        let buf = IucvBuffer::new(vec![1, 2, 3]).unwrap();
        assert_eq!(buf.as_bytes(), &[1, 2, 3]);
        assert_eq!(buf.len(), 3);
        assert!(!buf.is_empty());
    }

    #[test]
    fn iucv_buffer_reject_too_large() {
        let data = vec![0u8; 65536];
        let err = IucvBuffer::new(data).unwrap_err();
        assert_eq!(err.rc(), 24);
    }

    #[test]
    fn iucv_buffer_empty() {
        let buf = IucvBuffer::new(vec![]).unwrap();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }
}
