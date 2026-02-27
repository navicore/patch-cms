/// Which output stream a record is routed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Primary,
    Secondary,
}

/// A record emitted by a stage.
#[derive(Debug, Clone)]
pub struct OutputRecord {
    pub stream: Stream,
    pub data: String,
}

impl OutputRecord {
    /// Create a primary-stream record.
    pub fn primary(data: String) -> Self {
        Self {
            stream: Stream::Primary,
            data,
        }
    }

    /// Create a secondary-stream record.
    pub fn secondary(data: String) -> Self {
        Self {
            stream: Stream::Secondary,
            data,
        }
    }
}

/// A pipeline stage that processes records.
pub trait Stage: std::fmt::Debug {
    /// Stage name for diagnostics.
    fn name(&self) -> &str;

    /// Called once before any input records. Source stages emit records here.
    fn initialize(&mut self) -> Vec<OutputRecord> {
        Vec::new()
    }

    /// Called once per input record. Default is pass-through.
    fn process(&mut self, record: &str) -> Vec<OutputRecord> {
        vec![OutputRecord::primary(record.to_string())]
    }

    /// Called once after all input records. Buffering stages flush here.
    fn finish(&mut self) -> Vec<OutputRecord> {
        Vec::new()
    }

    /// Sink stages return accumulated data here.
    fn collected_output(&self) -> &[String] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_record_primary_constructor() {
        let rec = OutputRecord::primary("hello".to_string());
        assert_eq!(rec.stream, Stream::Primary);
        assert_eq!(rec.data, "hello");
    }

    #[test]
    fn output_record_secondary_constructor() {
        let rec = OutputRecord::secondary("world".to_string());
        assert_eq!(rec.stream, Stream::Secondary);
        assert_eq!(rec.data, "world");
    }

    #[test]
    fn default_stage_pass_through() {
        #[derive(Debug)]
        struct PassThrough;
        impl Stage for PassThrough {
            fn name(&self) -> &str {
                "passthrough"
            }
        }

        let mut s = PassThrough;
        assert!(s.initialize().is_empty());
        let out = s.process("test");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].stream, Stream::Primary);
        assert_eq!(out[0].data, "test");
        assert!(s.finish().is_empty());
        assert!(s.collected_output().is_empty());
    }
}
