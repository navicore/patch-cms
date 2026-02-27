use crate::error::Result;
use crate::stage::{OutputRecord, Stage};

/// Source stage that emits a constant text record.
#[derive(Debug)]
pub struct Literal {
    text: Option<String>,
}

impl Literal {
    pub fn new(text: String) -> Self {
        Self { text: Some(text) }
    }
}

impl Stage for Literal {
    fn name(&self) -> &str {
        "literal"
    }

    fn initialize(&mut self) -> Result<Vec<OutputRecord>> {
        Ok(self
            .text
            .take()
            .map(|t| vec![OutputRecord::primary(t)])
            .unwrap_or_default())
    }

    fn process(&mut self, _record: &str) -> Result<Vec<OutputRecord>> {
        // Source stage discards input
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_emits_on_init() {
        let mut s = Literal::new("hello world".to_string());
        let out = s.initialize().unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].data, "hello world");
    }

    #[test]
    fn literal_emits_once() {
        let mut s = Literal::new("hello".to_string());
        let first = s.initialize().unwrap();
        assert_eq!(first.len(), 1);
        let second = s.initialize().unwrap();
        assert!(second.is_empty());
    }

    #[test]
    fn literal_ignores_process() {
        let mut s = Literal::new("hello".to_string());
        let out = s.process("input").unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn literal_empty_args() {
        let mut s = Literal::new(String::new());
        let out = s.initialize().unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].data, "");
    }

    #[test]
    fn literal_preserves_whitespace() {
        let mut s = Literal::new("  hello  world  ".to_string());
        let out = s.initialize().unwrap();
        assert_eq!(out[0].data, "  hello  world  ");
    }
}
