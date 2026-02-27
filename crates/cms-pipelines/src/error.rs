use std::fmt;

/// Errors from the CMS Pipelines subsystem.
#[derive(Debug)]
pub enum PipelineError {
    /// Empty pipeline (RC=24)
    EmptyPipeline,
    /// Empty stage segment (RC=24)
    EmptyStage,
    /// Trailing pipe separator (RC=24)
    TrailingPipe,
    /// Unknown stage name (RC=28)
    UnknownStage(String),
    /// Invalid stage arguments (RC=24)
    InvalidArgument(String),
    /// Stage execution error (RC=32)
    StageError(String),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineError::EmptyPipeline => {
                write!(f, "DMSPIP024E Empty pipeline specification")
            }
            PipelineError::EmptyStage => {
                write!(f, "DMSPIP024E Empty stage in pipeline")
            }
            PipelineError::TrailingPipe => {
                write!(f, "DMSPIP024E Trailing pipe separator")
            }
            PipelineError::UnknownStage(name) => {
                write!(f, "DMSPIP028E Unknown stage - {}", name)
            }
            PipelineError::InvalidArgument(msg) => {
                write!(f, "DMSPIP024E Invalid argument - {}", msg)
            }
            PipelineError::StageError(msg) => {
                write!(f, "DMSPIP032E Stage error - {}", msg)
            }
        }
    }
}

impl std::error::Error for PipelineError {}

impl PipelineError {
    /// Return the CMS-style return code for this error.
    pub fn rc(&self) -> i32 {
        match self {
            PipelineError::EmptyPipeline => 24,
            PipelineError::EmptyStage => 24,
            PipelineError::TrailingPipe => 24,
            PipelineError::UnknownStage(_) => 28,
            PipelineError::InvalidArgument(_) => 24,
            PipelineError::StageError(_) => 32,
        }
    }
}

pub type Result<T> = std::result::Result<T, PipelineError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_empty_pipeline() {
        let e = PipelineError::EmptyPipeline;
        assert!(e.to_string().contains("Empty pipeline"));
        assert_eq!(e.rc(), 24);
    }

    #[test]
    fn error_display_empty_stage() {
        let e = PipelineError::EmptyStage;
        assert!(e.to_string().contains("Empty stage"));
        assert_eq!(e.rc(), 24);
    }

    #[test]
    fn error_display_trailing_pipe() {
        let e = PipelineError::TrailingPipe;
        assert!(e.to_string().contains("Trailing pipe"));
        assert_eq!(e.rc(), 24);
    }

    #[test]
    fn error_display_unknown_stage() {
        let e = PipelineError::UnknownStage("foobar".to_string());
        assert!(e.to_string().contains("foobar"));
        assert_eq!(e.rc(), 28);
    }

    #[test]
    fn error_display_invalid_argument() {
        let e = PipelineError::InvalidArgument("bad value".to_string());
        assert!(e.to_string().contains("bad value"));
        assert_eq!(e.rc(), 24);
    }

    #[test]
    fn error_display_stage_error() {
        let e = PipelineError::StageError("something broke".to_string());
        assert!(e.to_string().contains("something broke"));
        assert_eq!(e.rc(), 32);
    }

    #[test]
    fn error_messages_have_ibm_prefix() {
        let errors: Vec<PipelineError> = vec![
            PipelineError::EmptyPipeline,
            PipelineError::EmptyStage,
            PipelineError::TrailingPipe,
            PipelineError::UnknownStage("X".to_string()),
            PipelineError::InvalidArgument("X".to_string()),
            PipelineError::StageError("X".to_string()),
        ];
        for e in &errors {
            let msg = e.to_string();
            assert!(
                msg.starts_with("DMSPIP"),
                "Error '{}' missing DMSPIP prefix",
                msg
            );
        }
    }
}
