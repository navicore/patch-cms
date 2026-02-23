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
            "{:>7} {:8} {:8} {} {:>5} {} {}",
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
        let mut s = String::new();
        s.push_str(&format!("SPOOL_ID={}\n", self.spool_id));
        s.push_str(&format!("FILENAME={}\n", self.filename));
        s.push_str(&format!("FILETYPE={}\n", self.filetype));
        s.push_str(&format!("ORIGIN_USER={}\n", self.origin_user));
        s.push_str(&format!("DEST_USER={}\n", self.dest_user));
        s.push_str(&format!("CLASS={}\n", self.class));
        s.push_str(&format!("RECORDS={}\n", self.records));
        s.push_str(&format!("DEVICE={}\n", self.device));
        s.push_str(&format!("HOLD={}\n", self.hold));
        s.push_str(&format!("COPIES={}\n", self.copies));
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
                    "FILENAME" => filename = Some(value.to_string()),
                    "FILETYPE" => filetype = Some(value.to_string()),
                    "ORIGIN_USER" => origin_user = Some(value.to_string()),
                    "DEST_USER" => dest_user = value.to_string(),
                    "CLASS" => {
                        if let Some(c) = value.chars().next() {
                            if let Some(sc) = SpoolClass::for_file(c) {
                                class = sc;
                            }
                        }
                    }
                    "RECORDS" => records = value.parse().unwrap_or(0),
                    "DEVICE" => {
                        device = match value {
                            "READER" => Some(SpoolDevice::Reader),
                            "PRINTER" => Some(SpoolDevice::Printer),
                            "PUNCH" => Some(SpoolDevice::Punch),
                            _ => None,
                        };
                    }
                    "HOLD" => hold = value == "true",
                    "COPIES" => copies = value.parse().unwrap_or(1),
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
        // CLASS=* in a .meta file should be treated as default (A), not wildcard
        let meta =
            "SPOOL_ID=1\nFILENAME=TEST\nFILETYPE=DATA\nORIGIN_USER=U\nCLASS=*\nDEVICE=READER\n";
        let sf = SpoolFile::from_meta_string(meta).unwrap();
        // for_file rejects '*', so class stays at default 'A'
        assert_eq!(sf.class, SpoolClass::default());
    }
}
