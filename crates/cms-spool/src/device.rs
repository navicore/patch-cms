use std::fmt;

/// The three unit-record spool devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpoolDevice {
    Reader,
    Printer,
    Punch,
}

impl SpoolDevice {
    /// Short directory name used for filesystem-backed storage.
    pub fn dir_name(&self) -> &'static str {
        match self {
            SpoolDevice::Reader => "rdr",
            SpoolDevice::Printer => "prt",
            SpoolDevice::Punch => "pun",
        }
    }
}

impl fmt::Display for SpoolDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpoolDevice::Reader => write!(f, "READER"),
            SpoolDevice::Printer => write!(f, "PRINTER"),
            SpoolDevice::Punch => write!(f, "PUNCH"),
        }
    }
}

/// Spool class (A-Z or * for all classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpoolClass(pub(crate) char);

impl SpoolClass {
    /// The wildcard class that matches all other classes.
    pub const ALL: SpoolClass = SpoolClass('*');

    /// Create a spool class from a character. Returns `None` if invalid.
    /// Accepts A-Z and `*` (wildcard for queries).
    pub fn new(c: char) -> Option<Self> {
        let upper = c.to_ascii_uppercase();
        if upper == '*' || upper.is_ascii_uppercase() {
            Some(SpoolClass(upper))
        } else {
            None
        }
    }

    /// Create a spool class for storing on a file. Rejects `*` (wildcard).
    pub fn for_file(c: char) -> Option<Self> {
        let upper = c.to_ascii_uppercase();
        if upper.is_ascii_uppercase() && upper != '*' {
            Some(SpoolClass(upper))
        } else {
            None
        }
    }

    /// Returns true if this class matches the given class.
    /// The wildcard class `*` matches everything.
    pub fn matches(&self, other: &SpoolClass) -> bool {
        self.0 == '*' || other.0 == '*' || self.0 == other.0
    }
}

impl Default for SpoolClass {
    fn default() -> Self {
        SpoolClass('A')
    }
}

impl fmt::Display for SpoolClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Configuration for a spool device.
#[derive(Debug, Clone)]
pub struct DeviceConfig {
    pub device: SpoolDevice,
    pub class: SpoolClass,
    pub dest: String,
    pub copies: u32,
    pub hold: bool,
    pub continuous: bool,
}

impl DeviceConfig {
    pub fn new(device: SpoolDevice) -> Self {
        Self {
            device,
            class: SpoolClass::default(),
            dest: String::new(),
            copies: 1,
            hold: false,
            continuous: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spool_device_display() {
        assert_eq!(SpoolDevice::Reader.to_string(), "READER");
        assert_eq!(SpoolDevice::Printer.to_string(), "PRINTER");
        assert_eq!(SpoolDevice::Punch.to_string(), "PUNCH");
    }

    #[test]
    fn spool_device_dir_name() {
        assert_eq!(SpoolDevice::Reader.dir_name(), "rdr");
        assert_eq!(SpoolDevice::Printer.dir_name(), "prt");
        assert_eq!(SpoolDevice::Punch.dir_name(), "pun");
    }

    #[test]
    fn spool_class_new_valid() {
        assert_eq!(SpoolClass::new('A'), Some(SpoolClass('A')));
        assert_eq!(SpoolClass::new('z'), Some(SpoolClass('Z')));
        assert_eq!(SpoolClass::new('*'), Some(SpoolClass('*')));
    }

    #[test]
    fn spool_class_new_invalid() {
        assert_eq!(SpoolClass::new('1'), None);
        assert_eq!(SpoolClass::new(' '), None);
    }

    #[test]
    fn spool_class_for_file_rejects_wildcard() {
        assert!(SpoolClass::for_file('A').is_some());
        assert!(SpoolClass::for_file('Z').is_some());
        assert!(SpoolClass::for_file('*').is_none());
        assert!(SpoolClass::for_file('1').is_none());
    }

    #[test]
    fn spool_class_matches() {
        let a = SpoolClass('A');
        let b = SpoolClass('B');
        let all = SpoolClass::ALL;

        assert!(a.matches(&a));
        assert!(!a.matches(&b));
        assert!(all.matches(&a));
        assert!(a.matches(&all));
    }

    #[test]
    fn device_config_defaults() {
        let cfg = DeviceConfig::new(SpoolDevice::Printer);
        assert_eq!(cfg.device, SpoolDevice::Printer);
        assert_eq!(cfg.class, SpoolClass('A'));
        assert!(cfg.dest.is_empty());
        assert_eq!(cfg.copies, 1);
        assert!(!cfg.hold);
        assert!(!cfg.continuous);
    }
}
