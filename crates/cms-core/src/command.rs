use crate::error::CmsError;
use crate::filespec::FileSpec;
use crate::filesystem::CmsFileSystem;
use crate::globalv::GlobalVars;
use crate::minidisk::AccessMode;

// ---------------------------------------------------------------------------
// ExecHandler trait — decouples cms-core from patch-rexx
// ---------------------------------------------------------------------------

/// Trait for executing REXX EXEC files. Consumers provide their own implementation
/// that wires up to `patch-rexx` or another interpreter.
pub trait ExecHandler: Send {
    /// Execute a REXX source string with the given argument string.
    /// Returns `(return_code, output_messages)`.
    fn execute_exec(&mut self, source: &str, args: &str) -> (i32, Vec<String>);
}

/// Default handler when no REXX interpreter is available — always returns RC=28.
pub struct NoExecHandler;

impl ExecHandler for NoExecHandler {
    fn execute_exec(&mut self, _source: &str, _args: &str) -> (i32, Vec<String>) {
        (28, vec!["EXEC handler not available".to_string()])
    }
}

// ---------------------------------------------------------------------------
// SmsgSender trait — decouples cms-core from vm-iucv messaging
// ---------------------------------------------------------------------------

/// Trait for sending SMSG messages. Consumers provide their own implementation
/// that wires up to the vm-iucv actor framework.
pub trait SmsgSender: Send {
    fn send_smsg(&self, target: &str, text: &str) -> (i32, String);
}

/// Default sender when no SMSG facility is available — always returns RC=28.
pub struct NoSmsgSender;

impl SmsgSender for NoSmsgSender {
    fn send_smsg(&self, _target: &str, _text: &str) -> (i32, String) {
        (28, "SMSG facility not available".to_string())
    }
}

// ---------------------------------------------------------------------------
// CmsCommand — parsed command representation
// ---------------------------------------------------------------------------

/// A parsed CMS command ready for dispatch.
#[derive(Debug)]
pub enum CmsCommand {
    Listfile(FileSpec),
    State(FileSpec),
    Copyfile { from: FileSpec, to: FileSpec },
    Erase(FileSpec),
    Rename { from: FileSpec, to: FileSpec },
    Globalv(GlobalvSubcommand),
    Access { path: String, mode: String },
    Release(char),
    Exec { name: String, args: String },
    Smsg { userid: String, text: String },
}

/// Sub-commands for GLOBALV.
#[derive(Debug)]
pub enum GlobalvSubcommand {
    Select(String),
    Set { name: String, value: String },
    Get(Vec<String>),
    List(Option<String>),
    Delete(String),
    Purge,
}

// ---------------------------------------------------------------------------
// CmsCommandResult
// ---------------------------------------------------------------------------

/// Result of executing a CMS command.
pub struct CmsCommandResult {
    pub rc: i32,
    pub messages: Vec<String>,
}

impl CmsCommandResult {
    fn ok() -> Self {
        CmsCommandResult {
            rc: 0,
            messages: Vec::new(),
        }
    }

    fn ok_with(messages: Vec<String>) -> Self {
        CmsCommandResult { rc: 0, messages }
    }

    fn error(rc: i32, msg: impl Into<String>) -> Self {
        CmsCommandResult {
            rc,
            messages: vec![msg.into()],
        }
    }
}

// ---------------------------------------------------------------------------
// Abbreviation table + parsing
// ---------------------------------------------------------------------------

/// Each entry: (full_name, minimum_abbreviation_length).
/// Follows xedit-core's pattern and IBM CMS abbreviation conventions.
const CMS_COMMAND_TABLE: &[(&str, usize)] = &[
    ("ACCESS", 2),   // AC
    ("COPYFILE", 4), // COPY
    ("ERASE", 2),    // ER
    ("EXEC", 4),     // EXEC
    ("GLOBALV", 4),  // GLOB
    ("LISTFILE", 4), // LIST
    ("RELEASE", 3),  // REL
    ("RENAME", 3),   // REN
    ("SMSG", 2),     // SM
    ("STATE", 2),    // ST
];

/// Look up a command word against the abbreviation table.
fn lookup_command(input: &str) -> Option<&'static str> {
    let input_upper = input.to_ascii_uppercase();
    // Exact match first
    for &(name, _) in CMS_COMMAND_TABLE {
        if name == input_upper {
            return Some(name);
        }
    }
    // Abbreviation match
    let mut matches = Vec::new();
    for &(name, min_abbrev) in CMS_COMMAND_TABLE {
        if input_upper.len() >= min_abbrev && name.starts_with(&input_upper) {
            matches.push(name);
        }
    }
    if matches.len() == 1 {
        return Some(matches[0]);
    }
    None
}

/// Parse a raw command line into a `CmsCommand`.
pub fn parse_cms_command(input: &str) -> Result<CmsCommand, CmsError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CmsError::InvalidCommand("Empty command".to_string()));
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let cmd_word = parts.next().unwrap();
    let rest = parts.next().unwrap_or("").trim();

    let cmd_name = lookup_command(cmd_word)
        .ok_or_else(|| CmsError::UnknownCommand(cmd_word.to_ascii_uppercase()))?;

    match cmd_name {
        "LISTFILE" => parse_listfile(rest),
        "STATE" => parse_state(rest),
        "COPYFILE" => parse_copyfile(rest),
        "ERASE" => parse_erase(rest),
        "RENAME" => parse_rename(rest),
        "SMSG" => parse_smsg(rest),
        "GLOBALV" => parse_globalv(rest),
        "ACCESS" => parse_access(rest),
        "RELEASE" => parse_release(rest),
        "EXEC" => parse_exec(rest),
        _ => Err(CmsError::UnknownCommand(cmd_name.to_string())),
    }
}

fn parse_listfile(rest: &str) -> Result<CmsCommand, CmsError> {
    if rest.is_empty() {
        return Err(CmsError::InvalidCommand(
            "LISTFILE requires a file specification".to_string(),
        ));
    }
    let spec = FileSpec::parse(rest)?;
    Ok(CmsCommand::Listfile(spec))
}

fn parse_state(rest: &str) -> Result<CmsCommand, CmsError> {
    if rest.is_empty() {
        return Err(CmsError::InvalidCommand(
            "STATE requires a file specification".to_string(),
        ));
    }
    let spec = FileSpec::parse(rest)?;
    Ok(CmsCommand::State(spec))
}

fn parse_copyfile(rest: &str) -> Result<CmsCommand, CmsError> {
    // COPYFILE fn1 ft1 fm1 fn2 ft2 fm2
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.len() < 4 {
        return Err(CmsError::InvalidCommand(
            "COPYFILE requires source and destination file specifications".to_string(),
        ));
    }
    // Try 3+3 split first, then 3+2, then 2+2
    let (from, to) = if tokens.len() >= 6 {
        let from_str = format!("{} {} {}", tokens[0], tokens[1], tokens[2]);
        let to_str = format!("{} {} {}", tokens[3], tokens[4], tokens[5]);
        (FileSpec::parse(&from_str)?, FileSpec::parse(&to_str)?)
    } else if tokens.len() == 5 {
        let from_str = format!("{} {} {}", tokens[0], tokens[1], tokens[2]);
        let to_str = format!("{} {}", tokens[3], tokens[4]);
        (FileSpec::parse(&from_str)?, FileSpec::parse(&to_str)?)
    } else {
        // 4 tokens: fn1 ft1 fn2 ft2 (both default filemode)
        let from_str = format!("{} {}", tokens[0], tokens[1]);
        let to_str = format!("{} {}", tokens[2], tokens[3]);
        (FileSpec::parse(&from_str)?, FileSpec::parse(&to_str)?)
    };
    Ok(CmsCommand::Copyfile { from, to })
}

fn parse_erase(rest: &str) -> Result<CmsCommand, CmsError> {
    if rest.is_empty() {
        return Err(CmsError::InvalidCommand(
            "ERASE requires a file specification".to_string(),
        ));
    }
    let spec = FileSpec::parse(rest)?;
    Ok(CmsCommand::Erase(spec))
}

fn parse_rename(rest: &str) -> Result<CmsCommand, CmsError> {
    // RENAME fn1 ft1 fm fn2 ft2
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.len() < 4 {
        return Err(CmsError::InvalidCommand(
            "RENAME requires source and destination file specifications".to_string(),
        ));
    }
    let (from, to) = if tokens.len() >= 5 {
        let from_str = format!("{} {} {}", tokens[0], tokens[1], tokens[2]);
        let to_str = if tokens.len() >= 6 {
            format!("{} {} {}", tokens[3], tokens[4], tokens[5])
        } else {
            format!("{} {}", tokens[3], tokens[4])
        };
        (FileSpec::parse(&from_str)?, FileSpec::parse(&to_str)?)
    } else {
        // 4 tokens: fn1 ft1 fn2 ft2 (both default filemode)
        let from_str = format!("{} {}", tokens[0], tokens[1]);
        let to_str = format!("{} {}", tokens[2], tokens[3]);
        (FileSpec::parse(&from_str)?, FileSpec::parse(&to_str)?)
    };
    Ok(CmsCommand::Rename { from, to })
}

fn parse_smsg(rest: &str) -> Result<CmsCommand, CmsError> {
    let mut parts = rest.splitn(2, char::is_whitespace);
    let userid = match parts.next() {
        Some(u) if !u.is_empty() => u.to_ascii_uppercase(),
        _ => {
            return Err(CmsError::InvalidCommand(
                "SMSG requires a userid and text".to_string(),
            ))
        }
    };
    let text = parts.next().unwrap_or("").trim().to_string();
    if text.is_empty() {
        return Err(CmsError::InvalidCommand(
            "SMSG requires message text".to_string(),
        ));
    }
    Ok(CmsCommand::Smsg { userid, text })
}

fn parse_globalv(rest: &str) -> Result<CmsCommand, CmsError> {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.is_empty() {
        return Err(CmsError::InvalidCommand(
            "GLOBALV requires a subcommand".to_string(),
        ));
    }
    let sub = tokens[0].to_ascii_uppercase();
    match sub.as_str() {
        "SELECT" => {
            if tokens.len() < 2 {
                return Err(CmsError::InvalidCommand(
                    "GLOBALV SELECT requires a group name".to_string(),
                ));
            }
            Ok(CmsCommand::Globalv(GlobalvSubcommand::Select(
                tokens[1].to_string(),
            )))
        }
        "SET" => {
            if tokens.len() < 3 {
                return Err(CmsError::InvalidCommand(
                    "GLOBALV SET requires a name and value".to_string(),
                ));
            }
            // Value is everything after the variable name
            let name = tokens[1].to_string();
            let value = tokens[2..].join(" ");
            Ok(CmsCommand::Globalv(GlobalvSubcommand::Set { name, value }))
        }
        "GET" => {
            if tokens.len() < 2 {
                return Err(CmsError::InvalidCommand(
                    "GLOBALV GET requires a variable name".to_string(),
                ));
            }
            let names: Vec<String> = tokens[1..].iter().map(|s| s.to_string()).collect();
            Ok(CmsCommand::Globalv(GlobalvSubcommand::Get(names)))
        }
        "LIST" => {
            let group = if tokens.len() >= 2 {
                Some(tokens[1].to_string())
            } else {
                None
            };
            Ok(CmsCommand::Globalv(GlobalvSubcommand::List(group)))
        }
        "DELETE" => {
            if tokens.len() < 2 {
                return Err(CmsError::InvalidCommand(
                    "GLOBALV DELETE requires a variable name".to_string(),
                ));
            }
            Ok(CmsCommand::Globalv(GlobalvSubcommand::Delete(
                tokens[1].to_string(),
            )))
        }
        "PURGE" => Ok(CmsCommand::Globalv(GlobalvSubcommand::Purge)),
        _ => Err(CmsError::InvalidCommand(format!(
            "Unknown GLOBALV subcommand: {}",
            sub
        ))),
    }
}

fn parse_access(rest: &str) -> Result<CmsCommand, CmsError> {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.len() < 2 {
        return Err(CmsError::InvalidCommand(
            "ACCESS requires a path and filemode".to_string(),
        ));
    }
    Ok(CmsCommand::Access {
        path: tokens[0].to_string(),
        mode: tokens[1].to_string(),
    })
}

fn parse_release(rest: &str) -> Result<CmsCommand, CmsError> {
    let token = rest.trim();
    if token.is_empty() || token.len() != 1 {
        return Err(CmsError::InvalidCommand(
            "RELEASE requires a single disk letter".to_string(),
        ));
    }
    let letter = token.chars().next().unwrap().to_ascii_uppercase();
    if !letter.is_ascii_alphabetic() {
        return Err(CmsError::InvalidCommand(format!(
            "RELEASE: invalid disk letter '{}'",
            letter
        )));
    }
    Ok(CmsCommand::Release(letter))
}

fn parse_exec(rest: &str) -> Result<CmsCommand, CmsError> {
    if rest.is_empty() {
        return Err(CmsError::InvalidCommand(
            "EXEC requires a program name".to_string(),
        ));
    }
    let mut parts = rest.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap().to_ascii_uppercase();
    let args = parts.next().unwrap_or("").trim().to_string();
    Ok(CmsCommand::Exec { name, args })
}

// ---------------------------------------------------------------------------
// CommandProcessor
// ---------------------------------------------------------------------------

/// CMS command processor: parses and dispatches commands, resolves EXECs on disk.
pub struct CommandProcessor {
    filesystem: CmsFileSystem,
    globalv: GlobalVars,
    exec_handler: Box<dyn ExecHandler>,
    smsg_sender: Box<dyn SmsgSender>,
}

impl CommandProcessor {
    /// Create a new command processor with no EXEC handler (NoExecHandler).
    pub fn new(filesystem: CmsFileSystem) -> Self {
        CommandProcessor {
            filesystem,
            globalv: GlobalVars::new(),
            exec_handler: Box::new(NoExecHandler),
            smsg_sender: Box::new(NoSmsgSender),
        }
    }

    /// Create a new command processor with a custom EXEC handler.
    pub fn with_exec_handler(filesystem: CmsFileSystem, handler: Box<dyn ExecHandler>) -> Self {
        CommandProcessor {
            filesystem,
            globalv: GlobalVars::new(),
            exec_handler: handler,
            smsg_sender: Box::new(NoSmsgSender),
        }
    }

    /// Create a new command processor with both an EXEC handler and SMSG sender.
    pub fn with_smsg_sender(
        filesystem: CmsFileSystem,
        handler: Box<dyn ExecHandler>,
        sender: Box<dyn SmsgSender>,
    ) -> Self {
        CommandProcessor {
            filesystem,
            globalv: GlobalVars::new(),
            exec_handler: handler,
            smsg_sender: sender,
        }
    }

    pub fn filesystem(&self) -> &CmsFileSystem {
        &self.filesystem
    }

    pub fn filesystem_mut(&mut self) -> &mut CmsFileSystem {
        &mut self.filesystem
    }

    pub fn globalv(&self) -> &GlobalVars {
        &self.globalv
    }

    pub fn globalv_mut(&mut self) -> &mut GlobalVars {
        &mut self.globalv
    }

    /// Temporarily take the exec handler, replacing it with `NoExecHandler`.
    pub fn take_exec_handler(&mut self) -> Box<dyn ExecHandler> {
        std::mem::replace(&mut self.exec_handler, Box::new(NoExecHandler))
    }

    /// Set a new exec handler, returning the old one.
    pub fn set_exec_handler(&mut self, handler: Box<dyn ExecHandler>) -> Box<dyn ExecHandler> {
        std::mem::replace(&mut self.exec_handler, handler)
    }

    /// Temporarily take the SMSG sender, replacing it with `NoSmsgSender`.
    pub fn take_smsg_sender(&mut self) -> Box<dyn SmsgSender> {
        std::mem::replace(&mut self.smsg_sender, Box::new(NoSmsgSender))
    }

    /// Set a new SMSG sender, returning the old one.
    pub fn set_smsg_sender(&mut self, sender: Box<dyn SmsgSender>) -> Box<dyn SmsgSender> {
        std::mem::replace(&mut self.smsg_sender, sender)
    }

    /// Main entry point: parse and execute a command line.
    ///
    /// 1. Try to parse as a built-in command and dispatch.
    /// 2. On `UnknownCommand`, search disks for `name EXEC *` and run via ExecHandler.
    /// 3. Not found anywhere → RC=3.
    pub fn execute(&mut self, input: &str) -> CmsCommandResult {
        match parse_cms_command(input) {
            Ok(cmd) => self.dispatch(cmd),
            Err(CmsError::UnknownCommand(ref cmd_name)) => self.try_exec_fallback(cmd_name, input),
            Err(e) => CmsCommandResult::error(24, e.to_string()),
        }
    }

    /// Search for PROFILE EXEC on all disks, execute if found.
    /// Returns the EXEC output if found and run; None if not present.
    pub fn run_profile(&mut self) -> Option<CmsCommandResult> {
        let spec = FileSpec::parse("PROFILE EXEC *").ok()?;
        match self.filesystem.read_file(&spec) {
            Ok(source) => {
                let (rc, messages) = self.exec_handler.execute_exec(&source, "");
                Some(CmsCommandResult { rc, messages })
            }
            Err(CmsError::FileNotFound(_)) | Err(CmsError::DiskNotAccessed(_)) => None,
            Err(e) => Some(CmsCommandResult::error(28, e.to_string())),
        }
    }

    // --- dispatch built-in commands ---

    fn dispatch(&mut self, cmd: CmsCommand) -> CmsCommandResult {
        match cmd {
            CmsCommand::Listfile(spec) => self.cmd_listfile(spec),
            CmsCommand::State(spec) => self.cmd_state(spec),
            CmsCommand::Copyfile { from, to } => self.cmd_copyfile(from, to),
            CmsCommand::Erase(spec) => self.cmd_erase(spec),
            CmsCommand::Rename { from, to } => self.cmd_rename(from, to),
            CmsCommand::Globalv(sub) => self.cmd_globalv(sub),
            CmsCommand::Access { path, mode } => self.cmd_access(path, mode),
            CmsCommand::Release(letter) => self.cmd_release(letter),
            CmsCommand::Exec { name, args } => self.cmd_exec(name, args),
            CmsCommand::Smsg { userid, text } => self.cmd_smsg(userid, text),
        }
    }

    fn cmd_listfile(&self, spec: FileSpec) -> CmsCommandResult {
        match self.filesystem.listfile(&spec) {
            Ok(files) => {
                if files.is_empty() {
                    CmsCommandResult::error(28, format!("No files match {}", spec))
                } else {
                    let messages: Vec<String> = files
                        .iter()
                        .map(|f| {
                            format!(
                                "{} {} {}  {} lines",
                                f.spec.filename(),
                                f.spec.filetype(),
                                f.spec.filemode(),
                                f.line_count,
                            )
                        })
                        .collect();
                    CmsCommandResult::ok_with(messages)
                }
            }
            Err(e) => CmsCommandResult::error(28, e.to_string()),
        }
    }

    fn cmd_state(&self, spec: FileSpec) -> CmsCommandResult {
        match self.filesystem.state(&spec) {
            Ok(info) => CmsCommandResult::ok_with(vec![format!(
                "{} {} {}",
                info.spec.filename(),
                info.spec.filetype(),
                info.spec.filemode(),
            )]),
            Err(CmsError::FileNotFound(_)) => {
                CmsCommandResult::error(28, format!("File not found: {}", spec))
            }
            Err(e) => CmsCommandResult::error(20, e.to_string()),
        }
    }

    fn cmd_copyfile(&self, from: FileSpec, to: FileSpec) -> CmsCommandResult {
        match self.filesystem.copyfile(&from, &to) {
            Ok(()) => CmsCommandResult::ok(),
            Err(e) => CmsCommandResult::error(28, e.to_string()),
        }
    }

    fn cmd_erase(&self, spec: FileSpec) -> CmsCommandResult {
        match self.filesystem.erase(&spec) {
            Ok(()) => CmsCommandResult::ok(),
            Err(CmsError::FileNotFound(_)) => {
                CmsCommandResult::error(28, format!("File not found: {}", spec))
            }
            Err(e) => CmsCommandResult::error(28, e.to_string()),
        }
    }

    fn cmd_rename(&self, from: FileSpec, to: FileSpec) -> CmsCommandResult {
        match self.filesystem.rename(&from, &to) {
            Ok(()) => CmsCommandResult::ok(),
            Err(e) => CmsCommandResult::error(28, e.to_string()),
        }
    }

    fn cmd_globalv(&mut self, sub: GlobalvSubcommand) -> CmsCommandResult {
        match sub {
            GlobalvSubcommand::Select(group) => {
                self.globalv.select(&group);
                CmsCommandResult::ok()
            }
            GlobalvSubcommand::Set { name, value } => {
                self.globalv.set(&name, &value);
                CmsCommandResult::ok()
            }
            GlobalvSubcommand::Get(names) => {
                let mut messages = Vec::new();
                for name in &names {
                    match self.globalv.get(name) {
                        Some(val) => messages.push(val.to_string()),
                        None => {
                            return CmsCommandResult::error(
                                4,
                                format!("Variable {} not found", name),
                            );
                        }
                    }
                }
                CmsCommandResult::ok_with(messages)
            }
            GlobalvSubcommand::List(group) => {
                let items = match group {
                    Some(ref g) => self.globalv.list_group(g),
                    None => self.globalv.list(),
                };
                let messages: Vec<String> = items
                    .iter()
                    .map(|(k, v)| format!("{} = {}", k, v))
                    .collect();
                CmsCommandResult::ok_with(messages)
            }
            GlobalvSubcommand::Delete(name) => {
                if self.globalv.delete(&name) {
                    CmsCommandResult::ok()
                } else {
                    CmsCommandResult::error(4, format!("Variable {} not found", name))
                }
            }
            GlobalvSubcommand::Purge => {
                self.globalv.purge();
                CmsCommandResult::ok()
            }
        }
    }

    fn cmd_access(&mut self, path: String, mode: String) -> CmsCommandResult {
        let mode_upper = mode.to_ascii_uppercase();
        if mode_upper.is_empty() {
            return CmsCommandResult::error(24, "ACCESS requires a filemode");
        }
        let letter = mode_upper.chars().next().unwrap();
        if !letter.is_ascii_alphabetic() {
            return CmsCommandResult::error(24, format!("Invalid filemode letter '{}'", letter));
        }
        // Determine access mode from optional digit
        let access = if mode_upper.len() > 1 {
            let digit = mode_upper.chars().nth(1).unwrap_or('1');
            match digit.to_digit(10) {
                Some(d) => AccessMode::from_digit(d as u8),
                None => {
                    return CmsCommandResult::error(
                        24,
                        format!("Invalid filemode digit '{}'", digit),
                    );
                }
            }
        } else {
            AccessMode::ReadWrite
        };

        match self.filesystem.access_disk(letter, &path, access) {
            Ok(()) => CmsCommandResult::ok(),
            Err(e) => CmsCommandResult::error(28, e.to_string()),
        }
    }

    fn cmd_release(&mut self, letter: char) -> CmsCommandResult {
        self.filesystem.release_disk(letter);
        CmsCommandResult::ok()
    }

    fn cmd_smsg(&self, userid: String, text: String) -> CmsCommandResult {
        let (rc, msg) = self.smsg_sender.send_smsg(&userid, &text);
        if rc == 0 {
            CmsCommandResult::ok()
        } else {
            CmsCommandResult::error(rc, msg)
        }
    }

    fn cmd_exec(&mut self, name: String, args: String) -> CmsCommandResult {
        // Search for name EXEC on all disks
        let spec_str = format!("{} EXEC *", name);
        let spec = match FileSpec::parse(&spec_str) {
            Ok(s) => s,
            Err(e) => return CmsCommandResult::error(28, e.to_string()),
        };
        match self.filesystem.read_file(&spec) {
            Ok(source) => {
                let (rc, messages) = self.exec_handler.execute_exec(&source, &args);
                CmsCommandResult { rc, messages }
            }
            Err(CmsError::FileNotFound(_)) => {
                CmsCommandResult::error(3, format!("EXEC {} not found", name))
            }
            Err(e) => CmsCommandResult::error(28, e.to_string()),
        }
    }

    // --- EXEC fallback for unknown commands ---

    fn try_exec_fallback(&mut self, cmd_name: &str, input: &str) -> CmsCommandResult {
        let args = input
            .trim()
            .split_once(char::is_whitespace)
            .map(|x| x.1)
            .unwrap_or("")
            .trim();

        let spec_str = format!("{} EXEC *", cmd_name);
        let spec = match FileSpec::parse(&spec_str) {
            Ok(s) => s,
            Err(_) => {
                return CmsCommandResult::error(3, format!("Unknown CP/CMS command: {}", cmd_name));
            }
        };

        match self.filesystem.read_file(&spec) {
            Ok(source) => {
                let (rc, messages) = self.exec_handler.execute_exec(&source, args);
                CmsCommandResult { rc, messages }
            }
            Err(CmsError::FileNotFound(_)) | Err(CmsError::DiskNotAccessed(_)) => {
                CmsCommandResult::error(3, format!("Unknown CP/CMS command: {}", cmd_name))
            }
            Err(e) => CmsCommandResult::error(28, e.to_string()),
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -- parsing tests --

    #[test]
    fn parse_listfile() {
        let cmd = parse_cms_command("LISTFILE * EXEC A").unwrap();
        assert!(matches!(cmd, CmsCommand::Listfile(_)));
    }

    #[test]
    fn parse_listfile_abbreviated() {
        let cmd = parse_cms_command("list * EXEC A").unwrap();
        assert!(matches!(cmd, CmsCommand::Listfile(_)));
    }

    #[test]
    fn parse_state() {
        let cmd = parse_cms_command("STATE MYFILE DATA A").unwrap();
        assert!(matches!(cmd, CmsCommand::State(_)));
    }

    #[test]
    fn parse_state_abbreviated() {
        let cmd = parse_cms_command("st MYFILE DATA A").unwrap();
        assert!(matches!(cmd, CmsCommand::State(_)));
    }

    #[test]
    fn parse_erase() {
        let cmd = parse_cms_command("ERASE MYFILE DATA A").unwrap();
        assert!(matches!(cmd, CmsCommand::Erase(_)));
    }

    #[test]
    fn parse_erase_abbreviated() {
        let cmd = parse_cms_command("er MYFILE DATA A").unwrap();
        assert!(matches!(cmd, CmsCommand::Erase(_)));
    }

    #[test]
    fn parse_copyfile() {
        let cmd = parse_cms_command("COPYFILE FILE1 DATA A FILE2 DATA B").unwrap();
        assert!(matches!(cmd, CmsCommand::Copyfile { .. }));
    }

    #[test]
    fn parse_rename() {
        let cmd = parse_cms_command("RENAME OLD DATA A NEW DATA A").unwrap();
        assert!(matches!(cmd, CmsCommand::Rename { .. }));
    }

    #[test]
    fn parse_globalv_set() {
        let cmd = parse_cms_command("GLOBALV SET MYVAR hello world").unwrap();
        match cmd {
            CmsCommand::Globalv(GlobalvSubcommand::Set { name, value }) => {
                assert_eq!(name, "MYVAR");
                assert_eq!(value, "hello world");
            }
            _ => panic!("Expected GLOBALV SET"),
        }
    }

    #[test]
    fn parse_globalv_abbreviated() {
        let cmd = parse_cms_command("glob SET X 1").unwrap();
        assert!(matches!(
            cmd,
            CmsCommand::Globalv(GlobalvSubcommand::Set { .. })
        ));
    }

    #[test]
    fn parse_access() {
        let cmd = parse_cms_command("ACCESS /tmp/mydata B2").unwrap();
        match cmd {
            CmsCommand::Access { path, mode } => {
                assert_eq!(path, "/tmp/mydata");
                assert_eq!(mode, "B2");
            }
            _ => panic!("Expected ACCESS"),
        }
    }

    #[test]
    fn parse_release() {
        let cmd = parse_cms_command("RELEASE B").unwrap();
        assert!(matches!(cmd, CmsCommand::Release('B')));
    }

    #[test]
    fn parse_exec() {
        let cmd = parse_cms_command("EXEC MYPROG arg1 arg2").unwrap();
        match cmd {
            CmsCommand::Exec { name, args } => {
                assert_eq!(name, "MYPROG");
                assert_eq!(args, "arg1 arg2");
            }
            _ => panic!("Expected EXEC"),
        }
    }

    #[test]
    fn parse_unknown_command() {
        let err = parse_cms_command("BOGUS stuff").unwrap_err();
        assert!(matches!(err, CmsError::UnknownCommand(_)));
    }

    #[test]
    fn parse_empty_input() {
        let err = parse_cms_command("").unwrap_err();
        assert!(matches!(err, CmsError::InvalidCommand(_)));
    }

    #[test]
    fn parse_ambiguous_abbreviation() {
        // "S" could match STATE, so min-abbrev is 2 for STATE
        let err = parse_cms_command("S MYFILE DATA A").unwrap_err();
        assert!(matches!(err, CmsError::UnknownCommand(_)));
    }

    // -- execution tests with tempfile disks --

    fn setup_processor() -> (TempDir, CommandProcessor) {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        (dir, CommandProcessor::new(fs))
    }

    #[test]
    fn execute_state_found() {
        let (_dir, mut proc) = setup_processor();
        let spec = FileSpec::parse("MYFILE DATA A").unwrap();
        proc.filesystem.write_file(&spec, "content").unwrap();
        let result = proc.execute("STATE MYFILE DATA A");
        assert_eq!(result.rc, 0);
        assert!(!result.messages.is_empty());
    }

    #[test]
    fn execute_state_not_found() {
        let (_dir, mut proc) = setup_processor();
        let result = proc.execute("STATE NOFILE DATA A");
        assert_eq!(result.rc, 28);
    }

    #[test]
    fn execute_listfile() {
        let (_dir, mut proc) = setup_processor();
        proc.filesystem
            .write_file(&FileSpec::parse("F1 EXEC A").unwrap(), "a")
            .unwrap();
        proc.filesystem
            .write_file(&FileSpec::parse("F2 EXEC A").unwrap(), "b")
            .unwrap();
        let result = proc.execute("LISTFILE * EXEC A");
        assert_eq!(result.rc, 0);
        assert_eq!(result.messages.len(), 2);
    }

    #[test]
    fn execute_listfile_empty() {
        let (_dir, mut proc) = setup_processor();
        let result = proc.execute("LISTFILE * EXEC A");
        assert_eq!(result.rc, 28);
    }

    #[test]
    fn execute_erase() {
        let (_dir, mut proc) = setup_processor();
        proc.filesystem
            .write_file(&FileSpec::parse("GONE DATA A").unwrap(), "bye")
            .unwrap();
        let result = proc.execute("ERASE GONE DATA A");
        assert_eq!(result.rc, 0);
        // Verify gone
        let result2 = proc.execute("STATE GONE DATA A");
        assert_eq!(result2.rc, 28);
    }

    #[test]
    fn execute_copyfile() {
        let (_dir, mut proc) = setup_processor();
        proc.filesystem
            .write_file(&FileSpec::parse("SRC DATA A").unwrap(), "data")
            .unwrap();
        let result = proc.execute("COPYFILE SRC DATA A DST DATA A");
        assert_eq!(result.rc, 0);
        let content = proc
            .filesystem
            .read_file(&FileSpec::parse("DST DATA A").unwrap())
            .unwrap();
        assert_eq!(content, "data");
    }

    #[test]
    fn execute_rename() {
        let (_dir, mut proc) = setup_processor();
        proc.filesystem
            .write_file(&FileSpec::parse("OLD DATA A").unwrap(), "stuff")
            .unwrap();
        let result = proc.execute("RENAME OLD DATA A NEW DATA A");
        assert_eq!(result.rc, 0);
        assert!(proc
            .filesystem
            .read_file(&FileSpec::parse("NEW DATA A").unwrap())
            .is_ok());
    }

    #[test]
    fn execute_globalv_set_get() {
        let (_dir, mut proc) = setup_processor();
        proc.execute("GLOBALV SET COLOR blue");
        let result = proc.execute("GLOBALV GET COLOR");
        assert_eq!(result.rc, 0);
        assert_eq!(result.messages, vec!["blue"]);
    }

    #[test]
    fn execute_access_release() {
        let (dir, mut proc) = setup_processor();
        let b_path = dir.path().join("b");
        let cmd = format!("ACCESS {} B", b_path.to_str().unwrap());
        let result = proc.execute(&cmd);
        assert_eq!(result.rc, 0);
        assert!(proc.filesystem.disk('B').is_some());

        let result2 = proc.execute("RELEASE B");
        assert_eq!(result2.rc, 0);
        assert!(proc.filesystem.disk('B').is_none());
    }

    #[test]
    fn execute_unknown_command_rc3() {
        let (_dir, mut proc) = setup_processor();
        let result = proc.execute("BOGUSCMD");
        assert_eq!(result.rc, 3);
    }

    // -- EXEC resolution tests --

    struct MockExecHandler;

    impl ExecHandler for MockExecHandler {
        fn execute_exec(&mut self, source: &str, args: &str) -> (i32, Vec<String>) {
            (
                0,
                vec![format!(
                    "EXEC ran: source_len={} args={}",
                    source.len(),
                    args
                )],
            )
        }
    }

    #[test]
    fn exec_command_runs_file() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        let spec = FileSpec::parse("MYPROG EXEC A").unwrap();
        fs.write_file(&spec, "/* REXX */ say 'hello'").unwrap();

        let mut proc = CommandProcessor::with_exec_handler(fs, Box::new(MockExecHandler));
        let result = proc.execute("EXEC MYPROG arg1");
        assert_eq!(result.rc, 0);
        assert!(result.messages[0].contains("EXEC ran"));
    }

    #[test]
    fn unknown_command_falls_back_to_exec() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        let spec = FileSpec::parse("MYSCRIPT EXEC A").unwrap();
        fs.write_file(&spec, "/* REXX */ nop").unwrap();

        let mut proc = CommandProcessor::with_exec_handler(fs, Box::new(MockExecHandler));
        let result = proc.execute("MYSCRIPT some args");
        assert_eq!(result.rc, 0);
        assert!(result.messages[0].contains("args=some args"));
    }

    #[test]
    fn exec_not_found_rc3() {
        let (_dir, mut proc) = setup_processor();
        let result = proc.execute("EXEC NOPROG");
        assert_eq!(result.rc, 3);
    }

    // -- PROFILE EXEC tests --

    #[test]
    fn run_profile_found() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        let spec = FileSpec::parse("PROFILE EXEC A").unwrap();
        fs.write_file(&spec, "/* REXX */ say 'profile'").unwrap();

        let mut proc = CommandProcessor::with_exec_handler(fs, Box::new(MockExecHandler));
        let result = proc.run_profile();
        assert!(result.is_some());
        assert_eq!(result.unwrap().rc, 0);
    }

    #[test]
    fn run_profile_not_found() {
        let (_dir, mut proc) = setup_processor();
        let result = proc.run_profile();
        assert!(result.is_none());
    }

    // -- SMSG tests --

    #[test]
    fn parse_smsg() {
        let cmd = parse_cms_command("SMSG OPER Hello world").unwrap();
        match cmd {
            CmsCommand::Smsg { userid, text } => {
                assert_eq!(userid, "OPER");
                assert_eq!(text, "Hello world");
            }
            _ => panic!("Expected SMSG"),
        }
    }

    #[test]
    fn parse_smsg_abbreviated() {
        let cmd = parse_cms_command("sm OPER Hi").unwrap();
        assert!(matches!(cmd, CmsCommand::Smsg { .. }));
    }

    #[test]
    fn parse_smsg_missing_text() {
        let err = parse_cms_command("SMSG OPER").unwrap_err();
        assert!(matches!(err, CmsError::InvalidCommand(_)));
    }

    #[test]
    fn parse_smsg_missing_args() {
        let err = parse_cms_command("SMSG").unwrap_err();
        assert!(matches!(err, CmsError::InvalidCommand(_)));
    }

    use std::sync::{Arc, Mutex};

    struct MockSmsgSender {
        sent: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl SmsgSender for MockSmsgSender {
        fn send_smsg(&self, target: &str, text: &str) -> (i32, String) {
            self.sent
                .lock()
                .unwrap()
                .push((target.to_string(), text.to_string()));
            (0, String::new())
        }
    }

    #[test]
    fn cmd_smsg_with_mock_sender() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sender = MockSmsgSender {
            sent: Arc::clone(&sent),
        };
        let mut proc =
            CommandProcessor::with_smsg_sender(fs, Box::new(NoExecHandler), Box::new(sender));
        let result = proc.execute("SMSG OPER Hello from test");
        assert_eq!(result.rc, 0);
        let msgs = sent.lock().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].0, "OPER");
        assert_eq!(msgs[0].1, "Hello from test");
    }

    #[test]
    fn cmd_smsg_no_sender_rc28() {
        let (_dir, mut proc) = setup_processor();
        let result = proc.execute("SMSG OPER Hello");
        assert_eq!(result.rc, 28);
    }

    // -- take/set roundtrip tests --

    #[test]
    fn take_set_exec_handler_roundtrip() {
        let (_dir, mut proc) = setup_processor();
        let handler = proc.take_exec_handler();
        // After take, handler is NoExecHandler — verify it returns RC=28
        let result = proc.execute("EXEC ANYTHING");
        assert_eq!(result.rc, 3); // not found, but handler would be 28 if found
        proc.set_exec_handler(handler);
    }

    #[test]
    fn take_set_smsg_sender_roundtrip() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sender = MockSmsgSender {
            sent: Arc::clone(&sent),
        };
        let mut proc =
            CommandProcessor::with_smsg_sender(fs, Box::new(NoExecHandler), Box::new(sender));

        let taken = proc.take_smsg_sender();
        // After take, NoSmsgSender is active
        let result = proc.execute("SMSG OPER test");
        assert_eq!(result.rc, 28);

        // Restore
        proc.set_smsg_sender(taken);
        let result = proc.execute("SMSG OPER test2");
        assert_eq!(result.rc, 0);
        assert_eq!(sent.lock().unwrap().len(), 1);
    }
}
