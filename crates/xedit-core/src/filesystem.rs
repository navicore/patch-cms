use std::borrow::Cow;
use std::path::Path;

use crate::error::{Result, XeditError};

/// Identity components extracted from a file identifier.
///
/// For native OS files, filename comes from the path stem (uppercased)
/// and filetype from the extension (uppercased). For CMS files, these
/// map directly to the CMS fn/ft/fm components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIdentity {
    pub filename: String,
    pub filetype: String,
    pub filemode: String,
}

/// Abstraction over file I/O so xedit-core can work with native OS files
/// or CMS minidisk files without compile-time coupling.
///
/// `file_id` is an opaque string — `NativeFs` treats it as an OS path,
/// while a CMS adapter treats it as a filespec ("PROFILE EXEC A1").
pub trait FileSystem {
    fn read_file(&self, file_id: &str) -> Result<String>;
    fn write_file(&self, file_id: &str, content: &str) -> Result<()>;
    fn parse_file_id(&self, file_id: &str) -> Option<FileIdentity>;

    /// Normalize user input into the filesystem's canonical file identifier.
    ///
    /// The default implementation is a zero-cost pass-through.  `NativeFs`
    /// overrides this to detect CMS-style space-separated filespecs (e.g.
    /// `"PROFILE EXEC A"`) and convert them to `"profile.exec"`.
    fn normalize_file_id<'a>(&self, input: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(input)
    }
}

/// Default filesystem implementation that delegates to `std::fs`.
pub struct NativeFs;

impl FileSystem for NativeFs {
    fn read_file(&self, file_id: &str) -> Result<String> {
        std::fs::read_to_string(file_id).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                XeditError::FileNotFound(file_id.to_string())
            } else {
                XeditError::Io(e)
            }
        })
    }

    fn write_file(&self, file_id: &str, content: &str) -> Result<()> {
        std::fs::write(file_id, content)?;
        Ok(())
    }

    fn parse_file_id(&self, file_id: &str) -> Option<FileIdentity> {
        let path = Path::new(file_id);
        let filename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_uppercase();
        let filetype = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_uppercase();

        if filename.is_empty() {
            return None;
        }

        Some(FileIdentity {
            filename,
            filetype,
            filemode: "A1".to_string(),
        })
    }

    fn normalize_file_id<'a>(&self, input: &'a str) -> Cow<'a, str> {
        // Tokenize once, bounded to 4 elements regardless of input length.
        let tokens: Vec<&str> = input.split_whitespace().take(4).collect();
        let count = tokens.len();

        // CMS-style: 2-3 tokens, each 1-8 alphanumeric chars (no path separators).
        if (count == 2 || count == 3)
            && tokens.iter().all(|t| {
                !t.is_empty() && t.len() <= 8 && t.chars().all(|c| c.is_ascii_alphanumeric())
            })
        {
            // tokens[0] = filename, tokens[1] = filetype, tokens[2..] = filemode (ignored)
            Cow::Owned(format!(
                "{}.{}",
                tokens[0].to_lowercase(),
                tokens[1].to_lowercase()
            ))
        } else {
            Cow::Borrowed(input)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn native_fs_read_write_roundtrip() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "hello world").unwrap();
        tmp.flush().unwrap();

        let fs = NativeFs;
        let path = tmp.path().to_str().unwrap();
        let content = fs.read_file(path).unwrap();
        assert_eq!(content, "hello world");

        fs.write_file(path, "updated").unwrap();
        let content = fs.read_file(path).unwrap();
        assert_eq!(content, "updated");
    }

    #[test]
    fn native_fs_read_nonexistent() {
        let fs = NativeFs;
        let result = fs.read_file("/tmp/nonexistent_xedit_test_file_12345.txt");
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn native_fs_read_permission_denied() {
        use std::os::unix::fs::PermissionsExt;

        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "secret").unwrap();
        tmp.flush().unwrap();

        // Remove all permissions
        let path = tmp.path().to_str().unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let fs = NativeFs;
        let result = fs.read_file(path);
        assert!(result.is_err());
        match result.unwrap_err() {
            XeditError::Io(_) => {} // should be Io, not FileNotFound
            other => panic!("Expected Io error, got {:?}", other),
        }

        // Restore permissions so tempfile cleanup works
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    #[test]
    fn parse_file_id_with_extension() {
        let fs = NativeFs;
        let id = fs.parse_file_id("/some/path/profile.exec").unwrap();
        assert_eq!(id.filename, "PROFILE");
        assert_eq!(id.filetype, "EXEC");
        assert_eq!(id.filemode, "A1");
    }

    #[test]
    fn parse_file_id_without_extension() {
        let fs = NativeFs;
        let id = fs.parse_file_id("/some/path/makefile").unwrap();
        assert_eq!(id.filename, "MAKEFILE");
        assert_eq!(id.filetype, "");
        assert_eq!(id.filemode, "A1");
    }

    #[test]
    fn parse_file_id_empty() {
        let fs = NativeFs;
        assert!(fs.parse_file_id("").is_none());
    }

    // -- normalize_file_id tests --

    #[test]
    fn normalize_three_token_cms_style() {
        let fs = NativeFs;
        assert_eq!(fs.normalize_file_id("PROFILE EXEC A"), "profile.exec");
    }

    #[test]
    fn normalize_two_token_cms_style() {
        let fs = NativeFs;
        assert_eq!(fs.normalize_file_id("PROFILE EXEC"), "profile.exec");
    }

    #[test]
    fn normalize_dotted_path_passthrough() {
        let fs = NativeFs;
        assert_eq!(fs.normalize_file_id("profile.exec"), "profile.exec");
    }

    #[test]
    fn normalize_absolute_path_passthrough() {
        let fs = NativeFs;
        assert_eq!(
            fs.normalize_file_id("/home/user/profile.exec"),
            "/home/user/profile.exec"
        );
    }

    #[test]
    fn normalize_single_word_passthrough() {
        let fs = NativeFs;
        assert_eq!(fs.normalize_file_id("Makefile"), "Makefile");
    }

    #[test]
    fn normalize_four_tokens_passthrough() {
        let fs = NativeFs;
        assert_eq!(
            fs.normalize_file_id("one two three four"),
            "one two three four"
        );
    }

    #[test]
    fn normalize_mixed_case() {
        let fs = NativeFs;
        assert_eq!(fs.normalize_file_id("Profile Exec A1"), "profile.exec");
    }

    #[test]
    fn normalize_backslash_path_passthrough() {
        let fs = NativeFs;
        assert_eq!(
            fs.normalize_file_id("C:\\Users\\file txt"),
            "C:\\Users\\file txt"
        );
    }

    #[test]
    fn normalize_empty_string_passthrough() {
        let fs = NativeFs;
        assert_eq!(fs.normalize_file_id(""), "");
    }

    #[test]
    fn normalize_whitespace_only_passthrough() {
        let fs = NativeFs;
        assert_eq!(fs.normalize_file_id("   "), "   ");
    }

    #[test]
    fn normalize_tab_separated_tokens() {
        let fs = NativeFs;
        // split_whitespace splits on tabs too — two alphanumeric tokens
        // are treated as CMS-style, matching the current behavior.
        assert_eq!(fs.normalize_file_id("PROFILE\tEXEC"), "profile.exec");
    }

    #[test]
    fn normalize_token_exceeding_eight_chars_passthrough() {
        let fs = NativeFs;
        // CMS filenames are limited to 8 chars; longer tokens pass through
        assert_eq!(
            fs.normalize_file_id("LONGFILENAME EXEC"),
            "LONGFILENAME EXEC"
        );
    }
}
