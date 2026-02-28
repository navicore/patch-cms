use crate::error::{IucvError, Result};
use std::fmt;

/// A validated CMS machine identifier (userid).
///
/// 1-8 characters, uppercase alphanumeric plus `@`, `#`, `$`.
/// Input is automatically uppercased during construction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MachineId(String);

impl MachineId {
    /// Create a new `MachineId`, uppercasing the input and validating format.
    pub fn new(id: &str) -> Result<Self> {
        if !id.is_ascii() {
            return Err(IucvError::InvalidMachineId(id.to_string()));
        }
        let upper = id.to_uppercase();
        if upper.is_empty() || upper.len() > 8 {
            return Err(IucvError::InvalidMachineId(upper));
        }
        if !upper
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b"@#$".contains(&b))
        {
            return Err(IucvError::InvalidMachineId(upper));
        }
        Ok(MachineId(upper))
    }

    /// Return the machine id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MachineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_simple() {
        let id = MachineId::new("OPERATOR").unwrap();
        assert_eq!(id.as_str(), "OPERATOR");
    }

    #[test]
    fn valid_short() {
        let id = MachineId::new("A").unwrap();
        assert_eq!(id.as_str(), "A");
    }

    #[test]
    fn valid_max_length() {
        let id = MachineId::new("ABCDEFGH").unwrap();
        assert_eq!(id.as_str(), "ABCDEFGH");
    }

    #[test]
    fn valid_special_chars() {
        let id = MachineId::new("US@R#1$").unwrap();
        assert_eq!(id.as_str(), "US@R#1$");
    }

    #[test]
    fn lowercase_uppercased() {
        let id = MachineId::new("operator").unwrap();
        assert_eq!(id.as_str(), "OPERATOR");
    }

    #[test]
    fn reject_empty() {
        assert!(MachineId::new("").is_err());
    }

    #[test]
    fn reject_too_long() {
        assert!(MachineId::new("TOOLONGID").is_err());
    }

    #[test]
    fn reject_illegal_chars() {
        assert!(MachineId::new("BAD!").is_err());
        assert!(MachineId::new("SP ACE").is_err());
    }

    #[test]
    fn reject_non_ascii() {
        // "ß" uppercases to "SS" in Unicode — must be rejected before uppercasing
        assert!(MachineId::new("ß").is_err());
        assert!(MachineId::new("café").is_err());
    }
}
