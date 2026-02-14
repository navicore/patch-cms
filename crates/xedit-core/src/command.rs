use crate::target::Target;

/// XEDIT command line commands
#[derive(Debug, Clone)]
pub enum Command {
    // Navigation
    Up(usize),
    Down(usize),
    Top,
    Bottom,
    Forward(usize),
    Backward(usize),
    Left(usize),
    Right(usize),

    // Search and replace
    Locate(Target),
    Change {
        from: String,
        to: String,
        target: Option<Target>,
        count: Option<usize>,
    },

    // Editing
    Input(Option<String>),
    Delete(Option<Target>),

    // File operations
    File,
    Save,
    Quit,
    QQuit,
    Get(String),

    // Settings
    Set(SetCommand),
    Query(String),

    // Macros
    #[cfg(feature = "rexx")]
    Macro(String),

    // Undo
    Undo,

    // Filtering
    All(Option<Target>),

    // Sorting
    Sort {
        target: Option<Target>,
        ascending: bool,
        col_start: Option<usize>,
        col_end: Option<usize>,
    },

    // Cursor control
    Cursor(CursorTarget),

    // Display
    Refresh,
    Help,
    Nop,
}

/// SET subcommands
#[derive(Debug, Clone)]
pub enum SetCommand {
    Trunc(usize),
    Zone(usize, usize),
    Number(bool),
    Prefix(bool),
    Scale(bool),
    CurLine(CurLinePosition),
    Case(CaseSetting),
    Wrap(bool),
    Hex(bool),
    Stay(bool),
    MsgLine(usize),
    Verify(usize, usize),
    Shadow(bool),
    /// SET RESERVED row text
    Reserved(usize, String),
    /// SET RESERVED row OFF
    ReservedOff(usize),
    /// SET COLOR area colorname
    Color(ColorArea, String),
    /// SET PFn command_text (1-24)
    Pf(usize, String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CaseSetting {
    Mixed,
    Upper,
    Respect,
    Ignore,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CurLinePosition {
    Row(usize),
    Middle,
}

/// Areas that can have color overrides
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ColorArea {
    FileArea,
    CmdLine,
    Prefix,
    CurLine,
    IdLine,
    MsgLine,
    Shadow,
}

/// CURSOR command target
#[derive(Debug, Clone)]
pub enum CursorTarget {
    Home,
    File { line: usize, col: usize },
}

/// Result of executing a command
#[derive(Debug)]
pub struct CommandResult {
    pub action: CommandAction,
    pub message: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum CommandAction {
    Continue,
    Quit,
    EnterInput,
    Refresh,
}

impl CommandResult {
    pub fn ok() -> Self {
        Self {
            action: CommandAction::Continue,
            message: None,
        }
    }

    pub fn with_message(msg: impl Into<String>) -> Self {
        Self {
            action: CommandAction::Continue,
            message: Some(msg.into()),
        }
    }

    pub fn quit() -> Self {
        Self {
            action: CommandAction::Quit,
            message: None,
        }
    }

    pub fn enter_input() -> Self {
        Self {
            action: CommandAction::EnterInput,
            message: None,
        }
    }

    pub fn refresh() -> Self {
        Self {
            action: CommandAction::Refresh,
            message: None,
        }
    }
}

// -- Command abbreviation table --
// Each entry: (full_name, minimum_abbreviation_length)
// Follows IBM XEDIT abbreviation conventions.
const COMMAND_TABLE: &[(&str, usize)] = &[
    ("ALL", 3),      // ALL
    ("BACKWARD", 1), // B
    ("BOTTOM", 2),   // BO
    ("CHANGE", 1),   // C
    ("CURSOR", 3),   // CUR
    ("DELETE", 3),   // DEL
    ("DOWN", 2),     // DO
    ("FILE", 4),     // FILE
    ("FORWARD", 1),  // F
    ("GET", 3),      // GET
    ("HELP", 4),     // HELP
    ("INPUT", 1),    // I
    ("LEFT", 2),     // LE
    ("LOCATE", 1),   // L (but see disambiguation below)
    ("MACRO", 5),    // MACRO
    ("NEXT", 1),     // N
    ("QQUIT", 2),    // QQ
    ("QUERY", 2),    // QU
    ("QUIT", 4),     // QUIT
    ("REFRESH", 3),  // REF
    ("RIGHT", 2),    // RI
    ("SAVE", 2),     // SA
    ("SET", 3),      // SET
    ("SORT", 4),     // SORT
    ("TOP", 1),      // T
    ("UNDO", 4),     // UNDO
    ("UP", 1),       // U
];

fn lookup_command(input: &str) -> Option<&'static str> {
    let input_upper = input.to_uppercase();
    // Try exact match first
    for &(name, _) in COMMAND_TABLE {
        if name == input_upper {
            return Some(name);
        }
    }
    // Try abbreviation match
    let mut matches = Vec::new();
    for &(name, min_abbrev) in COMMAND_TABLE {
        if input_upper.len() >= min_abbrev && name.starts_with(&input_upper) {
            matches.push(name);
        }
    }
    // Disambiguation: prefer LOCATE over LEFT for single "L"
    if matches.len() > 1 && matches.contains(&"LOCATE") && input_upper == "L" {
        return Some("LOCATE");
    }
    matches.first().copied()
}

/// Parse a command line string into a Command
pub fn parse_command(input: &str) -> Result<Command, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(Command::Nop);
    }

    // /target/ is a LOCATE shorthand
    if input.starts_with('/') {
        let target = Target::parse(input)?;
        return Ok(Command::Locate(target));
    }

    let (cmd_word, args) = split_first_word(input);
    let cmd_name =
        lookup_command(cmd_word).ok_or_else(|| format!("Unknown command: {}", cmd_word))?;

    match cmd_name {
        "UP" => Ok(Command::Up(parse_optional_count(args)?)),
        "DOWN" | "NEXT" => Ok(Command::Down(parse_optional_count(args)?)),
        "TOP" => Ok(Command::Top),
        "BOTTOM" => Ok(Command::Bottom),
        "FORWARD" => Ok(Command::Forward(parse_optional_count(args)?)),
        "BACKWARD" => Ok(Command::Backward(parse_optional_count(args)?)),
        "LEFT" => Ok(Command::Left(parse_optional_count(args)?)),
        "RIGHT" => Ok(Command::Right(parse_optional_count(args)?)),
        "LOCATE" => {
            if args.is_empty() {
                return Err("LOCATE requires a target".to_string());
            }
            Ok(Command::Locate(Target::parse(args)?))
        }
        "CHANGE" => parse_change_args(args),
        "CURSOR" => parse_cursor_args(args),
        "INPUT" => {
            if args.is_empty() {
                Ok(Command::Input(None))
            } else {
                Ok(Command::Input(Some(args.to_string())))
            }
        }
        "DELETE" => {
            if args.is_empty() {
                Ok(Command::Delete(None))
            } else {
                Ok(Command::Delete(Some(Target::parse(args)?)))
            }
        }
        "FILE" => Ok(Command::File),
        "SAVE" => Ok(Command::Save),
        "QUIT" => Ok(Command::Quit),
        "QQUIT" => Ok(Command::QQuit),
        "GET" => {
            if args.is_empty() {
                Err("GET requires a filename".to_string())
            } else {
                Ok(Command::Get(args.to_string()))
            }
        }
        "SET" => parse_set_args(args),
        "QUERY" => Ok(Command::Query(args.to_string())),
        #[cfg(feature = "rexx")]
        "MACRO" => {
            if args.is_empty() {
                Err("MACRO requires a filename".to_string())
            } else {
                Ok(Command::Macro(args.to_string()))
            }
        }
        "ALL" => {
            if args.is_empty() {
                Ok(Command::All(None))
            } else {
                Ok(Command::All(Some(Target::parse(args)?)))
            }
        }
        "SORT" => parse_sort_args(args),
        "UNDO" => Ok(Command::Undo),
        "REFRESH" => Ok(Command::Refresh),
        "HELP" => Ok(Command::Help),
        _ => Err(format!("Unknown command: {}", cmd_word)),
    }
}

fn split_first_word(input: &str) -> (&str, &str) {
    match input.find(char::is_whitespace) {
        Some(pos) => (&input[..pos], input[pos..].trim()),
        None => (input, ""),
    }
}

fn parse_optional_count(args: &str) -> Result<usize, String> {
    if args.is_empty() {
        Ok(1)
    } else {
        args.parse::<usize>()
            .map_err(|_| format!("Invalid count: {}", args))
    }
}

fn parse_change_args(args: &str) -> Result<Command, String> {
    if args.is_empty() {
        return Err("CHANGE requires /old/new/ arguments".to_string());
    }
    let delim = args.chars().next().unwrap();
    let rest = &args[delim.len_utf8()..];

    let from_end = rest
        .find(delim)
        .ok_or_else(|| "CHANGE: missing delimiter after search string".to_string())?;
    let from = rest[..from_end].to_string();

    let after_from = &rest[from_end + delim.len_utf8()..];
    let (to, remainder) = if let Some(to_end) = after_from.find(delim) {
        (
            after_from[..to_end].to_string(),
            after_from[to_end + delim.len_utf8()..].trim(),
        )
    } else {
        (after_from.to_string(), "")
    };

    let (target, count) = if remainder.is_empty() {
        (None, None)
    } else if let Ok(n) = remainder.parse::<usize>() {
        (None, Some(n))
    } else {
        let parts: Vec<&str> = remainder.splitn(2, char::is_whitespace).collect();
        let target = Some(Target::parse(parts[0])?);
        let count = parts.get(1).and_then(|s| s.trim().parse::<usize>().ok());
        (target, count)
    };

    Ok(Command::Change {
        from,
        to,
        target,
        count,
    })
}

/// Parse CURSOR HOME | CURSOR FILE line col
fn parse_cursor_args(args: &str) -> Result<Command, String> {
    if args.is_empty() {
        return Err("CURSOR requires HOME or FILE line col".to_string());
    }
    let (subcmd, subargs) = split_first_word(args);
    let upper = subcmd.to_uppercase();
    match upper.as_str() {
        "HOME" | "H" => Ok(Command::Cursor(CursorTarget::Home)),
        "FILE" | "F" => {
            let parts: Vec<&str> = subargs.split_whitespace().collect();
            if parts.len() < 2 {
                return Err("CURSOR FILE requires line and col".to_string());
            }
            let line = parts[0]
                .parse::<usize>()
                .map_err(|_| format!("Invalid line number: {}", parts[0]))?;
            let col = parts[1]
                .parse::<usize>()
                .map_err(|_| format!("Invalid column: {}", parts[1]))?;
            Ok(Command::Cursor(CursorTarget::File { line, col }))
        }
        _ => Err(format!("CURSOR: expected HOME or FILE, got: {}", subcmd)),
    }
}

/// Parse SORT [target] [A|D] [col_start [col_end]]
fn parse_sort_args(args: &str) -> Result<Command, String> {
    let mut target = None;
    let mut ascending = true;
    let mut col_start = None;
    let mut col_end = None;

    let mut remaining = args.trim();

    // Try to parse a target first (starts with /, :, +, -, *, or digit)
    if !remaining.is_empty() {
        let first_char = remaining.chars().next().unwrap();
        if first_char == '/'
            || first_char == ':'
            || first_char == '+'
            || first_char == '-'
            || first_char == '*'
        {
            // Find where the target ends: after a closing delimiter or at next space
            let target_end = if first_char == '/' {
                // Find closing /
                let rest = &remaining[1..];
                match rest.find('/') {
                    Some(pos) => pos + 2, // include both delimiters
                    None => remaining.len(),
                }
            } else if first_char == '*' {
                1
            } else {
                remaining
                    .find(char::is_whitespace)
                    .unwrap_or(remaining.len())
            };
            let target_str = &remaining[..target_end];
            target = Some(Target::parse(target_str)?);
            remaining = remaining[target_end..].trim();
        } else if first_char.is_ascii_digit() {
            // Could be a plain number target
            let end = remaining
                .find(char::is_whitespace)
                .unwrap_or(remaining.len());
            let word = &remaining[..end];
            if word.parse::<i64>().is_ok() {
                target = Some(Target::parse(word)?);
                remaining = remaining[end..].trim();
            }
        }
    }

    // Parse A|D
    if !remaining.is_empty() {
        let (word, rest) = split_first_word(remaining);
        let upper = word.to_uppercase();
        if upper == "A" || upper == "ASCENDING" {
            ascending = true;
            remaining = rest;
        } else if upper == "D" || upper == "DESCENDING" {
            ascending = false;
            remaining = rest;
        }
    }

    // Parse col_start [col_end]
    if !remaining.is_empty() {
        let (word, rest) = split_first_word(remaining);
        col_start = Some(
            word.parse::<usize>()
                .map_err(|_| format!("Invalid column number: {}", word))?,
        );
        remaining = rest;
    }
    if !remaining.is_empty() {
        let (word, _) = split_first_word(remaining);
        col_end = Some(
            word.parse::<usize>()
                .map_err(|_| format!("Invalid column number: {}", word))?,
        );
    }

    Ok(Command::Sort {
        target,
        ascending,
        col_start,
        col_end,
    })
}

fn parse_set_args(args: &str) -> Result<Command, String> {
    if args.is_empty() {
        return Err("SET requires a subcommand".to_string());
    }
    let (subcmd, subargs) = split_first_word(args);
    let subcmd_upper = subcmd.to_uppercase();

    if matches_abbrev(&subcmd_upper, "TRUNCATE", 2) {
        let n = subargs
            .parse::<usize>()
            .map_err(|_| "SET TRUNC requires a column number".to_string())?;
        Ok(Command::Set(SetCommand::Trunc(n)))
    } else if matches_abbrev(&subcmd_upper, "NUMBER", 2) {
        Ok(Command::Set(SetCommand::Number(parse_on_off(subargs)?)))
    } else if matches_abbrev(&subcmd_upper, "PREFIX", 2) {
        Ok(Command::Set(SetCommand::Prefix(parse_on_off(subargs)?)))
    } else if matches_abbrev(&subcmd_upper, "SCALE", 2) {
        Ok(Command::Set(SetCommand::Scale(parse_on_off(subargs)?)))
    } else if matches_abbrev(&subcmd_upper, "CURLINE", 3) {
        match subargs.to_uppercase().as_str() {
            "M" | "MIDDLE" => Ok(Command::Set(SetCommand::CurLine(CurLinePosition::Middle))),
            _ => {
                let n = subargs
                    .parse::<usize>()
                    .map_err(|_| "SET CURLINE requires row number or M".to_string())?;
                Ok(Command::Set(SetCommand::CurLine(CurLinePosition::Row(n))))
            }
        }
    } else if matches_abbrev(&subcmd_upper, "CASE", 2) {
        let setting = match subargs.to_uppercase().as_str() {
            "M" | "MIXED" => CaseSetting::Mixed,
            "U" | "UPPER" => CaseSetting::Upper,
            "R" | "RESPECT" => CaseSetting::Respect,
            "I" | "IGNORE" => CaseSetting::Ignore,
            _ => {
                return Err(format!(
                    "SET CASE: expected MIXED/UPPER/RESPECT/IGNORE, got: {}",
                    subargs
                ))
            }
        };
        Ok(Command::Set(SetCommand::Case(setting)))
    } else if matches_abbrev(&subcmd_upper, "WRAP", 2) {
        Ok(Command::Set(SetCommand::Wrap(parse_on_off(subargs)?)))
    } else if matches_abbrev(&subcmd_upper, "HEX", 3) {
        Ok(Command::Set(SetCommand::Hex(parse_on_off(subargs)?)))
    } else if matches_abbrev(&subcmd_upper, "RESERVED", 3) {
        // SET RESERVED row text | SET RESERVED row OFF
        let (row_str, rest) = split_first_word(subargs);
        let row = row_str
            .parse::<usize>()
            .map_err(|_| format!("SET RESERVED: invalid row: {}", row_str))?;
        if rest.is_empty() || rest.to_uppercase() == "OFF" {
            Ok(Command::Set(SetCommand::ReservedOff(row)))
        } else {
            Ok(Command::Set(SetCommand::Reserved(row, rest.to_string())))
        }
    } else if matches_abbrev(&subcmd_upper, "COLOR", 3)
        || matches_abbrev(&subcmd_upper, "COLOUR", 3)
    {
        // SET COLOR area colorname
        let (area_str, color) = split_first_word(subargs);
        if color.is_empty() {
            return Err("SET COLOR requires area and color".to_string());
        }
        let area = match area_str.to_uppercase().as_str() {
            "FILEAREA" | "FILE" => ColorArea::FileArea,
            "CMDLINE" | "CMD" => ColorArea::CmdLine,
            "PREFIX" | "PRE" => ColorArea::Prefix,
            "CURLINE" | "CUR" => ColorArea::CurLine,
            "IDLINE" | "ID" => ColorArea::IdLine,
            "MSGLINE" | "MSG" => ColorArea::MsgLine,
            "SHADOW" | "SHA" => ColorArea::Shadow,
            _ => return Err(format!("SET COLOR: unknown area: {}", area_str)),
        };
        Ok(Command::Set(SetCommand::Color(area, color.to_uppercase())))
    } else if matches_abbrev(&subcmd_upper, "SHADOW", 3) {
        Ok(Command::Set(SetCommand::Shadow(parse_on_off(subargs)?)))
    } else if matches_abbrev(&subcmd_upper, "STAY", 2) {
        Ok(Command::Set(SetCommand::Stay(parse_on_off(subargs)?)))
    } else if let Some(num_str) = subcmd_upper.strip_prefix("PF") {
        // SET PFn command_text
        let num = num_str
            .parse::<usize>()
            .map_err(|_| format!("Invalid PF key number: {}", num_str))?;
        if !(1..=24).contains(&num) {
            return Err(format!("PF key must be 1-24, got: {}", num));
        }
        if subargs.is_empty() || subargs.to_uppercase() == "OFF" {
            Ok(Command::Set(SetCommand::Pf(num, String::new())))
        } else {
            Ok(Command::Set(SetCommand::Pf(num, subargs.to_string())))
        }
    } else {
        Err(format!("Unknown SET subcommand: {}", subcmd))
    }
}

fn matches_abbrev(input: &str, full: &str, min: usize) -> bool {
    input.len() >= min && full.starts_with(input)
}

fn parse_on_off(s: &str) -> Result<bool, String> {
    match s.to_uppercase().as_str() {
        "ON" => Ok(true),
        "OFF" => Ok(false),
        _ => Err(format!("Expected ON or OFF, got: {}", s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_up() {
        match parse_command("u 5").unwrap() {
            Command::Up(5) => {}
            other => panic!("Expected Up(5), got {:?}", other),
        }
    }

    #[test]
    fn parse_down_abbreviated() {
        match parse_command("do 3").unwrap() {
            Command::Down(3) => {}
            other => panic!("Expected Down(3), got {:?}", other),
        }
    }

    #[test]
    fn parse_locate() {
        match parse_command("l /hello/").unwrap() {
            Command::Locate(Target::StringForward(s)) => assert_eq!(s, "hello"),
            other => panic!("Expected Locate, got {:?}", other),
        }
    }

    #[test]
    fn parse_locate_shorthand() {
        match parse_command("/hello/").unwrap() {
            Command::Locate(Target::StringForward(s)) => assert_eq!(s, "hello"),
            other => panic!("Expected Locate, got {:?}", other),
        }
    }

    #[test]
    fn parse_change() {
        match parse_command("c /foo/bar/").unwrap() {
            Command::Change { from, to, .. } => {
                assert_eq!(from, "foo");
                assert_eq!(to, "bar");
            }
            other => panic!("Expected Change, got {:?}", other),
        }
    }

    #[test]
    fn parse_qquit() {
        match parse_command("qq").unwrap() {
            Command::QQuit => {}
            other => panic!("Expected QQuit, got {:?}", other),
        }
    }

    #[test]
    fn parse_set_number() {
        match parse_command("set nu on").unwrap() {
            Command::Set(SetCommand::Number(true)) => {}
            other => panic!("Expected Set Number ON, got {:?}", other),
        }
    }

    #[test]
    fn parse_set_case_respect() {
        match parse_command("set ca respect").unwrap() {
            Command::Set(SetCommand::Case(CaseSetting::Respect)) => {}
            other => panic!("Expected Set Case Respect, got {:?}", other),
        }
    }

    #[test]
    fn parse_nop() {
        match parse_command("").unwrap() {
            Command::Nop => {}
            other => panic!("Expected Nop, got {:?}", other),
        }
    }
}
