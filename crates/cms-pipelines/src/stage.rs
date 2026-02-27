use crate::error::Result;

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
    fn initialize(&mut self) -> Result<Vec<OutputRecord>> {
        Ok(Vec::new())
    }

    /// Called once per input record. Default is pass-through.
    fn process(&mut self, record: &str) -> Result<Vec<OutputRecord>> {
        Ok(vec![OutputRecord::primary(record.to_string())])
    }

    /// Called once after all input records. Buffering stages flush here.
    fn finish(&mut self) -> Result<Vec<OutputRecord>> {
        Ok(Vec::new())
    }

    /// Sink stages return accumulated data here.
    fn collected_output(&self) -> &[String] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::PipelineError;

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
        assert!(s.initialize().unwrap().is_empty());
        let out = s.process("test").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].stream, Stream::Primary);
        assert_eq!(out[0].data, "test");
        assert!(s.finish().unwrap().is_empty());
        assert!(s.collected_output().is_empty());
    }

    #[test]
    fn stage_can_return_error() {
        #[derive(Debug)]
        struct FailStage;
        impl Stage for FailStage {
            fn name(&self) -> &str {
                "fail"
            }
            fn process(&mut self, _record: &str) -> Result<Vec<OutputRecord>> {
                Err(PipelineError::StageError("test failure".to_string()))
            }
        }

        let mut s = FailStage;
        let err = s.process("input").unwrap_err();
        assert_eq!(err.rc(), 32);
    }
}
