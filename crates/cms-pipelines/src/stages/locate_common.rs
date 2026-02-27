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

    let delim = args.chars().next().unwrap();
    let rest = &args[delim.len_utf8()..];

    match rest.find(delim) {
        Some(pos) => Ok(DelimitedPattern {
            pattern: rest[..pos].to_string(),
        }),
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
    fn missing_closing_delimiter() {
        let err = parse_delimited_pattern("/hello", "locate").unwrap_err();
        assert_eq!(err.rc(), 24);
        assert!(err.to_string().contains("missing closing delimiter"));
    }

    #[test]
    fn empty_args() {
        let err = parse_delimited_pattern("", "locate").unwrap_err();
        assert_eq!(err.rc(), 24);
        assert!(err.to_string().contains("requires a delimited string"));
    }
}
