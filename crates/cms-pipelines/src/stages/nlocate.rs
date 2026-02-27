use crate::error::Result;
use crate::stage::{OutputRecord, Stage};
use crate::stages::locate_common::parse_delimited_pattern;

/// Filter stage — inverse of `locate`. Non-matching records go to primary.
#[derive(Debug)]
pub struct Nlocate {
    pattern: String,
    processed_any: bool,
    any_primary: bool,
}

impl Nlocate {
    pub fn new(args: &str) -> Result<Self> {
        let parsed = parse_delimited_pattern(args, "nlocate")?;
        Ok(Self {
            pattern: parsed.pattern,
            processed_any: false,
            any_primary: false,
        })
    }
}

impl Stage for Nlocate {
    fn name(&self) -> &str {
        "nlocate"
    }

    fn process(&mut self, record: &str) -> Result<Vec<OutputRecord>> {
        self.processed_any = true;
        if record.contains(&self.pattern) {
            Ok(vec![OutputRecord::secondary(record.to_string())])
        } else {
            self.any_primary = true;
            Ok(vec![OutputRecord::primary(record.to_string())])
        }
    }

    fn stage_rc(&self) -> i32 {
        if self.processed_any && !self.any_primary {
            4
        } else {
            0
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
    fn empty_record_no_match() {
        let mut s = Nlocate::new("/abc/").unwrap();
        let out = s.process("").unwrap();
        assert_eq!(out[0].stream, Stream::Primary);
    }

    #[test]
    fn rc_four_when_all_match() {
        let mut s = Nlocate::new("/hello/").unwrap();
        s.process("hello world").unwrap();
        assert_eq!(s.stage_rc(), 4);
    }

    #[test]
    fn rc_zero_when_no_match() {
        let mut s = Nlocate::new("/hello/").unwrap();
        s.process("goodbye").unwrap();
        assert_eq!(s.stage_rc(), 0);
    }

    #[test]
    fn name() {
        let s = Nlocate::new("/x/").unwrap();
        assert_eq!(s.name(), "nlocate");
    }
}
