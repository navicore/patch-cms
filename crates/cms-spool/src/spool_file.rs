use crate::device::{SpoolClass, SpoolDevice};

/// Metadata for a file in the spool queue.
#[derive(Debug, Clone)]
pub struct SpoolFile {
    /// Unique spool identifier.
    pub spool_id: u64,
    /// CMS filename (e.g. "PROFILE").
    pub filename: String,
    /// CMS filetype (e.g. "EXEC").
    pub filetype: String,
    /// User who originated this file.
    pub origin_user: String,
    /// Destination user (empty = self).
    pub dest_user: String,
    /// Spool class.
    pub class: SpoolClass,
    /// Number of records (lines).
    pub records: usize,
    /// Which device queue this file is on.
    pub device: SpoolDevice,
    /// Whether the file is held.
    pub hold: bool,
    /// Number of copies requested.
    pub copies: u32,
}

/// Validate metadata fields before enqueue.
///
/// Checks that required fields are non-empty, respect the 8-character CMS
/// maximum, contain only CMS-legal characters (`A-Z 0-9 @ # $`), and do
/// not contain newlines (which would corrupt the key=value `.meta` format).
/// `dest_user` may be empty (meaning "self").
pub fn validate_enqueue_fields(
    filename: &str,
    filetype: &str,
    origin_user: &str,
    dest_user: &str,
) -> crate::error::Result<()> {
    /// CMS filenames/filetypes/userids: uppercase alphanumeric + @#$
    fn is_cms_legal(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 8
            && s.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b"@#$".contains(&b))
    }
    if !is_cms_legal(filename) || !is_cms_legal(filetype) || !is_cms_legal(origin_user) {
        return Err(crate::error::SpoolError::InvalidParameter(
            "spool metadata fields must be 1-8 CMS-legal characters (A-Z 0-9 @ # $)".to_string(),
        ));
    }
    // dest_user may be empty (meaning "self"), but if set must be valid
    if !dest_user.is_empty() && !is_cms_legal(dest_user) {
        return Err(crate::error::SpoolError::InvalidParameter(
            "dest_user must be 1-8 CMS-legal characters (A-Z 0-9 @ # $)".to_string(),
        ));
    }
    Ok(())
}

impl SpoolFile {
    /// Create a new spool file with the given metadata.
    pub fn new(
        spool_id: u64,
        filename: &str,
        filetype: &str,
        origin_user: &str,
        device: SpoolDevice,
    ) -> Self {
        Self {
            spool_id,
            filename: filename.to_ascii_uppercase(),
            filetype: filetype.to_ascii_uppercase(),
            origin_user: origin_user.to_ascii_uppercase(),
            dest_user: String::new(),
            class: SpoolClass::default(),
            records: 0,
            device,
            hold: false,
            copies: 1,
        }
    }

    /// Format a one-line summary for QUERY output.
    pub fn summary(&self) -> String {
        format!(
            "{:>7} {:8} {:8} {:2}{:>5} {:4} {}",
            self.spool_id,
            self.filename,
            self.filetype,
            self.class,
            self.records,
            if self.hold { "HOLD" } else { "NONE" },
            if self.dest_user.is_empty() {
                &self.origin_user
            } else {
                &self.dest_user
            },
        )
    }

    /// Serialize metadata to key=value format (no serde dependency).
    pub fn to_meta_string(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::with_capacity(200);
        let _ = writeln!(s, "SPOOL_ID={}", self.spool_id);
        let _ = writeln!(s, "FILENAME={}", self.filename);
        let _ = writeln!(s, "FILETYPE={}", self.filetype);
        let _ = writeln!(s, "ORIGIN_USER={}", self.origin_user);
        let _ = writeln!(s, "DEST_USER={}", self.dest_user);
        let _ = writeln!(s, "CLASS={}", self.class);
        let _ = writeln!(s, "RECORDS={}", self.records);
        let _ = writeln!(s, "DEVICE={}", self.device.as_meta_str());
        let _ = writeln!(s, "HOLD={}", self.hold);
        let _ = writeln!(s, "COPIES={}", self.copies);
        s
    }

    /// Parse metadata from key=value format.
    pub fn from_meta_string(s: &str) -> Option<Self> {
        let mut spool_id = None;
        let mut filename = None;
        let mut filetype = None;
        let mut origin_user = None;
        let mut dest_user = String::new();
        let mut class = SpoolClass::default();
        let mut records = 0usize;
        let mut device = None;
        let mut hold = false;
        let mut copies = 1u32;

        for line in s.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                match key {
                    "SPOOL_ID" => spool_id = value.parse().ok(),
                    "FILENAME" => {
                        if value.is_empty() || value.len() > 8 {
                            return None;
                        }
                        filename = Some(value.to_ascii_uppercase());
                    }
                    "FILETYPE" => {
                        if value.is_empty() || value.len() > 8 {
                            return None;
                        }
                        filetype = Some(value.to_ascii_uppercase());
                    }
                    "ORIGIN_USER" => {
                        if value.is_empty() || value.len() > 8 {
                            return None;
                        }
                        origin_user = Some(value.to_ascii_uppercase());
                    }
                    "DEST_USER" => {
                        if value.len() > 8 {
                            return None;
                        }
                        dest_user = value.to_ascii_uppercase();
                    }
                    "CLASS" => {
                        if value.len() != 1 {
                            return None; // CLASS must be exactly one character
                        }
                        class = SpoolClass::for_file(value.chars().next()?)?;
                    }
                    "RECORDS" => records = value.parse().unwrap_or(0),
                    "DEVICE" => {
                        device = SpoolDevice::from_meta_str(value);
                    }
                    "HOLD" => hold = value == "true",
                    "COPIES" => {
                        copies = value
                            .parse()
                            .ok()
                            .filter(|n: &u32| (1..=255).contains(n))
                            .unwrap_or(1)
                    }
                    _ => {}
                }
            }
        }

        Some(SpoolFile {
            spool_id: spool_id?,
            filename: filename?,
            filetype: filetype?,
            origin_user: origin_user?,
            dest_user,
            class,
            records,
            device: device?,
            hold,
            copies,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spool_file_new_uppercases() {
        let sf = SpoolFile::new(1, "profile", "exec", "jones", SpoolDevice::Reader);
        assert_eq!(sf.filename, "PROFILE");
        assert_eq!(sf.filetype, "EXEC");
        assert_eq!(sf.origin_user, "JONES");
    }

    #[test]
    fn spool_file_summary() {
        let sf = SpoolFile::new(12345, "MYFILE", "DATA", "JONES", SpoolDevice::Reader);
        let s = sf.summary();
        assert!(s.contains("12345"));
        assert!(s.contains("MYFILE"));
        assert!(s.contains("DATA"));
        assert!(s.contains("JONES"));
    }

    #[test]
    fn spool_file_meta_roundtrip() {
        let mut sf = SpoolFile::new(42, "TEST", "FILE", "USER1", SpoolDevice::Printer);
        sf.dest_user = "USER2".to_string();
        sf.class = SpoolClass('B');
        sf.records = 100;
        sf.hold = true;
        sf.copies = 3;

        let meta = sf.to_meta_string();
        let parsed = SpoolFile::from_meta_string(&meta).unwrap();

        assert_eq!(parsed.spool_id, 42);
        assert_eq!(parsed.filename, "TEST");
        assert_eq!(parsed.filetype, "FILE");
        assert_eq!(parsed.origin_user, "USER1");
        assert_eq!(parsed.dest_user, "USER2");
        assert_eq!(parsed.class, SpoolClass('B'));
        assert_eq!(parsed.records, 100);
        assert_eq!(parsed.device, SpoolDevice::Printer);
        assert!(parsed.hold);
        assert_eq!(parsed.copies, 3);
    }

    #[test]
    fn spool_file_meta_parse_incomplete() {
        // Missing required fields should return None
        let result = SpoolFile::from_meta_string("SPOOL_ID=1\nFILENAME=X\n");
        assert!(result.is_none());
    }

    #[test]
    fn spool_file_meta_rejects_wildcard_class() {
        let meta =
            "SPOOL_ID=1\nFILENAME=TEST\nFILETYPE=DATA\nORIGIN_USER=U\nCLASS=*\nDEVICE=READER\n";
        assert!(SpoolFile::from_meta_string(meta).is_none());
    }

    #[test]
    fn spool_file_meta_multi_char_class_rejected() {
        let meta = "SPOOL_ID=1\nFILENAME=T\nFILETYPE=D\nORIGIN_USER=U\nCLASS=AB\nDEVICE=READER\n";
        assert!(SpoolFile::from_meta_string(meta).is_none());
    }

    #[test]
    fn spool_file_meta_empty_class_rejected() {
        let meta = "SPOOL_ID=1\nFILENAME=T\nFILETYPE=D\nORIGIN_USER=U\nCLASS=\nDEVICE=READER\n";
        assert!(SpoolFile::from_meta_string(meta).is_none());
    }

    #[test]
    fn spool_file_meta_invalid_class_digit_rejected() {
        let meta = "SPOOL_ID=1\nFILENAME=T\nFILETYPE=D\nORIGIN_USER=U\nCLASS=5\nDEVICE=READER\n";
        assert!(SpoolFile::from_meta_string(meta).is_none());
    }

    #[test]
    fn spool_file_meta_corrupt_records_still_parses() {
        let meta =
            "SPOOL_ID=1\nFILENAME=T\nFILETYPE=D\nORIGIN_USER=U\nRECORDS=NaN\nDEVICE=READER\n";
        let sf = SpoolFile::from_meta_string(meta).unwrap();
        assert_eq!(sf.records, 0);
    }
}
