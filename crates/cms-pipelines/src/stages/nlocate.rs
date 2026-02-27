use crate::error::Result;
use crate::stage::{OutputRecord, Stage};
use crate::stages::locate_common::parse_delimited_pattern;

/// Filter stage — inverse of `locate`. Non-matching records go to primary.
#[derive(Debug)]
pub struct Nlocate {
    pattern: String,
}

impl Nlocate {
    pub fn new(args: &str) -> Result<Self> {
        let parsed = parse_delimited_pattern(args, "nlocate")?;
        Ok(Self {
            pattern: parsed.pattern,
        })
    }
}

impl Stage for Nlocate {
    fn name(&self) -> &str {
        "nlocate"
    }

    fn process(&mut self, record: &str) -> Result<Vec<OutputRecord>> {
        if self.pattern.is_empty() || record.contains(&self.pattern) {
            Ok(vec![OutputRecord::secondary(record.to_string())])
        } else {
            Ok(vec![OutputRecord::primary(record.to_string())])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::Stream;

    #[test]
    fn match_goes_to_secondary() {
        let mut s = Nlocate::new("/hello/").unwrap();
        let out = s.process("hello world").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].stream, Stream::Secondary);
    }

    #[test]
    fn no_match_goes_to_primary() {
        let mut s = Nlocate::new("/hello/").unwrap();
        let out = s.process("goodbye world").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].stream, Stream::Primary);
        assert_eq!(out[0].data, "goodbye world");
    }

    #[test]
    fn empty_pattern_matches_all_to_secondary() {
        let mut s = Nlocate::new("//").unwrap();
        let out = s.process("anything").unwrap();
        assert_eq!(out[0].stream, Stream::Secondary);
    }

    #[test]
    fn case_sensitive() {
        let mut s = Nlocate::new("/Hello/").unwrap();
        let out = s.process("hello").unwrap();
        // "hello" doesn't match "Hello", so it goes to primary
        assert_eq!(out[0].stream, Stream::Primary);
    }

    #[test]
    fn name() {
        let s = Nlocate::new("/x/").unwrap();
        assert_eq!(s.name(), "nlocate");
    }
}
