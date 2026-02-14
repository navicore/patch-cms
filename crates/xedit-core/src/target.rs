/// A target specifies a location in XEDIT's addressing system.
///
/// Targets are one of XEDIT's most distinctive features, allowing
/// precise addressing by line number, relative offset, or string search.
#[derive(Debug, Clone)]
pub enum Target {
    /// Absolute line number `:n`
    Absolute(usize),
    /// Relative line offset `+n` or `-n`
    Relative(i64),
    /// String search forward `/string/`
    StringForward(String),
    /// String search backward `-/string/`
    StringBackward(String),
    /// All remaining lines `*`
    Star,
}

impl Target {
    /// Parse a target specification from a string
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        if input.is_empty() {
            return Err("Empty target".to_string());
        }

        if input == "*" {
            return Ok(Target::Star);
        }

        // Absolute line number :n
        if let Some(rest) = input.strip_prefix(':') {
            return rest
                .parse::<usize>()
                .map(Target::Absolute)
                .map_err(|_| format!("Invalid line number: {}", rest));
        }

        // Relative positive +n
        if let Some(rest) = input.strip_prefix('+') {
            if rest.is_empty() {
                return Ok(Target::Relative(1));
            }
            return rest
                .parse::<i64>()
                .map(Target::Relative)
                .map_err(|_| format!("Invalid offset: +{}", rest));
        }

        // Negative: relative number or backward search
        if let Some(rest) = input.strip_prefix('-') {
            if rest.starts_with('/') {
                let search_str = extract_delimited(rest, '/')?;
                return Ok(Target::StringBackward(search_str));
            }
            return rest
                .parse::<i64>()
                .map(|n| Target::Relative(-n))
                .map_err(|_| format!("Invalid offset: {}", input));
        }

        // Forward string search /string/
        if input.starts_with('/') {
            let search_str = extract_delimited(input, '/')?;
            return Ok(Target::StringForward(search_str));
        }

        // Plain number = relative forward
        if let Ok(n) = input.parse::<i64>() {
            return Ok(Target::Relative(n));
        }

        Err(format!("Invalid target: {}", input))
    }

    /// Resolve this target to an absolute line number.
    ///
    /// Returns None if the target cannot be resolved (e.g., string not found).
    pub fn resolve(
        &self,
        current_line: usize,
        buffer_len: usize,
        case_respect: bool,
        line_text_fn: &dyn Fn(usize) -> Option<String>,
    ) -> Option<usize> {
        match self {
            Target::Absolute(n) => {
                if *n <= buffer_len {
                    Some(*n)
                } else {
                    None
                }
            }
            Target::Relative(offset) => {
                let new_line = current_line as i64 + offset;
                if new_line >= 0 && new_line <= buffer_len as i64 {
                    Some(new_line as usize)
                } else {
                    None
                }
            }
            Target::StringForward(s) => {
                let needle = if case_respect {
                    s.clone()
                } else {
                    s.to_uppercase()
                };
                for i in (current_line + 1)..=buffer_len {
                    if let Some(text) = line_text_fn(i) {
                        let haystack = if case_respect {
                            text
                        } else {
                            text.to_uppercase()
                        };
                        if haystack.contains(&needle) {
                            return Some(i);
                        }
                    }
                }
                None
            }
            Target::StringBackward(s) => {
                if current_line == 0 {
                    return None;
                }
                let needle = if case_respect {
                    s.clone()
                } else {
                    s.to_uppercase()
                };
                for i in (1..current_line).rev() {
                    if let Some(text) = line_text_fn(i) {
                        let haystack = if case_respect {
                            text
                        } else {
                            text.to_uppercase()
                        };
                        if haystack.contains(&needle) {
                            return Some(i);
                        }
                    }
                }
                None
            }
            Target::Star => Some(buffer_len),
        }
    }
}

fn extract_delimited(input: &str, delim: char) -> Result<String, String> {
    let rest = &input[delim.len_utf8()..];
    if let Some(end) = rest.find(delim) {
        Ok(rest[..end].to_string())
    } else {
        Ok(rest.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_absolute() {
        match Target::parse(":5").unwrap() {
            Target::Absolute(5) => {}
            other => panic!("Expected Absolute(5), got {:?}", other),
        }
    }

    #[test]
    fn parse_relative_positive() {
        match Target::parse("+3").unwrap() {
            Target::Relative(3) => {}
            other => panic!("Expected Relative(3), got {:?}", other),
        }
    }

    #[test]
    fn parse_relative_negative() {
        match Target::parse("-2").unwrap() {
            Target::Relative(-2) => {}
            other => panic!("Expected Relative(-2), got {:?}", other),
        }
    }

    #[test]
    fn parse_string_forward() {
        match Target::parse("/hello/").unwrap() {
            Target::StringForward(s) => assert_eq!(s, "hello"),
            other => panic!("Expected StringForward, got {:?}", other),
        }
    }

    #[test]
    fn parse_string_backward() {
        match Target::parse("-/hello/").unwrap() {
            Target::StringBackward(s) => assert_eq!(s, "hello"),
            other => panic!("Expected StringBackward, got {:?}", other),
        }
    }

    #[test]
    fn parse_star() {
        match Target::parse("*").unwrap() {
            Target::Star => {}
            other => panic!("Expected Star, got {:?}", other),
        }
    }

    #[test]
    fn resolve_forward_search() {
        let lines = vec!["alpha", "beta", "gamma", "delta"];
        let result = Target::StringForward("gamma".into())
            .resolve(1, 4, false, &|n| lines.get(n - 1).map(|s| s.to_string()));
        assert_eq!(result, Some(3));
    }

    #[test]
    fn resolve_backward_search() {
        let lines = vec!["alpha", "beta", "gamma", "delta"];
        let result = Target::StringBackward("alpha".into())
            .resolve(3, 4, false, &|n| lines.get(n - 1).map(|s| s.to_string()));
        assert_eq!(result, Some(1));
    }
}
