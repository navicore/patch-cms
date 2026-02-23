use crate::backend::SpoolBackend;
use crate::device::{SpoolClass, SpoolDevice};
use crate::spool::{SpoolCommandResult, SpoolManager};

/// Parsed spool commands.
#[derive(Debug, PartialEq)]
pub enum SpoolCommand {
    /// SPOOL device configuration: `SP PRT CLASS A DEST OPERATOR`
    Spool {
        device: SpoolDevice,
        class: Option<SpoolClass>,
        dest: Option<String>,
        hold: Option<bool>,
        continuous: Option<bool>,
        copies: Option<u32>,
    },
    /// SENDFILE: `SE MYFILE DATA A TO JONES`
    SendFile {
        filename: String,
        filetype: String,
        filemode: Option<char>,
        dest_user: Option<String>,
    },
    /// RECEIVE: `REC` or `REC NEWNAME DATA A`
    Receive {
        filename: Option<String>,
        filetype: Option<String>,
        filemode: Option<char>,
    },
    /// PURGE: `PUR READER ALL` or `PUR READER 12345`
    Purge {
        device: SpoolDevice,
        target: PurgeTarget,
    },
    /// QUERY: `QUERY READER`
    Query {
        device: SpoolDevice,
        class: Option<SpoolClass>,
    },
}

/// Target for a PURGE command.
#[derive(Debug, PartialEq)]
pub enum PurgeTarget {
    /// Purge all files in the queue.
    All,
    /// Purge a specific file by spool ID.
    SpoolId(u64),
}

/// Abbreviation table for spool commands.
const SPOOL_COMMAND_TABLE: &[(&str, usize)] = &[
    ("PURGE", 3),    // PUR
    ("QUERY", 1),    // Q
    ("RECEIVE", 3),  // REC
    ("SENDFILE", 2), // SE
    ("SPOOL", 2),    // SP
];

/// Abbreviation table for device names.
const DEVICE_TABLE: &[(&str, usize, SpoolDevice)] = &[
    ("PRINTER", 2, SpoolDevice::Printer), // PR
    ("PUNCH", 2, SpoolDevice::Punch),     // PU
    ("READER", 1, SpoolDevice::Reader),   // R
];

/// Common CMS device aliases (not prefix abbreviations).
const DEVICE_ALIASES: &[(&str, SpoolDevice)] = &[
    ("PRT", SpoolDevice::Printer),
    ("RDR", SpoolDevice::Reader),
    ("PUN", SpoolDevice::Punch),
    ("PCH", SpoolDevice::Punch),
];

fn lookup_command(input: &str) -> Option<&'static str> {
    let input_upper = input.to_ascii_uppercase();
    // Exact match first
    for &(name, _) in SPOOL_COMMAND_TABLE {
        if name == input_upper {
            return Some(name);
        }
    }
    // Abbreviation match — no Vec needed, just count
    let mut found = None;
    let mut count = 0usize;
    for &(name, min_abbrev) in SPOOL_COMMAND_TABLE {
        if input_upper.len() >= min_abbrev && name.starts_with(&input_upper) {
            found = Some(name);
            count += 1;
            if count > 1 {
                return None;
            }
        }
    }
    if count == 1 {
        found
    } else {
        None
    }
}

fn lookup_device(input: &str) -> Option<SpoolDevice> {
    let input_upper = input.to_ascii_uppercase();
    // Exact match first
    for &(name, _, device) in DEVICE_TABLE {
        if name == input_upper {
            return Some(device);
        }
    }
    // Common aliases (PRT, RDR, PUN, PCH)
    for &(alias, device) in DEVICE_ALIASES {
        if alias == input_upper {
            return Some(device);
        }
    }
    // Abbreviation match — no Vec needed, just count
    let mut found = None;
    let mut count = 0usize;
    for &(name, min_abbrev, device) in DEVICE_TABLE {
        if input_upper.len() >= min_abbrev && name.starts_with(&input_upper) {
            found = Some(device);
            count += 1;
            if count > 1 {
                return None;
            }
        }
    }
    if count == 1 {
        found
    } else {
        None
    }
}

/// Parse a spool command string.
///
/// Returns `None` if the input is not a spool command (allowing fallback
/// to other command processors).
pub fn parse_spool_command(input: &str) -> Option<SpoolCommand> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    let cmd_word = parts[0];
    let rest = &parts[1..];

    let cmd_name = lookup_command(cmd_word)?;

    match cmd_name {
        "SPOOL" => parse_spool_device(rest),
        "SENDFILE" => parse_sendfile(rest),
        "RECEIVE" => parse_receive(rest),
        "PURGE" => parse_purge(rest),
        "QUERY" => parse_query(rest),
        _ => None,
    }
}

fn parse_spool_device(parts: &[&str]) -> Option<SpoolCommand> {
    if parts.is_empty() {
        return None;
    }

    let device = lookup_device(parts[0])?;
    let mut class = None;
    let mut dest = None;
    let mut hold = None;
    let mut continuous = None;
    let mut copies = None;

    let mut seen: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
    let mut i = 1;
    while i < parts.len() {
        let word = parts[i].to_ascii_uppercase();
        // Reject duplicate keywords
        let keyword: &'static str = match word.as_str() {
            "HOLD" | "NOHOLD" => "HOLD",
            "CONT" | "NOCONT" => "CONT",
            "CLASS" => "CLASS",
            "DEST" => "DEST",
            "COPY" => "COPY",
            _ => "", // unknown — handled by match below
        };
        if !keyword.is_empty() && !seen.insert(keyword) {
            return None; // duplicate option — RC=24
        }
        match word.as_str() {
            "CLASS" => {
                i += 1;
                if i >= parts.len() {
                    return None; // dangling CLASS — RC=24
                }
                if parts[i].len() != 1 {
                    return None; // class must be exactly one character — RC=24
                }
                match parts[i].chars().next().and_then(SpoolClass::for_file) {
                    Some(c) => class = Some(c),
                    None => return None, // invalid class char — RC=24
                }
            }
            "DEST" => {
                i += 1;
                if i >= parts.len() {
                    return None; // dangling DEST — RC=24
                }
                dest = Some(parts[i].to_ascii_uppercase());
            }
            "HOLD" => hold = Some(true),
            "NOHOLD" => hold = Some(false),
            "CONT" => continuous = Some(true),
            "NOCONT" => continuous = Some(false),
            "COPY" => {
                i += 1;
                if i >= parts.len() {
                    return None; // dangling COPY — RC=24
                }
                match parts[i].parse::<u32>() {
                    Ok(n) if n >= 1 => copies = Some(n),
                    _ => return None, // invalid copy count — RC=24
                }
            }
            _ => return None, // unknown option — RC=24
        }
        i += 1;
    }

    Some(SpoolCommand::Spool {
        device,
        class,
        dest,
        hold,
        continuous,
        copies,
    })
}

fn parse_sendfile(parts: &[&str]) -> Option<SpoolCommand> {
    if parts.is_empty() {
        return None;
    }

    let filename = parts[0].to_ascii_uppercase();
    let filetype = if parts.len() > 1 {
        parts[1].to_ascii_uppercase()
    } else {
        return None;
    };

    let mut filemode = None;
    let mut dest_user = None;
    let mut i = 2;

    // Optional filemode: single letter (A) or letter+digit (A1)
    if i < parts.len() {
        let word = parts[i].to_ascii_uppercase();
        if word != "TO" {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                if first.is_ascii_uppercase() {
                    let rest = chars.next();
                    let valid = match rest {
                        None => true,                                            // single letter
                        Some(d) if d.is_ascii_digit() => chars.next().is_none(), // letter+digit
                        _ => false,
                    };
                    if valid {
                        filemode = Some(first);
                        i += 1;
                    }
                }
            }
        }
    }

    // Optional TO <userid> — reject any other tokens
    if i < parts.len() {
        let word = parts[i].to_ascii_uppercase();
        if word == "TO" {
            i += 1;
            if i >= parts.len() {
                return None; // dangling TO — RC=24
            }
            dest_user = Some(parts[i].to_ascii_uppercase());
            i += 1;
        } else {
            return None; // unexpected token — RC=24
        }
    }
    // Reject trailing tokens
    if i < parts.len() {
        return None;
    }

    Some(SpoolCommand::SendFile {
        filename,
        filetype,
        filemode,
        dest_user,
    })
}

fn parse_receive(parts: &[&str]) -> Option<SpoolCommand> {
    if parts.is_empty() {
        return Some(SpoolCommand::Receive {
            filename: None,
            filetype: None,
            filemode: None,
        });
    }

    let filename = Some(parts[0].to_ascii_uppercase());
    let filetype = if parts.len() > 1 {
        Some(parts[1].to_ascii_uppercase())
    } else {
        None
    };
    let mut consumed = if filetype.is_some() { 2 } else { 1 };
    let filemode = if parts.len() > consumed {
        let word = parts[consumed].to_ascii_uppercase();
        // Filemode: single letter or letter+digit
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            if first.is_ascii_uppercase() {
                let valid = match chars.next() {
                    None => true,
                    Some(d) if d.is_ascii_digit() => chars.next().is_none(),
                    _ => false,
                };
                if valid {
                    consumed += 1;
                    Some(first)
                } else {
                    return None; // invalid filemode — RC=24
                }
            } else {
                return None; // invalid filemode — RC=24
            }
        } else {
            None
        }
    } else {
        None
    };

    // Reject trailing tokens
    if consumed < parts.len() {
        return None;
    }

    Some(SpoolCommand::Receive {
        filename,
        filetype,
        filemode,
    })
}

fn parse_purge(parts: &[&str]) -> Option<SpoolCommand> {
    if parts.is_empty() {
        return None;
    }

    let device = lookup_device(parts[0])?;

    // Require explicit ALL or a spool ID — bare "PURGE <device>" is invalid (RC=24)
    if parts.len() < 2 {
        return None;
    }

    let target = {
        let word = parts[1].to_ascii_uppercase();
        if word == "ALL" {
            PurgeTarget::All
        } else if let Ok(id) = parts[1].parse::<u64>() {
            PurgeTarget::SpoolId(id)
        } else {
            return None;
        }
    };

    // Reject trailing tokens
    if parts.len() > 2 {
        return None;
    }

    Some(SpoolCommand::Purge { device, target })
}

fn parse_query(parts: &[&str]) -> Option<SpoolCommand> {
    if parts.is_empty() {
        return None;
    }

    let device = lookup_device(parts[0])?;

    let class = if parts.len() > 1 {
        let word = parts[1].to_ascii_uppercase();
        if word == "CLASS" {
            if parts.len() > 2 {
                if parts[2].len() != 1 {
                    return None; // class must be exactly one character — RC=24
                }
                parts[2].chars().next().and_then(SpoolClass::for_file)
            } else {
                return None; // dangling CLASS without value — RC=24
            }
        } else {
            return None; // unknown query option — RC=24
        }
    } else {
        None
    };

    // Reject trailing tokens (valid forms: Q R, Q R CLASS A)
    let expected_len = if class.is_some() { 3 } else { 1 };
    if parts.len() > expected_len {
        return None;
    }

    Some(SpoolCommand::Query { device, class })
}

/// Execute a parsed spool command against a SpoolManager.
///
/// Execute a parsed spool command against a SpoolManager.
///
/// Handles SPOOL, PURGE, and QUERY commands. SENDFILE and RECEIVE require
/// external file I/O and must be handled by the caller before invoking this
/// function (see `SpoolCommand::SendFile` and `SpoolCommand::Receive`).
///
/// Returns RC=24 if called with `SpoolCommand::SendFile` or
/// `SpoolCommand::Receive` (these require CMS filesystem bridging).
pub fn execute_spool_command<B: SpoolBackend>(
    cmd: &SpoolCommand,
    manager: &mut SpoolManager<B>,
) -> SpoolCommandResult {
    match cmd {
        SpoolCommand::Spool {
            device,
            class,
            dest,
            hold,
            continuous,
            copies,
        } => {
            manager.configure_device(
                *device,
                *class,
                dest.as_deref(),
                *hold,
                *continuous,
                *copies,
            );
            let cfg = manager.device_config(*device);
            SpoolCommandResult::ok_with(vec![format!(
                "{} CLASS {} DEST {} COPY {} {}{}",
                device,
                cfg.class,
                if cfg.dest.is_empty() {
                    "OFF"
                } else {
                    &cfg.dest
                },
                cfg.copies,
                if cfg.hold { "HOLD " } else { "" },
                if cfg.continuous { "CONT" } else { "" },
            )])
        }
        SpoolCommand::SendFile { .. } => {
            SpoolCommandResult::error(24, "DMSSPL024E SENDFILE requires CMS filesystem bridging")
        }
        SpoolCommand::Receive { .. } => {
            SpoolCommandResult::error(24, "DMSSPL024E RECEIVE requires CMS filesystem bridging")
        }
        SpoolCommand::Purge { device, target } => match target {
            PurgeTarget::All => match manager.purge_all(*device, None) {
                Ok(count) => SpoolCommandResult::ok_with(vec![format!(
                    "{} file(s) purged from {}",
                    count, device
                )]),
                Err(e) => SpoolCommandResult::error(e.rc(), e.to_string()),
            },
            PurgeTarget::SpoolId(id) => match manager.purge_file(*device, *id) {
                Ok(()) => SpoolCommandResult::ok_with(vec![format!("File {} purged", id)]),
                Err(e) => SpoolCommandResult::error(e.rc(), e.to_string()),
            },
        },
        SpoolCommand::Query { device, class } => match manager.query_device(*device, *class) {
            Ok(files) => {
                if files.is_empty() {
                    SpoolCommandResult::ok_with(vec![format!("No files in {}", device)])
                } else {
                    let mut msgs = vec!["SPOOLID FILE     TYPE     CL  RECS HOLD DEST".to_string()];
                    for f in &files {
                        msgs.push(f.summary());
                    }
                    SpoolCommandResult::ok_with(msgs)
                }
            }
            Err(e) => SpoolCommandResult::error(e.rc(), e.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::InMemoryBackend;

    // -- parse tests --

    #[test]
    fn parse_spool_printer_class() {
        let cmd = parse_spool_command("SP PRT CLASS B").unwrap();
        match cmd {
            SpoolCommand::Spool {
                device,
                class,
                dest,
                ..
            } => {
                assert_eq!(device, SpoolDevice::Printer);
                assert_eq!(class, Some(SpoolClass('B')));
                assert!(dest.is_none());
            }
            _ => panic!("Expected Spool command"),
        }
    }

    #[test]
    fn parse_spool_reader_hold() {
        let cmd = parse_spool_command("SPOOL READER HOLD").unwrap();
        match cmd {
            SpoolCommand::Spool { device, hold, .. } => {
                assert_eq!(device, SpoolDevice::Reader);
                assert_eq!(hold, Some(true));
            }
            _ => panic!("Expected Spool command"),
        }
    }

    #[test]
    fn parse_spool_dest() {
        let cmd = parse_spool_command("SP PRT DEST OPERATOR").unwrap();
        match cmd {
            SpoolCommand::Spool { dest, .. } => {
                assert_eq!(dest, Some("OPERATOR".to_string()));
            }
            _ => panic!("Expected Spool command"),
        }
    }

    #[test]
    fn parse_spool_copy() {
        let cmd = parse_spool_command("SP PRT COPY 3").unwrap();
        match cmd {
            SpoolCommand::Spool { copies, .. } => {
                assert_eq!(copies, Some(3));
            }
            _ => panic!("Expected Spool command"),
        }
    }

    #[test]
    fn parse_sendfile_basic() {
        let cmd = parse_spool_command("SE MYFILE DATA A").unwrap();
        match cmd {
            SpoolCommand::SendFile {
                filename,
                filetype,
                filemode,
                dest_user,
            } => {
                assert_eq!(filename, "MYFILE");
                assert_eq!(filetype, "DATA");
                assert_eq!(filemode, Some('A'));
                assert!(dest_user.is_none());
            }
            _ => panic!("Expected SendFile command"),
        }
    }

    #[test]
    fn parse_sendfile_with_dest() {
        let cmd = parse_spool_command("SENDFILE MYFILE DATA A TO JONES").unwrap();
        match cmd {
            SpoolCommand::SendFile {
                filename,
                filetype,
                dest_user,
                ..
            } => {
                assert_eq!(filename, "MYFILE");
                assert_eq!(filetype, "DATA");
                assert_eq!(dest_user, Some("JONES".to_string()));
            }
            _ => panic!("Expected SendFile command"),
        }
    }

    #[test]
    fn parse_sendfile_no_mode() {
        let cmd = parse_spool_command("SE MYFILE DATA TO JONES").unwrap();
        match cmd {
            SpoolCommand::SendFile {
                filename,
                filetype,
                filemode,
                dest_user,
            } => {
                assert_eq!(filename, "MYFILE");
                assert_eq!(filetype, "DATA");
                assert!(filemode.is_none());
                assert_eq!(dest_user, Some("JONES".to_string()));
            }
            _ => panic!("Expected SendFile command"),
        }
    }

    #[test]
    fn parse_receive_bare() {
        let cmd = parse_spool_command("REC").unwrap();
        match cmd {
            SpoolCommand::Receive {
                filename,
                filetype,
                filemode,
            } => {
                assert!(filename.is_none());
                assert!(filetype.is_none());
                assert!(filemode.is_none());
            }
            _ => panic!("Expected Receive command"),
        }
    }

    #[test]
    fn parse_receive_with_rename() {
        let cmd = parse_spool_command("RECEIVE NEWNAME DATA A").unwrap();
        match cmd {
            SpoolCommand::Receive {
                filename,
                filetype,
                filemode,
            } => {
                assert_eq!(filename, Some("NEWNAME".to_string()));
                assert_eq!(filetype, Some("DATA".to_string()));
                assert_eq!(filemode, Some('A'));
            }
            _ => panic!("Expected Receive command"),
        }
    }

    #[test]
    fn parse_purge_all() {
        let cmd = parse_spool_command("PUR READER ALL").unwrap();
        match cmd {
            SpoolCommand::Purge { device, target } => {
                assert_eq!(device, SpoolDevice::Reader);
                assert_eq!(target, PurgeTarget::All);
            }
            _ => panic!("Expected Purge command"),
        }
    }

    #[test]
    fn parse_purge_id() {
        let cmd = parse_spool_command("PURGE READER 12345").unwrap();
        match cmd {
            SpoolCommand::Purge { device, target } => {
                assert_eq!(device, SpoolDevice::Reader);
                assert_eq!(target, PurgeTarget::SpoolId(12345));
            }
            _ => panic!("Expected Purge command"),
        }
    }

    #[test]
    fn parse_purge_bare_device_is_error() {
        // Bare "PUR PRT" without ALL or spool ID is invalid per CMS semantics
        assert!(parse_spool_command("PUR PRT").is_none());
    }

    #[test]
    fn parse_query_reader() {
        let cmd = parse_spool_command("Q R").unwrap();
        match cmd {
            SpoolCommand::Query { device, class } => {
                assert_eq!(device, SpoolDevice::Reader);
                assert!(class.is_none());
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn parse_query_printer_class() {
        let cmd = parse_spool_command("QUERY PRINTER CLASS B").unwrap();
        match cmd {
            SpoolCommand::Query { device, class } => {
                assert_eq!(device, SpoolDevice::Printer);
                assert_eq!(class, Some(SpoolClass('B')));
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert!(parse_spool_command("FOOBAR").is_none());
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(parse_spool_command("").is_none());
    }

    #[test]
    fn parse_spool_no_device_returns_none() {
        assert!(parse_spool_command("SP").is_none());
    }

    #[test]
    fn parse_sendfile_no_filetype_returns_none() {
        assert!(parse_spool_command("SE MYFILE").is_none());
    }

    // -- execute tests --

    fn make_manager() -> SpoolManager<InMemoryBackend> {
        SpoolManager::new(InMemoryBackend::new(), "TESTUSER")
    }

    #[test]
    fn execute_spool_configure() {
        let mut mgr = make_manager();
        let cmd = parse_spool_command("SP PRT CLASS B DEST OPERATOR COPY 3").unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 0);
        assert!(!result.messages.is_empty());

        let cfg = mgr.device_config(SpoolDevice::Printer);
        assert_eq!(cfg.class, SpoolClass('B'));
        assert_eq!(cfg.dest, "OPERATOR");
        assert_eq!(cfg.copies, 3);
    }

    #[test]
    fn execute_purge_all() {
        let mut mgr = make_manager();
        mgr.print_file("A", "B", "data").unwrap();
        mgr.print_file("C", "D", "data").unwrap();

        let cmd = parse_spool_command("PUR PRT ALL").unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 0);
        assert!(result.messages[0].contains("2"));
    }

    #[test]
    fn execute_purge_specific() {
        let mut mgr = make_manager();
        let id = mgr.print_file("A", "B", "data").unwrap();

        let cmd = parse_spool_command(&format!("PUR PRT {}", id)).unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 0);
    }

    #[test]
    fn execute_purge_not_found() {
        let mut mgr = make_manager();
        let cmd = parse_spool_command("PUR PRT 999").unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_ne!(result.rc, 0);
    }

    #[test]
    fn execute_query_empty() {
        let mut mgr = SpoolManager::new(InMemoryBackend::new(), "U");
        let cmd = parse_spool_command("Q R").unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 0);
        assert!(result.messages[0].contains("No files"));
    }

    #[test]
    fn execute_query_with_files() {
        let mut mgr = make_manager();
        mgr.send_file("FILE1", "DATA", "content\n", None).unwrap();

        let cmd = parse_spool_command("Q R").unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 0);
        assert!(result.messages.len() > 1); // header + file entry
    }

    #[test]
    fn execute_sendfile_returns_rc24() {
        let mut mgr = make_manager();
        let cmd = parse_spool_command("SE MYFILE DATA A").unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 24);
    }

    #[test]
    fn execute_receive_returns_rc24() {
        let mut mgr = make_manager();
        let cmd = parse_spool_command("REC").unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 24);
    }

    #[test]
    fn abbreviation_sp_works() {
        assert!(parse_spool_command("SP PRT CLASS A").is_some());
    }

    #[test]
    fn abbreviation_se_works() {
        assert!(parse_spool_command("SE FILE DATA").is_some());
    }

    #[test]
    fn abbreviation_rec_works() {
        assert!(parse_spool_command("REC").is_some());
    }

    #[test]
    fn abbreviation_pur_works() {
        assert!(parse_spool_command("PUR R ALL").is_some());
    }

    #[test]
    fn abbreviation_q_works() {
        assert!(parse_spool_command("Q R").is_some());
    }

    #[test]
    fn device_abbreviation_r_for_reader() {
        let cmd = parse_spool_command("Q R").unwrap();
        match cmd {
            SpoolCommand::Query { device, .. } => assert_eq!(device, SpoolDevice::Reader),
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn device_abbreviation_pr_for_printer() {
        let cmd = parse_spool_command("Q PR").unwrap();
        match cmd {
            SpoolCommand::Query { device, .. } => assert_eq!(device, SpoolDevice::Printer),
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn device_abbreviation_pu_for_punch() {
        let cmd = parse_spool_command("Q PU").unwrap();
        match cmd {
            SpoolCommand::Query { device, .. } => assert_eq!(device, SpoolDevice::Punch),
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn spool_nohold() {
        let cmd = parse_spool_command("SP PRT NOHOLD").unwrap();
        match cmd {
            SpoolCommand::Spool { hold, .. } => assert_eq!(hold, Some(false)),
            _ => panic!("Expected Spool"),
        }
    }

    #[test]
    fn spool_cont() {
        let cmd = parse_spool_command("SP PRT CONT").unwrap();
        match cmd {
            SpoolCommand::Spool { continuous, .. } => assert_eq!(continuous, Some(true)),
            _ => panic!("Expected Spool"),
        }
    }

    #[test]
    fn spool_unknown_option_rejected() {
        assert!(parse_spool_command("SP PRT BOGUSOPT VALUE").is_none());
        assert!(parse_spool_command("SP PRT COPIES 3").is_none());
    }

    #[test]
    fn spool_invalid_class_rejected() {
        assert!(parse_spool_command("SP PRT CLASS *").is_none());
        assert!(parse_spool_command("SP PRT CLASS 1").is_none());
        assert!(parse_spool_command("SP PRT CLASS AB").is_none());
        assert!(parse_spool_command("Q R CLASS AB").is_none());
        assert!(parse_spool_command("Q R CLASS *").is_none());
    }

    #[test]
    fn spool_dangling_keywords_rejected() {
        assert!(parse_spool_command("SP PRT CLASS").is_none());
        assert!(parse_spool_command("SP PRT DEST").is_none());
        assert!(parse_spool_command("SP PRT COPY").is_none());
    }

    #[test]
    fn receive_trailing_tokens_rejected() {
        assert!(parse_spool_command("REC FOO BAR A EXTRA").is_none());
    }

    #[test]
    fn receive_invalid_filemode_rejected() {
        assert!(parse_spool_command("REC FOO BAR 123").is_none());
    }

    #[test]
    fn sendfile_junk_tokens_rejected() {
        assert!(parse_spool_command("SE FILE DATA A JUNK TO JONES").is_none());
    }

    #[test]
    fn purge_trailing_tokens_rejected() {
        assert!(parse_spool_command("PUR R ALL EXTRA").is_none());
    }

    #[test]
    fn query_dangling_class_rejected() {
        assert!(parse_spool_command("Q R CLASS").is_none());
    }

    #[test]
    fn copy_zero_rejected() {
        assert!(parse_spool_command("SP PRT COPY 0").is_none());
    }

    #[test]
    fn copy_non_numeric_rejected() {
        assert!(parse_spool_command("SP PRT COPY abc").is_none());
    }

    #[test]
    fn sendfile_dangling_to_rejected() {
        assert!(parse_spool_command("SE MYFILE DATA TO").is_none());
    }

    #[test]
    fn spool_duplicate_options_rejected() {
        assert!(parse_spool_command("SP PRT CLASS A CLASS B").is_none());
        assert!(parse_spool_command("SP PRT HOLD HOLD").is_none());
        assert!(parse_spool_command("SP PRT HOLD NOHOLD").is_none());
    }

    // -- Phase 8e: edge case and polish tests --

    #[test]
    fn case_insensitive_commands() {
        assert!(parse_spool_command("sp prt class a").is_some());
        assert!(parse_spool_command("Spool Printer Class B").is_some());
        assert!(parse_spool_command("QUERY READER").is_some());
    }

    #[test]
    fn device_alias_rdr() {
        let cmd = parse_spool_command("Q RDR").unwrap();
        match cmd {
            SpoolCommand::Query { device, .. } => assert_eq!(device, SpoolDevice::Reader),
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn device_alias_pun() {
        let cmd = parse_spool_command("Q PUN").unwrap();
        match cmd {
            SpoolCommand::Query { device, .. } => assert_eq!(device, SpoolDevice::Punch),
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn device_alias_pch() {
        let cmd = parse_spool_command("Q PCH").unwrap();
        match cmd {
            SpoolCommand::Query { device, .. } => assert_eq!(device, SpoolDevice::Punch),
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn spool_multiple_options() {
        let cmd = parse_spool_command("SP PRT CLASS B DEST OPER HOLD CONT COPY 5").unwrap();
        match cmd {
            SpoolCommand::Spool {
                device,
                class,
                dest,
                hold,
                continuous,
                copies,
            } => {
                assert_eq!(device, SpoolDevice::Printer);
                assert_eq!(class, Some(SpoolClass('B')));
                assert_eq!(dest, Some("OPER".to_string()));
                assert_eq!(hold, Some(true));
                assert_eq!(continuous, Some(true));
                assert_eq!(copies, Some(5));
            }
            _ => panic!("Expected Spool"),
        }
    }

    #[test]
    fn purge_all_with_query_verification() {
        let mut mgr = make_manager();
        mgr.send_file("A", "B", "d", None).unwrap();
        mgr.send_file("C", "D", "d", None).unwrap();

        let before = mgr.query_device(SpoolDevice::Reader, None).unwrap();
        assert_eq!(before.len(), 2);

        let cmd = parse_spool_command("PUR R ALL").unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 0);
        assert!(result.messages[0].contains("2"));

        let after = mgr.query_device(SpoolDevice::Reader, None).unwrap();
        assert!(after.is_empty());
    }

    #[test]
    fn query_empty_shows_message() {
        let mut mgr = make_manager();
        let cmd = parse_spool_command("Q PRT").unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 0);
        assert!(result.messages[0].contains("No files"));
    }

    #[test]
    fn query_with_files_shows_header_and_entries() {
        let mut mgr = make_manager();
        mgr.send_file("TEST", "FILE", "line1\nline2\nline3\n", None)
            .unwrap();

        let cmd = parse_spool_command("Q R").unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 0);
        assert!(result.messages.len() >= 2); // header + at least one entry
        assert!(result.messages[0].contains("SPOOLID"));
        assert!(result.messages[1].contains("TEST"));
    }

    #[test]
    fn spool_configure_display_message() {
        let mut mgr = make_manager();
        let cmd = parse_spool_command("SP PRT CLASS B DEST OPER").unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 0);
        assert!(result.messages[0].contains("PRINTER"));
        assert!(result.messages[0].contains("B"));
        assert!(result.messages[0].contains("OPER"));
    }

    #[test]
    fn sendfile_lowercase_normalized() {
        let cmd = parse_spool_command("se myfile data a to jones").unwrap();
        match cmd {
            SpoolCommand::SendFile {
                filename,
                filetype,
                dest_user,
                ..
            } => {
                assert_eq!(filename, "MYFILE");
                assert_eq!(filetype, "DATA");
                assert_eq!(dest_user, Some("JONES".to_string()));
            }
            _ => panic!("Expected SendFile"),
        }
    }

    #[test]
    fn receive_with_partial_rename() {
        // Just filename, no filetype
        let cmd = parse_spool_command("REC NEWNAME").unwrap();
        match cmd {
            SpoolCommand::Receive {
                filename,
                filetype,
                filemode,
            } => {
                assert_eq!(filename, Some("NEWNAME".to_string()));
                assert!(filetype.is_none());
                assert!(filemode.is_none());
            }
            _ => panic!("Expected Receive"),
        }
    }

    #[test]
    fn purge_reader_with_id() {
        let mut mgr = make_manager();
        let id1 = mgr.send_file("A", "B", "d1", None).unwrap();
        let _id2 = mgr.send_file("C", "D", "d2", None).unwrap();

        let cmd = parse_spool_command(&format!("PUR R {}", id1)).unwrap();
        let result = execute_spool_command(&cmd, &mut mgr);
        assert_eq!(result.rc, 0);

        let remaining = mgr.query_device(SpoolDevice::Reader, None).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].filename, "C");
    }
}
