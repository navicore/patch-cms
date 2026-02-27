use crate::error::{PipelineError, Result};

/// A single stage specification parsed from a pipeline string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSpec {
    /// Lowercased stage name, used for matching in `create_stage()`.
    pub name: String,
    /// Original-case stage name, used in error messages.
    pub raw_name: String,
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

    // Check for trailing pipe (only when there is content before the final |;
    // bare "|" falls through to split, producing EmptyStage for the empty segments)
    if body.ends_with('|') && body.len() > 1 {
        return Err(PipelineError::TrailingPipe);
    }

    let segments: Vec<&str> = body.split('|').collect();

    const MAX_STAGES: usize = 255;
    if segments.len() > MAX_STAGES {
        return Err(PipelineError::InvalidArgument(format!(
            "pipeline exceeds {} stages",
            MAX_STAGES
        )));
    }

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
            raw_name: name.to_string(),
            args: args.to_string(),
        });
    }

    Ok(PipelineSpec { stages })
}

/// Strip a leading `PIPE` keyword (case-insensitive) if present.
fn strip_pipe_prefix(input: &str) -> &str {
    let b = input.as_bytes();
    if b.len() >= 5 && b[..4].eq_ignore_ascii_case(b"PIPE") && (b[4] == b' ' || b[4] == b'\t') {
        &input[5..]
    } else if b.len() == 4 && b.eq_ignore_ascii_case(b"PIPE") {
        ""
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

    #[test]
    fn parse_pipe_in_args_splits_on_pipe() {
        // Known limitation: pipe characters in stage arguments are treated as
        // stage separators. Future quoted-string support (`literal 'a|b'`)
        // will fix this. For now, document the current behavior.
        let spec = parse_pipeline("literal a|b | console").unwrap();
        // Parsed as 3 stages: "literal a", "b", "console" — not 2
        assert_eq!(spec.stages.len(), 3);
        assert_eq!(spec.stages[0].name, "literal");
        assert_eq!(spec.stages[0].args, "a");
        assert_eq!(spec.stages[1].name, "b");
    }

    #[test]
    fn parse_too_many_stages_rejected() {
        let stages: Vec<&str> = (0..256).map(|_| "console").collect();
        let input = stages.join(" | ");
        let err = parse_pipeline(&input).unwrap_err();
        assert!(matches!(err, PipelineError::InvalidArgument(_)));
        assert_eq!(err.rc(), 24);
    }

    #[test]
    fn parse_max_stages_allowed() {
        let stages: Vec<&str> = (0..255).map(|_| "console").collect();
        let input = stages.join(" | ");
        let spec = parse_pipeline(&input).unwrap();
        assert_eq!(spec.stages.len(), 255);
    }

    #[test]
    fn parse_bare_pipe_is_empty_stage() {
        // "|" has empty segments on both sides — EmptyStage, not TrailingPipe
        let err = parse_pipeline("|").unwrap_err();
        assert!(matches!(err, PipelineError::EmptyStage));
    }

    #[test]
    fn parse_raw_name_preserves_case() {
        let spec = parse_pipeline("LITERAL Hello | CONSOLE").unwrap();
        assert_eq!(spec.stages[0].name, "literal");
        assert_eq!(spec.stages[0].raw_name, "LITERAL");
        assert_eq!(spec.stages[1].name, "console");
        assert_eq!(spec.stages[1].raw_name, "CONSOLE");
    }
}
