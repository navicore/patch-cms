use crate::error::{PipelineError, Result};

/// A single stage specification parsed from a pipeline string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSpec {
    pub name: String,
    pub args: String,
}

/// A parsed pipeline consisting of one or more stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineSpec {
    pub stages: Vec<StageSpec>,
}

/// Parse a PIPE command string into a `PipelineSpec`.
///
/// Syntax: `[PIPE] stage1 [| stage2 [| ...]]`
///
/// - Splits on `|`
/// - First token of each segment is the stage name (lowercased)
/// - Remaining tokens are the args string
/// - Optional leading `PIPE` prefix is stripped (case-insensitive)
pub fn parse_pipeline(input: &str) -> Result<PipelineSpec> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(PipelineError::EmptyPipeline);
    }

    // Strip optional leading PIPE prefix
    let body = strip_pipe_prefix(trimmed);
    let body = body.trim();
    if body.is_empty() {
        return Err(PipelineError::EmptyPipeline);
    }

    // Check for trailing pipe
    if body.ends_with('|') {
        return Err(PipelineError::TrailingPipe);
    }

    let segments: Vec<&str> = body.split('|').collect();
    let mut stages = Vec::with_capacity(segments.len());

    for seg in &segments {
        let seg = seg.trim();
        if seg.is_empty() {
            return Err(PipelineError::EmptyStage);
        }

        let (name, args) = match seg.find(char::is_whitespace) {
            Some(pos) => {
                let name = &seg[..pos];
                let args = seg[pos..].trim();
                (name, args)
            }
            None => (seg, ""),
        };

        stages.push(StageSpec {
            name: name.to_ascii_lowercase(),
            args: args.to_string(),
        });
    }

    Ok(PipelineSpec { stages })
}

/// Strip a leading `PIPE` keyword (case-insensitive) if present.
fn strip_pipe_prefix(input: &str) -> &str {
    let upper = input.to_ascii_uppercase();
    if upper.starts_with("PIPE ") || upper.starts_with("PIPE\t") {
        &input[5..]
    } else if upper == "PIPE" {
        &input[4..]
    } else {
        input
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_stage() {
        let spec = parse_pipeline("literal hello").unwrap();
        assert_eq!(spec.stages.len(), 1);
        assert_eq!(spec.stages[0].name, "literal");
        assert_eq!(spec.stages[0].args, "hello");
    }

    #[test]
    fn parse_multi_stage() {
        let spec = parse_pipeline("literal hello | console").unwrap();
        assert_eq!(spec.stages.len(), 2);
        assert_eq!(spec.stages[0].name, "literal");
        assert_eq!(spec.stages[0].args, "hello");
        assert_eq!(spec.stages[1].name, "console");
        assert_eq!(spec.stages[1].args, "");
    }

    #[test]
    fn parse_case_normalization() {
        let spec = parse_pipeline("LITERAL Hello World").unwrap();
        assert_eq!(spec.stages[0].name, "literal");
        assert_eq!(spec.stages[0].args, "Hello World");
    }

    #[test]
    fn parse_pipe_prefix_stripped() {
        let spec = parse_pipeline("PIPE literal hello | console").unwrap();
        assert_eq!(spec.stages.len(), 2);
        assert_eq!(spec.stages[0].name, "literal");
    }

    #[test]
    fn parse_pipe_prefix_case_insensitive() {
        let spec = parse_pipeline("pipe literal test").unwrap();
        assert_eq!(spec.stages[0].name, "literal");
        assert_eq!(spec.stages[0].args, "test");
    }

    #[test]
    fn parse_empty_input_error() {
        let err = parse_pipeline("").unwrap_err();
        assert_eq!(err.rc(), 24);
        assert!(matches!(err, PipelineError::EmptyPipeline));
    }

    #[test]
    fn parse_whitespace_only_error() {
        let err = parse_pipeline("   ").unwrap_err();
        assert!(matches!(err, PipelineError::EmptyPipeline));
    }

    #[test]
    fn parse_trailing_pipe_error() {
        let err = parse_pipeline("literal hello |").unwrap_err();
        assert!(matches!(err, PipelineError::TrailingPipe));
    }

    #[test]
    fn parse_empty_segment_error() {
        let err = parse_pipeline("literal hello || console").unwrap_err();
        assert!(matches!(err, PipelineError::EmptyStage));
    }

    #[test]
    fn parse_pipe_only_is_empty() {
        let err = parse_pipeline("PIPE").unwrap_err();
        assert!(matches!(err, PipelineError::EmptyPipeline));
    }
}
