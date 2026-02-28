use crate::error::{PipelineError, Result};

/// A pattern extracted from a delimited string argument (e.g., `/hello/`).
#[derive(Debug)]
pub(crate) struct DelimitedPattern {
    pub pattern: String,
}

/// Parse a CMS-style delimited string argument.
///
/// The first character of `args` is the delimiter; the pattern is the text
/// between the first and second occurrence of that delimiter.
/// Content after the closing delimiter is ignored (future: column ranges).
pub(crate) fn parse_delimited_pattern(args: &str, stage_name: &str) -> Result<DelimitedPattern> {
    if args.is_empty() {
        return Err(PipelineError::InvalidArgument(format!(
            "{stage_name} requires a delimited string argument"
        )));
    }

    let delim = args.chars().next().expect("args non-empty: checked above");
    if delim.is_whitespace() {
        return Err(PipelineError::InvalidArgument(format!(
            "{stage_name}: delimiter must not be whitespace"
        )));
    }
    let rest = &args[delim.len_utf8()..];

    match rest.find(delim) {
        Some(pos) => {
            let trailing = &rest[pos + delim.len_utf8()..];
            if !trailing.trim().is_empty() {
                return Err(PipelineError::InvalidArgument(format!(
                    "{stage_name}: unsupported options after closing delimiter"
                )));
            }
            Ok(DelimitedPattern {
                pattern: rest[..pos].to_string(),
            })
        }
        None => Err(PipelineError::InvalidArgument(format!(
            "missing closing delimiter '{delim}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_delimited_pattern() {
        let p = parse_delimited_pattern("/hello/", "locate").unwrap();
        assert_eq!(p.pattern, "hello");
    }

    #[test]
    fn empty_pattern() {
        let p = parse_delimited_pattern("//", "locate").unwrap();
        assert_eq!(p.pattern, "");
    }

    #[test]
    fn alt_delimiter() {
        let p = parse_delimited_pattern("!world!", "locate").unwrap();
        assert_eq!(p.pattern, "world");
    }

    #[test]
    fn trailing_non_whitespace_after_delimiter_rejected() {
        // /foo/bar/ → pattern "foo", trailing "bar/" is non-whitespace → error
        let err = parse_delimited_pattern("/foo/bar/", "locate").unwrap_err();
        assert_eq!(err.rc(), 24);
        assert!(err.to_string().contains("unsupported options"));
    }

    #[test]
    fn trailing_column_range_rejected() {
        // Column ranges are not yet supported — reject rather than silently ignore
        let err = parse_delimited_pattern("/abc/ 10 20", "locate").unwrap_err();
        assert_eq!(err.rc(), 24);
        assert!(err.to_string().contains("unsupported options"));
    }

    #[test]
    fn trailing_whitespace_after_delimiter_ok() {
        let p = parse_delimited_pattern("/abc/   ", "locate").unwrap();
        assert_eq!(p.pattern, "abc");
    }

    #[test]
    fn missing_closing_delimiter() {
        let err = parse_delimited_pattern("/hello", "locate").unwrap_err();
        assert_eq!(err.rc(), 24);
        assert!(err.to_string().contains("missing closing delimiter"));
    }

    #[test]
    fn whitespace_delimiter_rejected() {
        // Defence-in-depth: the pipeline parser trims args before they reach
        // here, so this path is only reachable via direct constructor calls.
        let err = parse_delimited_pattern("  /abc/", "locate").unwrap_err();
        assert_eq!(err.rc(), 24);
        assert!(err.to_string().contains("delimiter must not be whitespace"));
    }

    #[test]
    fn empty_args() {
        let err = parse_delimited_pattern("", "locate").unwrap_err();
        assert_eq!(err.rc(), 24);
        assert!(err.to_string().contains("requires a delimited string"));
    }
}
