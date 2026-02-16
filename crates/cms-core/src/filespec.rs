use std::fmt;

use crate::error::{CmsError, Result};

/// CMS file identity: FILENAME FILETYPE FILEMODE
///
/// FILENAME and FILETYPE are 1-8 characters from `[A-Z0-9$#@]` (uppercased),
/// or `*` as a wildcard for the entire component.
///
/// FILEMODE is a letter A-Z plus an optional digit 0-6. Default is `A1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSpec {
    filename: String,
    filetype: String,
    mode_letter: char,
    mode_number: u8,
}

/// Check whether a string is a valid CMS filename/filetype component.
/// Must be 1-8 characters, all `[A-Z0-9$#@]` (already uppercased), or exactly `*`.
fn validate_component(s: &str, label: &str) -> Result<()> {
    if s == "*" {
        return Ok(());
    }
    if s.is_empty() || s.len() > 8 {
        return Err(CmsError::InvalidFileSpec(format!(
            "{} must be 1-8 characters, got '{}'",
            label, s
        )));
    }
    for ch in s.chars() {
        if !ch.is_ascii_alphanumeric() && ch != '$' && ch != '#' && ch != '@' {
            return Err(CmsError::InvalidFileSpec(format!(
                "{} contains invalid character '{}'",
                label, ch
            )));
        }
    }
    Ok(())
}

impl FileSpec {
    /// Construct a FileSpec with explicit components and validation.
    pub fn new(filename: &str, filetype: &str, filemode: &str) -> Result<Self> {
        let filename = filename.to_ascii_uppercase();
        let filetype = filetype.to_ascii_uppercase();
        let filemode = filemode.to_ascii_uppercase();

        validate_component(&filename, "Filename")?;
        validate_component(&filetype, "Filetype")?;

        let (letter, number) = parse_filemode(&filemode)?;

        Ok(FileSpec {
            filename,
            filetype,
            mode_letter: letter,
            mode_number: number,
        })
    }

    /// Parse a space-delimited filespec: `"PROFILE EXEC A1"`.
    ///
    /// Filemode is optional and defaults to `A1`. If only a letter is given
    /// (e.g. `"PROFILE EXEC A"`), the digit defaults to `1`.
    pub fn parse(input: &str) -> Result<Self> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.len() < 2 || parts.len() > 3 {
            return Err(CmsError::InvalidFileSpec(format!(
                "Expected 'fn ft [fm]', got '{}'",
                input
            )));
        }

        let filename = parts[0].to_ascii_uppercase();
        let filetype = parts[1].to_ascii_uppercase();

        validate_component(&filename, "Filename")?;
        validate_component(&filetype, "Filetype")?;

        let (letter, number) = if parts.len() == 3 {
            let fm = parts[2].to_ascii_uppercase();
            parse_filemode(&fm)?
        } else {
            ('A', 1)
        };

        Ok(FileSpec {
            filename,
            filetype,
            mode_letter: letter,
            mode_number: number,
        })
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn filetype(&self) -> &str {
        &self.filetype
    }

    pub fn mode_letter(&self) -> char {
        self.mode_letter
    }

    pub fn mode_number(&self) -> u8 {
        self.mode_number
    }

    /// Returns the full filemode string, e.g. `"A1"`.
    pub fn filemode(&self) -> String {
        format!("{}{}", self.mode_letter, self.mode_number)
    }

    /// True if filename or filetype is `*`.
    pub fn has_wildcards(&self) -> bool {
        self.filename == "*" || self.filetype == "*"
    }

    /// Pattern matching with `*` wildcards.
    /// A `*` in self matches any value in the corresponding component of `other`.
    /// The filemode letter `*` in self matches any disk letter.
    pub fn matches(&self, other: &FileSpec) -> bool {
        let fn_match = self.filename == "*" || self.filename == other.filename;
        let ft_match = self.filetype == "*" || self.filetype == other.filetype;
        let fm_match = self.mode_letter == '*' || self.mode_letter == other.mode_letter;
        fn_match && ft_match && fm_match
    }

    /// Returns `"filename.filetype"` in lowercase for on-disk storage.
    pub fn disk_filename(&self) -> String {
        format!(
            "{}.{}",
            self.filename.to_lowercase(),
            self.filetype.to_lowercase()
        )
    }
}

/// Parse a filemode string like `"A1"`, `"A"`, or `"*"`.
fn parse_filemode(fm: &str) -> Result<(char, u8)> {
    if fm.is_empty() {
        return Ok(('A', 1));
    }

    let mut chars = fm.chars();
    let letter = chars.next().unwrap();

    if letter == '*' {
        // Wildcard filemode — digit is ignored for matching purposes
        let number = match chars.next() {
            Some(d) if d.is_ascii_digit() => {
                let n = d.to_digit(10).unwrap() as u8;
                if n > 6 {
                    return Err(CmsError::InvalidFileSpec(format!(
                        "Filemode digit must be 0-6, got {}",
                        n
                    )));
                }
                n
            }
            Some(c) => {
                return Err(CmsError::InvalidFileSpec(format!(
                    "Invalid filemode digit '{}'",
                    c
                )));
            }
            None => 1,
        };
        return Ok(('*', number));
    }

    if !letter.is_ascii_uppercase() {
        return Err(CmsError::InvalidFileSpec(format!(
            "Filemode letter must be A-Z, got '{}'",
            letter
        )));
    }

    let number = match chars.next() {
        Some(d) if d.is_ascii_digit() => {
            let n = d.to_digit(10).unwrap() as u8;
            if n > 6 {
                return Err(CmsError::InvalidFileSpec(format!(
                    "Filemode digit must be 0-6, got {}",
                    n
                )));
            }
            n
        }
        Some(c) => {
            return Err(CmsError::InvalidFileSpec(format!(
                "Invalid filemode digit '{}'",
                c
            )));
        }
        None => 1,
    };

    if chars.next().is_some() {
        return Err(CmsError::InvalidFileSpec(format!(
            "Filemode too long: '{}'",
            fm
        )));
    }

    Ok((letter, number))
}

impl fmt::Display for FileSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {}{}",
            self.filename, self.filetype, self.mode_letter, self.mode_number
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_spec() {
        let spec = FileSpec::parse("PROFILE EXEC A1").unwrap();
        assert_eq!(spec.filename(), "PROFILE");
        assert_eq!(spec.filetype(), "EXEC");
        assert_eq!(spec.mode_letter(), 'A');
        assert_eq!(spec.mode_number(), 1);
    }

    #[test]
    fn parse_default_filemode() {
        let spec = FileSpec::parse("MYFILE DATA").unwrap();
        assert_eq!(spec.mode_letter(), 'A');
        assert_eq!(spec.mode_number(), 1);
    }

    #[test]
    fn parse_letter_only_filemode() {
        let spec = FileSpec::parse("X Y S").unwrap();
        assert_eq!(spec.mode_letter(), 'S');
        assert_eq!(spec.mode_number(), 1);
    }

    #[test]
    fn parse_with_digit() {
        let spec = FileSpec::parse("X Y S2").unwrap();
        assert_eq!(spec.mode_letter(), 'S');
        assert_eq!(spec.mode_number(), 2);
    }

    #[test]
    fn parse_wildcards_filename() {
        let spec = FileSpec::parse("* EXEC A").unwrap();
        assert!(spec.has_wildcards());
        assert_eq!(spec.filename(), "*");
        assert_eq!(spec.filetype(), "EXEC");
    }

    #[test]
    fn parse_wildcards_filetype() {
        let spec = FileSpec::parse("MYFILE * A").unwrap();
        assert!(spec.has_wildcards());
    }

    #[test]
    fn parse_wildcard_filemode() {
        let spec = FileSpec::parse("MYFILE DATA *").unwrap();
        assert_eq!(spec.mode_letter(), '*');
    }

    #[test]
    fn reject_filename_too_long() {
        let result = FileSpec::parse("TOOLONGNAME EXEC A");
        assert!(result.is_err());
    }

    #[test]
    fn reject_invalid_chars() {
        let result = FileSpec::parse("BAD!NAME EXEC A");
        assert!(result.is_err());
    }

    #[test]
    fn reject_bad_filemode_letter() {
        let result = FileSpec::parse("X Y 1");
        assert!(result.is_err());
    }

    #[test]
    fn reject_bad_mode_digit() {
        let result = FileSpec::parse("X Y A7");
        assert!(result.is_err());
    }

    #[test]
    fn case_insensitivity() {
        let spec = FileSpec::parse("profile exec a1").unwrap();
        assert_eq!(spec.filename(), "PROFILE");
        assert_eq!(spec.filetype(), "EXEC");
        assert_eq!(spec.mode_letter(), 'A');
    }

    #[test]
    fn matches_exact() {
        let pattern = FileSpec::parse("PROFILE EXEC A").unwrap();
        let candidate = FileSpec::parse("PROFILE EXEC A").unwrap();
        assert!(pattern.matches(&candidate));
    }

    #[test]
    fn matches_wildcard_filename() {
        let pattern = FileSpec::parse("* EXEC A").unwrap();
        let candidate = FileSpec::parse("PROFILE EXEC A").unwrap();
        assert!(pattern.matches(&candidate));
    }

    #[test]
    fn matches_wildcard_filetype() {
        let pattern = FileSpec::parse("PROFILE * A").unwrap();
        let candidate = FileSpec::parse("PROFILE EXEC A").unwrap();
        assert!(pattern.matches(&candidate));
    }

    #[test]
    fn matches_wildcard_filemode() {
        let pattern = FileSpec::parse("PROFILE EXEC *").unwrap();
        let a = FileSpec::parse("PROFILE EXEC A").unwrap();
        let b = FileSpec::parse("PROFILE EXEC B").unwrap();
        assert!(pattern.matches(&a));
        assert!(pattern.matches(&b));
    }

    #[test]
    fn no_match() {
        let pattern = FileSpec::parse("PROFILE EXEC A").unwrap();
        let candidate = FileSpec::parse("OTHER EXEC A").unwrap();
        assert!(!pattern.matches(&candidate));
    }

    #[test]
    fn disk_filename_lowercase() {
        let spec = FileSpec::parse("PROFILE EXEC A1").unwrap();
        assert_eq!(spec.disk_filename(), "profile.exec");
    }

    #[test]
    fn display_roundtrip() {
        let spec = FileSpec::parse("PROFILE EXEC A1").unwrap();
        assert_eq!(spec.to_string(), "PROFILE EXEC A1");
        let reparsed = FileSpec::parse(&spec.to_string()).unwrap();
        assert_eq!(spec, reparsed);
    }

    #[test]
    fn new_explicit() {
        let spec = FileSpec::new("profile", "exec", "A1").unwrap();
        assert_eq!(spec.filename(), "PROFILE");
        assert_eq!(spec.filetype(), "EXEC");
        assert_eq!(spec.filemode(), "A1");
    }

    #[test]
    fn has_wildcards_ignores_mode_letter() {
        // mode_letter '*' is a search directive, not a filename/filetype wildcard
        let spec = FileSpec::parse("MYFILE DATA *").unwrap();
        assert!(!spec.has_wildcards());
    }

    #[test]
    fn reject_non_ascii_chars() {
        let result = FileSpec::parse("caf\u{00e9} DATA A");
        assert!(result.is_err());
    }

    #[test]
    fn reject_single_token() {
        let result = FileSpec::parse("ONLYNAME");
        assert!(result.is_err());
    }

    #[test]
    fn reject_too_many_tokens() {
        let result = FileSpec::parse("FN FT FM EXTRA");
        assert!(result.is_err());
    }
}
