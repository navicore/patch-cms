use crate::error::Result;
use crate::stage::{OutputRecord, Stage};

/// Sink stage that collects records.
#[derive(Debug, Default)]
pub struct Console {
    records: Vec<String>,
}

impl Console {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Stage for Console {
    fn name(&self) -> &str {
        "console"
    }

    fn process(&mut self, record: &str) -> Result<Vec<OutputRecord>> {
        self.records.push(record.to_string());
        // Sink — no downstream output
        Ok(Vec::new())
    }

    fn collected_output(&self) -> &[String] {
        &self.records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_collects_records() {
        let mut s = Console::new();
        s.process("line1").unwrap();
        s.process("line2").unwrap();
        assert_eq!(s.collected_output(), &["line1", "line2"]);
    }

    #[test]
    fn console_returns_empty_downstream() {
        let mut s = Console::new();
        let out = s.process("test").unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn console_initially_empty() {
        let s = Console::new();
        assert!(s.collected_output().is_empty());
    }

    #[test]
    fn console_preserves_order() {
        let mut s = Console::new();
        s.process("c").unwrap();
        s.process("a").unwrap();
        s.process("b").unwrap();
        assert_eq!(s.collected_output(), &["c", "a", "b"]);
    }
}
