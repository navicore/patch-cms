use crate::error::Result;
use crate::stage::{OutputRecord, Stage};
use crate::stages::locate_common::parse_delimited_pattern;

/// Filter stage that routes matching records to primary, non-matching to secondary.
#[derive(Debug)]
pub struct Locate {
    pattern: String,
    any_primary: bool,
}

impl Locate {
    pub fn new(args: &str) -> Result<Self> {
        let parsed = parse_delimited_pattern(args, "locate")?;
        Ok(Self {
            pattern: parsed.pattern,
            any_primary: false,
        })
    }
}

impl Stage for Locate {
    fn name(&self) -> &str {
        "locate"
    }

    fn process(&mut self, record: &str) -> Result<Vec<OutputRecord>> {
        if record.contains(&self.pattern) {
            self.any_primary = true;
            Ok(vec![OutputRecord::primary(record.to_string())])
        } else {
            Ok(vec![OutputRecord::secondary(record.to_string())])
        }
    }

    fn stage_rc(&self) -> i32 {
        if self.any_primary {
            0
        } else {
            4
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::Stream;

    #[test]
    fn match_goes_to_primary() {
        let mut s = Locate::new("/hello/").unwrap();
        let out = s.process("hello world").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].stream, Stream::Primary);
        assert_eq!(out[0].data, "hello world");
    }

    #[test]
    fn no_match_goes_to_secondary() {
        let mut s = Locate::new("/hello/").unwrap();
        let out = s.process("goodbye world").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].stream, Stream::Secondary);
        assert_eq!(out[0].data, "goodbye world");
    }

    #[test]
    fn empty_pattern_matches_all() {
        let mut s = Locate::new("//").unwrap();
        let out = s.process("anything").unwrap();
        assert_eq!(out[0].stream, Stream::Primary);
    }

    #[test]
    fn case_sensitive() {
        let mut s = Locate::new("/Hello/").unwrap();
        let out = s.process("hello").unwrap();
        assert_eq!(out[0].stream, Stream::Secondary);
    }

    #[test]
    fn pattern_at_end_of_record() {
        let mut s = Locate::new("/end/").unwrap();
        let out = s.process("the end").unwrap();
        assert_eq!(out[0].stream, Stream::Primary);
    }

    #[test]
    fn empty_record_no_match() {
        let mut s = Locate::new("/abc/").unwrap();
        let out = s.process("").unwrap();
        assert_eq!(out[0].stream, Stream::Secondary);
    }

    #[test]
    fn rc_zero_when_match_found() {
        let mut s = Locate::new("/hello/").unwrap();
        s.process("hello world").unwrap();
        assert_eq!(s.stage_rc(), 0);
    }

    #[test]
    fn rc_four_when_no_match() {
        let mut s = Locate::new("/hello/").unwrap();
        s.process("goodbye").unwrap();
        assert_eq!(s.stage_rc(), 4);
    }

    #[test]
    fn rc_zero_when_mixed_matches() {
        let mut s = Locate::new("/hello/").unwrap();
        s.process("hello world").unwrap();
        s.process("goodbye").unwrap();
        assert_eq!(s.stage_rc(), 0);
    }

    #[test]
    fn rc_four_when_no_records_processed() {
        let s = Locate::new("/hello/").unwrap();
        assert_eq!(s.stage_rc(), 4);
    }

    #[test]
    fn name() {
        let s = Locate::new("/x/").unwrap();
        assert_eq!(s.name(), "locate");
    }
}
