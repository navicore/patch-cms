/// A target specifies a location in XEDIT's addressing system.
///
/// Targets are one of XEDIT's most distinctive features, allowing
/// precise addressing by line number, relative offset, or string search.
/// Compound targets use `&` (AND) and `|` (OR) to combine conditions.
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
    /// Both targets must match the line
    And(Box<Target>, Box<Target>),
    /// Either target must match the line
    Or(Box<Target>, Box<Target>),
}

impl Target {
    /// Parse a target specification from a string
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        if input.is_empty() {
            return Err("Empty target".to_string());
        }
        try_parse_compound(input)
    }

    /// Check whether a line's text satisfies a string-based target condition.
    ///
    /// This is used by ALL filtering and compound target resolution.
    /// Only string-based targets (forward/backward search, And, Or) are meaningful;
    /// positional targets (Absolute, Relative, Star) always return false.
    pub fn matches_line(&self, case_respect: bool, line_text: &str) -> bool {
        match self {
            Target::StringForward(s) | Target::StringBackward(s) => {
                let (needle, haystack) = if case_respect {
                    (s.as_str().to_string(), line_text.to_string())
                } else {
                    (s.to_uppercase(), line_text.to_uppercase())
                };
                haystack.contains(&needle)
            }
            Target::And(a, b) => {
                a.matches_line(case_respect, line_text) && b.matches_line(case_respect, line_text)
            }
            Target::Or(a, b) => {
                a.matches_line(case_respect, line_text) || b.matches_line(case_respect, line_text)
            }
            // Positional targets don't match by content
            _ => false,
        }
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
            Target::And(_, _) | Target::Or(_, _) => {
                // Search forward for first line satisfying compound condition
                for i in (current_line + 1)..=buffer_len {
                    if let Some(text) = line_text_fn(i) {
                        if self.matches_line(case_respect, &text) {
                            return Some(i);
                        }
                    }
                }
                None
            }
        }
    }
}

/// Try to parse a compound target (containing & or |), falling back to simple.
/// OR (`|`) has lower precedence — checked first at top level.
/// AND (`&`) has higher precedence — checked in sub-parser.
fn try_parse_compound(input: &str) -> Result<Target, String> {
    // OR has lower precedence: check first so it binds last
    if let Some(pos) = find_operator(input, '|') {
        let left = try_parse_and(input[..pos].trim())?;
        let right = try_parse_compound(input[pos + 1..].trim())?;
        return Ok(Target::Or(Box::new(left), Box::new(right)));
    }
    try_parse_and(input)
}

/// Parse AND-level compound targets (higher precedence than OR).
fn try_parse_and(input: &str) -> Result<Target, String> {
    if let Some(pos) = find_operator(input, '&') {
        let left = parse_simple(input[..pos].trim())?;
        let right = try_parse_and(input[pos + 1..].trim())?;
        return Ok(Target::And(Box::new(left), Box::new(right)));
    }
    parse_simple(input)
}

/// Find position of operator outside of /delimiters/
fn find_operator(input: &str, op: char) -> Option<usize> {
    let mut in_delim = false;
    for (i, c) in input.char_indices() {
        if c == '/' {
            in_delim = !in_delim;
        } else if c == op && !in_delim {
            return Some(i);
        }
    }
    None
}

/// Parse a simple (non-compound) target
fn parse_simple(input: &str) -> Result<Target, String> {
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

    #[test]
    fn parse_and_target() {
        match Target::parse("/hello/&/world/").unwrap() {
            Target::And(a, b) => {
                match *a {
                    Target::StringForward(ref s) => assert_eq!(s, "hello"),
                    ref other => panic!("Expected StringForward, got {:?}", other),
                }
                match *b {
                    Target::StringForward(ref s) => assert_eq!(s, "world"),
                    ref other => panic!("Expected StringForward, got {:?}", other),
                }
            }
            other => panic!("Expected And, got {:?}", other),
        }
    }

    #[test]
    fn parse_or_target() {
        match Target::parse("/hello/|/world/").unwrap() {
            Target::Or(a, b) => {
                match *a {
                    Target::StringForward(ref s) => assert_eq!(s, "hello"),
                    ref other => panic!("Expected StringForward, got {:?}", other),
                }
                match *b {
                    Target::StringForward(ref s) => assert_eq!(s, "world"),
                    ref other => panic!("Expected StringForward, got {:?}", other),
                }
            }
            other => panic!("Expected Or, got {:?}", other),
        }
    }

    #[test]
    fn mixed_precedence_and_binds_tighter() {
        // /a/|/b/&/c/ should parse as /a/ | (/b/ & /c/)
        let target = Target::parse("/a/|/b/&/c/").unwrap();
        match target {
            Target::Or(left, right) => {
                match *left {
                    Target::StringForward(ref s) => assert_eq!(s, "a"),
                    ref other => panic!("Expected StringForward(a), got {:?}", other),
                }
                match *right {
                    Target::And(_, _) => {} // /b/ & /c/
                    ref other => panic!("Expected And, got {:?}", other),
                }
            }
            other => panic!("Expected Or at top level, got {:?}", other),
        }
    }

    #[test]
    fn resolve_and_target() {
        let lines = vec![
            "hello world",
            "hello there",
            "goodbye world",
            "hello world again",
        ];
        let target = Target::parse("/hello/&/world/").unwrap();
        let result = target.resolve(0, 4, false, &|n| lines.get(n - 1).map(|s| s.to_string()));
        assert_eq!(result, Some(1)); // "hello world" matches both
    }

    #[test]
    fn resolve_or_target() {
        let lines = vec!["alpha", "beta", "gamma"];
        let target = Target::parse("/beta/|/gamma/").unwrap();
        let result = target.resolve(0, 3, false, &|n| lines.get(n - 1).map(|s| s.to_string()));
        assert_eq!(result, Some(2)); // "beta" matches first
    }
}
