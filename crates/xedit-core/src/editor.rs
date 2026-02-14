use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::buffer::Buffer;
use crate::command::*;
use crate::error::{Result, XeditError};
use crate::prefix::*;
use crate::target::Target;

/// Cursor placement request from CURSOR command
#[derive(Debug, Clone)]
pub enum CursorRequest {
    Home,
    File { line: usize, col: usize },
}

/// Snapshot of editor state for single-level undo
#[derive(Debug, Clone)]
struct UndoSnapshot {
    lines: Vec<String>,
    current_line: usize,
    current_col: usize,
    alt_count: usize,
}

/// XEDIT editor state for a single file.
///
/// This is a pure data model with no I/O beyond file loading/saving.
/// It is designed to be embedded: the TUI or host application drives it
/// by calling `execute()` with parsed commands and rendering the state.
#[derive(Debug)]
pub struct Editor {
    buffer: Buffer,
    /// Current line: 0 = Top of File, 1..=len = file lines
    current_line: usize,
    /// Current column (1-based)
    current_col: usize,

    // File identity (CMS-style)
    filename: String,
    filetype: String,
    filemode: String,
    filepath: Option<PathBuf>,
    readonly: bool,

    // Settings
    trunc: usize,
    zone_left: usize,
    zone_right: usize,
    show_number: bool,
    show_prefix: bool,
    show_scale: bool,
    case_respect: bool,
    hex: bool,
    stay: bool,
    wrap: bool,
    curline: CurLinePosition,
    verify_start: usize,
    verify_end: usize,

    // PF key assignments: PF1..PF24 stored as index 0..23
    pf_keys: [Option<String>; 24],

    // Macro settings
    /// Search path for REXX macros (directories to check)
    macro_path: Vec<PathBuf>,

    // Operational state
    alt_count: usize,
    message: Option<String>,
    pending_block: Option<PendingBlock>,
    pending_operation: Option<PendingOperation>,
    /// Number of lines per page (set by TUI based on screen size)
    page_size: usize,

    // ALL filter: if Some, each entry says whether the line is visible
    all_filter: Option<Vec<bool>>,
    show_shadow: bool,

    // Command history
    command_history: Vec<String>,

    // Undo
    undo_snapshot: Option<UndoSnapshot>,

    // Cursor request
    cursor_request: Option<CursorRequest>,

    // Display customization
    reserved_lines: HashMap<usize, String>,
    color_overrides: HashMap<String, String>,
}

/// Classic VM/CMS XEDIT default PF key assignments
fn default_pf_keys() -> [Option<String>; 24] {
    let mut keys: [Option<String>; 24] = Default::default();
    keys[0] = Some("HELP".to_string()); // PF1
    keys[2] = Some("QUIT".to_string()); // PF3
    keys[3] = Some("TOP".to_string()); // PF4 (TAB in some configs)
    keys[4] = Some("BOTTOM".to_string()); // PF5
    keys[5] = Some("?".to_string()); // PF6 (repeat last command placeholder)
    keys[6] = Some("BACKWARD".to_string()); // PF7
    keys[7] = Some("FORWARD".to_string()); // PF8
    keys[8] = Some("=".to_string()); // PF9 (repeat last command placeholder)
    keys[9] = Some("LOCATE".to_string()); // PF10 (cursor-locate placeholder)
    keys[10] = Some("SAVE".to_string()); // PF11
    keys[11] = Some("FILE".to_string()); // PF12
                                         // PF13-PF24: unassigned by default (mirrored in some configs)
    keys
}

impl Editor {
    pub fn new() -> Self {
        Self {
            buffer: Buffer::new(),
            current_line: 0,
            current_col: 1,
            filename: String::new(),
            filetype: String::new(),
            filemode: "A1".to_string(),
            filepath: None,
            readonly: false,
            trunc: 72,
            zone_left: 1,
            zone_right: 72,
            show_number: true,
            show_prefix: true,
            show_scale: false,
            case_respect: false,
            hex: false,
            stay: true,
            wrap: false,
            curline: CurLinePosition::Middle,
            verify_start: 1,
            verify_end: 80,
            pf_keys: default_pf_keys(),
            macro_path: vec![PathBuf::from(".")],
            alt_count: 0,
            message: None,
            pending_block: None,
            pending_operation: None,
            page_size: 20,
            all_filter: None,
            show_shadow: true,
            command_history: Vec::new(),
            undo_snapshot: None,
            cursor_request: None,
            reserved_lines: HashMap::new(),
            color_overrides: HashMap::new(),
        }
    }

    // -- Accessors for the TUI --

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub fn current_line(&self) -> usize {
        self.current_line
    }

    pub fn current_col(&self) -> usize {
        self.current_col
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn filetype(&self) -> &str {
        &self.filetype
    }

    pub fn filemode(&self) -> &str {
        &self.filemode
    }

    pub fn trunc(&self) -> usize {
        self.trunc
    }

    pub fn alt_count(&self) -> usize {
        self.alt_count
    }

    /// Get the command string assigned to a PF key (1-based: PF1..PF24)
    pub fn pf_key(&self, num: usize) -> Option<&str> {
        if (1..=24).contains(&num) {
            self.pf_keys[num - 1].as_deref()
        } else {
            None
        }
    }

    /// Set a PF key assignment (1-based)
    pub fn set_pf_key(&mut self, num: usize, command: Option<String>) {
        if (1..=24).contains(&num) {
            self.pf_keys[num - 1] = command;
        }
    }

    pub fn show_number(&self) -> bool {
        self.show_number
    }

    pub fn show_prefix(&self) -> bool {
        self.show_prefix
    }

    pub fn show_scale(&self) -> bool {
        self.show_scale
    }

    pub fn curline_position(&self) -> &CurLinePosition {
        &self.curline
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
    }

    pub fn clear_message(&mut self) {
        self.message = None;
    }

    pub fn is_modified(&self) -> bool {
        self.buffer.is_modified()
    }

    pub fn has_pending_block(&self) -> bool {
        self.pending_block.is_some()
    }

    pub fn pending_block(&self) -> Option<&PendingBlock> {
        self.pending_block.as_ref()
    }

    pub fn set_page_size(&mut self, size: usize) {
        self.page_size = size.max(1);
    }

    // -- Cursor request --

    pub fn cursor_request(&self) -> Option<&CursorRequest> {
        self.cursor_request.as_ref()
    }

    pub fn take_cursor_request(&mut self) -> Option<CursorRequest> {
        self.cursor_request.take()
    }

    fn cmd_cursor(&mut self, target: &CursorTarget) -> Result<CommandResult> {
        match target {
            CursorTarget::Home => {
                self.cursor_request = Some(CursorRequest::Home);
            }
            CursorTarget::File { line, col } => {
                self.cursor_request = Some(CursorRequest::File {
                    line: *line,
                    col: *col,
                });
            }
        }
        Ok(CommandResult::ok())
    }

    // -- ALL filter --

    pub fn all_filter_active(&self) -> bool {
        self.all_filter.is_some()
    }

    pub fn is_line_visible(&self, line_num: usize) -> bool {
        match &self.all_filter {
            Some(filter) => {
                if line_num == 0 || line_num > filter.len() {
                    true
                } else {
                    filter[line_num - 1]
                }
            }
            None => true,
        }
    }

    /// Count consecutive hidden lines starting after line_num
    pub fn shadow_count_after(&self, line_num: usize) -> usize {
        match &self.all_filter {
            Some(filter) => {
                let mut count = 0;
                let mut i = line_num; // line_num is 1-based, next is line_num+1
                while i < filter.len() {
                    if !filter[i] {
                        count += 1;
                        i += 1;
                    } else {
                        break;
                    }
                }
                count
            }
            None => 0,
        }
    }

    pub fn show_shadow(&self) -> bool {
        self.show_shadow
    }

    fn cmd_all(&mut self, target: Option<&Target>) -> Result<CommandResult> {
        match target {
            Some(t) => {
                let case_respect = self.case_respect;
                let mut filter = Vec::with_capacity(self.buffer.len());
                let mut visible_count = 0;
                for i in 1..=self.buffer.len() {
                    let text = self.buffer.line_text(i).unwrap_or("");
                    let visible = t.matches_line(case_respect, text);
                    if visible {
                        visible_count += 1;
                    }
                    filter.push(visible);
                }
                self.all_filter = Some(filter);
                Ok(CommandResult::with_message(format!(
                    "{} line(s) displayed",
                    visible_count
                )))
            }
            None => {
                self.all_filter = None;
                Ok(CommandResult::with_message("ALL reset"))
            }
        }
    }

    fn cmd_sort(
        &mut self,
        target: Option<&Target>,
        ascending: bool,
        col_start: Option<usize>,
        col_end: Option<usize>,
    ) -> Result<CommandResult> {
        self.snapshot_for_undo();

        let start = if self.current_line == 0 {
            1
        } else {
            self.current_line
        };
        let end = if let Some(t) = target {
            let case_respect = self.case_respect;
            let buffer = &self.buffer;
            t.resolve(self.current_line, buffer.len(), case_respect, &|n| {
                buffer.line_text(n).map(String::from)
            })
            .unwrap_or(self.buffer.len())
        } else {
            self.buffer.len()
        };

        if start > end || start > self.buffer.len() {
            return Err(XeditError::InvalidCommand("Nothing to sort".to_string()));
        }

        // Extract lines in range
        let mut lines_to_sort: Vec<String> = (start..=end)
            .filter_map(|i| self.buffer.line_text(i).map(String::from))
            .collect();

        // Sort by key
        lines_to_sort.sort_by(|a, b| {
            let key_a = sort_key(a, col_start, col_end);
            let key_b = sort_key(b, col_start, col_end);
            if ascending {
                key_a.cmp(&key_b)
            } else {
                key_b.cmp(&key_a)
            }
        });

        // Replace in buffer
        for (i, text) in lines_to_sort.into_iter().enumerate() {
            let line_num = start + i;
            if let Some(line) = self.buffer.get_mut(line_num) {
                line.set_text(text);
            }
        }

        self.alt_count += 1;
        let count = end - start + 1;
        let direction = if ascending { "ascending" } else { "descending" };
        Ok(CommandResult::with_message(format!(
            "{} line(s) sorted {}",
            count, direction
        )))
    }

    /// Set the macro search path (list of directories)
    pub fn set_macro_path(&mut self, paths: Vec<PathBuf>) {
        self.macro_path = paths;
    }

    /// Get the macro search path
    pub fn macro_path(&self) -> &[PathBuf] {
        &self.macro_path
    }

    // -- Display customization --

    pub fn reserved_line(&self, row: usize) -> Option<&str> {
        self.reserved_lines.get(&row).map(|s| s.as_str())
    }

    pub fn reserved_lines(&self) -> &HashMap<usize, String> {
        &self.reserved_lines
    }

    pub fn color_override(&self, area: &str) -> Option<&str> {
        self.color_overrides.get(area).map(|s| s.as_str())
    }

    pub fn color_overrides(&self) -> &HashMap<String, String> {
        &self.color_overrides
    }

    // -- Command history --

    /// Push a command into the history (skips empty, `?`, `=`)
    pub fn push_history(&mut self, cmd_text: &str) {
        let trimmed = cmd_text.trim();
        if trimmed.is_empty() || trimmed == "?" || trimmed == "=" {
            return;
        }
        self.command_history.push(trimmed.to_string());
    }

    /// Get the most recent command in history
    pub fn last_command(&self) -> Option<&str> {
        self.command_history.last().map(|s| s.as_str())
    }

    /// Number of commands in history
    pub fn history_len(&self) -> usize {
        self.command_history.len()
    }

    /// Get a command by index (0 = oldest)
    pub fn history_get(&self, index: usize) -> Option<&str> {
        self.command_history.get(index).map(|s| s.as_str())
    }

    // -- Undo --

    /// Capture buffer state before a modifying command
    fn snapshot_for_undo(&mut self) {
        self.undo_snapshot = Some(UndoSnapshot {
            lines: self
                .buffer
                .lines()
                .iter()
                .map(|l| l.text().to_string())
                .collect(),
            current_line: self.current_line,
            current_col: self.current_col,
            alt_count: self.alt_count,
        });
    }

    fn cmd_undo(&mut self) -> Result<CommandResult> {
        if let Some(snap) = self.undo_snapshot.take() {
            self.buffer = Buffer::from_lines(snap.lines);
            self.current_line = snap.current_line;
            self.current_col = snap.current_col;
            self.alt_count = snap.alt_count;
            // Clear ALL filter — it may reference the old buffer layout
            self.all_filter = None;
            Ok(CommandResult::with_message("Undone"))
        } else {
            Err(XeditError::InvalidCommand("Nothing to undo".to_string()))
        }
    }

    /// Search the macro path for a macro file, returning its full path and contents.
    #[cfg(feature = "rexx")]
    fn find_macro(&self, name: &str) -> Option<(PathBuf, String)> {
        let candidates = [
            format!("{}.xedit", name),
            name.to_string(),
            format!("{}.XEDIT", name),
            format!("{}.xedit", name.to_lowercase()),
        ];
        for dir in &self.macro_path {
            for candidate in &candidates {
                let path = dir.join(candidate);
                if let Ok(source) = fs::read_to_string(&path) {
                    return Some((path, source));
                }
            }
        }
        None
    }

    /// Run the PROFILE XEDIT macro if it exists.
    ///
    /// Called automatically after loading a file. The profile macro can
    /// customize settings (PF keys, number/prefix display, case, etc.)
    /// based on filetype or other conditions. A missing profile is silently
    /// ignored — it's optional.
    #[cfg(feature = "rexx")]
    pub fn run_profile(&mut self) {
        if let Some((_path, source)) = self.find_macro("PROFILE") {
            match crate::macro_engine::run_macro(self, &source, "") {
                Ok(()) => {}
                Err(e) => {
                    self.message = Some(format!("PROFILE error: {}", e));
                }
            }
        }
    }

    /// Get the current line text, or a marker for TOF/EOF
    pub fn current_line_text(&self) -> &str {
        if self.current_line == 0 {
            ""
        } else {
            self.buffer.line_text(self.current_line).unwrap_or("")
        }
    }

    /// Whether the current line pointer is at Top of File
    pub fn at_tof(&self) -> bool {
        self.current_line == 0
    }

    /// Whether the current line pointer is at the last line
    pub fn at_eof(&self) -> bool {
        self.current_line >= self.buffer.len()
    }

    // -- File operations --

    pub fn load_file(&mut self, path: &Path) -> Result<()> {
        let content = fs::read_to_string(path)
            .map_err(|_| XeditError::FileNotFound(path.display().to_string()))?;
        let lines: Vec<String> = content.lines().map(String::from).collect();
        self.buffer = Buffer::from_lines(lines);

        if let Some(stem) = path.file_stem() {
            self.filename = stem.to_string_lossy().to_uppercase();
        }
        if let Some(ext) = path.extension() {
            self.filetype = ext.to_string_lossy().to_uppercase();
        }
        self.filepath = Some(path.to_path_buf());

        let max_width = self
            .buffer
            .lines()
            .iter()
            .map(|l| l.len())
            .max()
            .unwrap_or(80);
        if max_width > self.trunc {
            self.trunc = max_width;
            self.zone_right = max_width;
            self.verify_end = max_width;
        }

        self.current_line = if self.buffer.is_empty() { 0 } else { 1 };
        self.alt_count = 0;
        Ok(())
    }

    pub fn save_file(&mut self) -> Result<()> {
        let path = self.filepath.as_ref().ok_or(XeditError::NoFile)?;
        if self.readonly {
            return Err(XeditError::ReadOnly);
        }
        let content: String = self
            .buffer
            .lines()
            .iter()
            .map(|l| l.text())
            .collect::<Vec<_>>()
            .join("\n");

        let content = if content.is_empty() {
            String::new()
        } else {
            content + "\n"
        };

        fs::write(path, &content)?;
        self.buffer.clear_modified();
        self.alt_count = 0;
        Ok(())
    }

    // -- Command execution --

    pub fn execute(&mut self, cmd: &Command) -> Result<CommandResult> {
        self.message = None;
        let result = match cmd {
            Command::Up(n) => self.cmd_up(*n),
            Command::Down(n) => self.cmd_down(*n),
            Command::Top => self.cmd_top(),
            Command::Bottom => self.cmd_bottom(),
            Command::Forward(n) => self.cmd_forward(*n),
            Command::Backward(n) => self.cmd_backward(*n),
            Command::Left(n) => self.cmd_left(*n),
            Command::Right(n) => self.cmd_right(*n),
            Command::Locate(target) => self.cmd_locate(target),
            Command::Change {
                from,
                to,
                target,
                count,
            } => self.cmd_change(from, to, target.as_ref(), *count),
            Command::Input(text) => self.cmd_input(text.as_deref()),
            Command::Delete(target) => self.cmd_delete(target.as_ref()),
            Command::File => self.cmd_file(),
            Command::Save => self.cmd_save(),
            Command::Quit => self.cmd_quit(),
            Command::QQuit => Ok(CommandResult::quit()),
            Command::Get(filename) => self.cmd_get(filename),
            Command::Undo => self.cmd_undo(),
            Command::Cursor(target) => self.cmd_cursor(target),
            Command::All(target) => self.cmd_all(target.as_ref()),
            Command::Sort {
                target,
                ascending,
                col_start,
                col_end,
            } => self.cmd_sort(target.as_ref(), *ascending, *col_start, *col_end),
            Command::Set(subcmd) => self.cmd_set(subcmd),
            Command::Query(what) => self.cmd_query(what),
            Command::Refresh => Ok(CommandResult::refresh()),
            Command::Help => self.cmd_help(),
            #[cfg(feature = "rexx")]
            Command::Macro(args) => self.cmd_macro(args),
            Command::Nop => Ok(CommandResult::ok()),
        };

        // Capture message from result
        if let Ok(ref r) = result {
            if let Some(ref msg) = r.message {
                self.message = Some(msg.clone());
            }
        }
        if let Err(ref e) = result {
            self.message = Some(e.to_string());
        }

        result
    }

    /// Insert a line in input mode (called by TUI for each line entered)
    pub fn input_line(&mut self, text: &str) {
        self.buffer.insert_after(self.current_line, text);
        self.current_line += 1;
        self.alt_count += 1;
    }

    // -- Character-level editing (for data area screen editing) --

    /// Overtype a character at the given line and 0-based column
    pub fn overtype_char(&mut self, line_num: usize, col: usize, ch: char) {
        if line_num == 0 || line_num > self.buffer.len() {
            return;
        }
        if let Some(line) = self.buffer.get_mut(line_num) {
            let mut text = line.text().to_string();
            // Pad with spaces if needed
            while text.len() <= col {
                text.push(' ');
            }
            let mut chars: Vec<char> = text.chars().collect();
            if col < chars.len() {
                chars[col] = ch;
            }
            line.set_text(chars.into_iter().collect::<String>());
        }
    }

    /// Insert a character at the given line and 0-based column
    pub fn insert_char(&mut self, line_num: usize, col: usize, ch: char) {
        if line_num == 0 || line_num > self.buffer.len() {
            return;
        }
        if let Some(line) = self.buffer.get_mut(line_num) {
            let mut text = line.text().to_string();
            while text.len() < col {
                text.push(' ');
            }
            text.insert(col, ch);
            line.set_text(text);
        }
    }

    /// Delete the character at the given line and 0-based column
    pub fn delete_char(&mut self, line_num: usize, col: usize) {
        if line_num == 0 || line_num > self.buffer.len() {
            return;
        }
        if let Some(line) = self.buffer.get_mut(line_num) {
            let mut text = line.text().to_string();
            if col < text.len() {
                text.remove(col);
                line.set_text(text);
            }
        }
    }

    /// Set the current line directly (used when cursor movement drives position)
    pub fn set_current_line(&mut self, line: usize) {
        self.current_line = line.min(self.buffer.len());
    }

    // -- Navigation --

    fn cmd_up(&mut self, n: usize) -> Result<CommandResult> {
        if self.all_filter.is_some() {
            let mut remaining = n;
            while remaining > 0 && self.current_line > 0 {
                self.current_line -= 1;
                if self.is_line_visible(self.current_line) || self.current_line == 0 {
                    remaining -= 1;
                }
            }
        } else {
            self.current_line = self.current_line.saturating_sub(n);
        }
        Ok(CommandResult::ok())
    }

    fn cmd_down(&mut self, n: usize) -> Result<CommandResult> {
        if self.all_filter.is_some() {
            let mut remaining = n;
            while remaining > 0 && self.current_line < self.buffer.len() {
                self.current_line += 1;
                if self.is_line_visible(self.current_line) {
                    remaining -= 1;
                }
            }
        } else {
            self.current_line = (self.current_line + n).min(self.buffer.len());
        }
        Ok(CommandResult::ok())
    }

    fn cmd_top(&mut self) -> Result<CommandResult> {
        self.current_line = 0;
        Ok(CommandResult::ok())
    }

    fn cmd_bottom(&mut self) -> Result<CommandResult> {
        self.current_line = self.buffer.len();
        Ok(CommandResult::ok())
    }

    fn cmd_forward(&mut self, pages: usize) -> Result<CommandResult> {
        self.cmd_down(pages * self.page_size)
    }

    fn cmd_backward(&mut self, pages: usize) -> Result<CommandResult> {
        self.cmd_up(pages * self.page_size)
    }

    fn cmd_left(&mut self, n: usize) -> Result<CommandResult> {
        self.current_col = self.current_col.saturating_sub(n).max(1);
        Ok(CommandResult::ok())
    }

    fn cmd_right(&mut self, n: usize) -> Result<CommandResult> {
        self.current_col += n;
        Ok(CommandResult::ok())
    }

    // -- Search --

    fn cmd_locate(&mut self, target: &Target) -> Result<CommandResult> {
        let case_respect = self.case_respect;
        let buffer = &self.buffer;
        let resolved = target.resolve(self.current_line, buffer.len(), case_respect, &|n| {
            buffer.line_text(n).map(String::from)
        });
        match resolved {
            Some(line) => {
                self.current_line = line;
                Ok(CommandResult::ok())
            }
            None => {
                let msg = match target {
                    Target::StringForward(s) | Target::StringBackward(s) => {
                        format!("Target not found: {}", s)
                    }
                    _ => "Target not found".to_string(),
                };
                Err(XeditError::TargetNotFound(msg))
            }
        }
    }

    fn cmd_change(
        &mut self,
        from: &str,
        to: &str,
        target: Option<&Target>,
        count: Option<usize>,
    ) -> Result<CommandResult> {
        self.snapshot_for_undo();
        let max_changes = count.unwrap_or(1);
        let mut changes_made = 0;

        let end_line = if let Some(t) = target {
            let case_respect = self.case_respect;
            let buffer = &self.buffer;
            t.resolve(self.current_line, buffer.len(), case_respect, &|n| {
                buffer.line_text(n).map(String::from)
            })
            .unwrap_or(self.buffer.len())
        } else {
            self.buffer.len()
        };

        let start = if self.current_line == 0 {
            1
        } else {
            self.current_line
        };

        for line_num in start..=end_line {
            if changes_made >= max_changes {
                break;
            }
            if let Some(line) = self.buffer.get(line_num) {
                let text = line.text().to_string();
                let (needle, haystack) = if self.case_respect {
                    (from.to_string(), text.clone())
                } else {
                    (from.to_uppercase(), text.to_uppercase())
                };

                if let Some(pos) = haystack.find(&needle) {
                    let new_text = format!("{}{}{}", &text[..pos], to, &text[pos + from.len()..]);
                    if let Some(line_mut) = self.buffer.get_mut(line_num) {
                        line_mut.set_text(new_text);
                    }
                    changes_made += 1;
                    self.alt_count += 1;
                    if !self.stay {
                        self.current_line = line_num;
                    }
                }
            }
        }

        if changes_made > 0 {
            Ok(CommandResult::with_message(format!(
                "{} change(s) made",
                changes_made
            )))
        } else {
            Err(XeditError::TargetNotFound(format!(
                "\"{}\" not found",
                from
            )))
        }
    }

    // -- Editing --

    fn cmd_input(&mut self, text: Option<&str>) -> Result<CommandResult> {
        if let Some(text) = text {
            self.snapshot_for_undo();
            self.buffer.insert_after(self.current_line, text);
            self.current_line += 1;
            self.alt_count += 1;
            Ok(CommandResult::ok())
        } else {
            Ok(CommandResult::enter_input())
        }
    }

    fn cmd_delete(&mut self, target: Option<&Target>) -> Result<CommandResult> {
        self.snapshot_for_undo();
        if self.current_line == 0 {
            return Err(XeditError::InvalidCommand(
                "Cannot delete at Top of File".to_string(),
            ));
        }

        match target {
            None => {
                self.buffer.delete(self.current_line);
                self.alt_count += 1;
                if self.current_line > self.buffer.len() {
                    self.current_line = self.buffer.len();
                }
                Ok(CommandResult::ok())
            }
            Some(Target::Star) => {
                let count = self.buffer.len() - self.current_line + 1;
                self.buffer
                    .delete_range(self.current_line, self.buffer.len());
                self.alt_count += count;
                if self.current_line > self.buffer.len() {
                    self.current_line = self.buffer.len();
                }
                Ok(CommandResult::with_message(format!(
                    "{} line(s) deleted",
                    count
                )))
            }
            Some(Target::Relative(n)) if *n > 0 => {
                let end = (self.current_line + *n as usize - 1).min(self.buffer.len());
                let count = end - self.current_line + 1;
                self.buffer.delete_range(self.current_line, end);
                self.alt_count += count;
                if self.current_line > self.buffer.len() {
                    self.current_line = self.buffer.len();
                }
                Ok(CommandResult::with_message(format!(
                    "{} line(s) deleted",
                    count
                )))
            }
            _ => Err(XeditError::InvalidCommand(
                "Invalid target for DELETE".to_string(),
            )),
        }
    }

    // -- File commands --

    fn cmd_file(&mut self) -> Result<CommandResult> {
        self.save_file()?;
        Ok(CommandResult::quit())
    }

    fn cmd_save(&mut self) -> Result<CommandResult> {
        self.save_file()?;
        Ok(CommandResult::with_message("File saved"))
    }

    fn cmd_quit(&mut self) -> Result<CommandResult> {
        if self.buffer.is_modified() {
            Err(XeditError::FileModified)
        } else {
            Ok(CommandResult::quit())
        }
    }

    fn cmd_get(&mut self, filename: &str) -> Result<CommandResult> {
        let path = Path::new(filename);
        let content =
            fs::read_to_string(path).map_err(|_| XeditError::FileNotFound(filename.to_string()))?;
        let lines: Vec<String> = content.lines().map(String::from).collect();
        let count = lines.len();
        self.buffer.insert_lines_after(self.current_line, lines);
        self.alt_count += count;
        Ok(CommandResult::with_message(format!(
            "{} line(s) read from {}",
            count, filename
        )))
    }

    // -- Settings --

    fn cmd_set(&mut self, subcmd: &SetCommand) -> Result<CommandResult> {
        match subcmd {
            SetCommand::Trunc(n) => {
                self.trunc = *n;
                self.zone_right = *n;
            }
            SetCommand::Zone(left, right) => {
                self.zone_left = *left;
                self.zone_right = *right;
            }
            SetCommand::Number(on) => self.show_number = *on,
            SetCommand::Prefix(on) => self.show_prefix = *on,
            SetCommand::Scale(on) => self.show_scale = *on,
            SetCommand::CurLine(pos) => self.curline = pos.clone(),
            SetCommand::Case(setting) => {
                self.case_respect = matches!(setting, CaseSetting::Respect);
            }
            SetCommand::Wrap(on) => self.wrap = *on,
            SetCommand::Hex(on) => self.hex = *on,
            SetCommand::Stay(on) => self.stay = *on,
            SetCommand::Shadow(on) => self.show_shadow = *on,
            SetCommand::Reserved(row, text) => {
                self.reserved_lines.insert(*row, text.clone());
            }
            SetCommand::ReservedOff(row) => {
                self.reserved_lines.remove(row);
            }
            SetCommand::Color(area, color) => {
                let key = format!("{:?}", area);
                self.color_overrides.insert(key, color.clone());
            }
            SetCommand::MsgLine(_) => {}
            SetCommand::Verify(start, end) => {
                self.verify_start = *start;
                self.verify_end = *end;
            }
            SetCommand::Pf(num, cmd) => {
                if cmd.is_empty() {
                    self.set_pf_key(*num, None);
                } else {
                    self.set_pf_key(*num, Some(cmd.clone()));
                }
            }
        }
        Ok(CommandResult::ok())
    }

    fn cmd_query(&self, what: &str) -> Result<CommandResult> {
        let what_upper = what.trim().to_uppercase();
        let msg = match what_upper.as_str() {
            "" => format!(
                "Size={} Line={} Col={} Alt={} Trunc={}",
                self.buffer.len(),
                self.current_line,
                self.current_col,
                self.alt_count,
                self.trunc,
            ),
            "SIZE" => format!("Size={}", self.buffer.len()),
            "LINE" => format!("Line={}", self.current_line),
            "COLUMN" | "COL" => format!("Col={}", self.current_col),
            "TRUNC" => format!("Trunc={}", self.trunc),
            "ALT" => format!("Alt={}", self.alt_count),
            "LRECL" => format!("Lrecl={}", self.buffer.lrecl()),
            "RECFM" => format!("Recfm={:?}", self.buffer.recfm()),
            _ => {
                return Err(XeditError::InvalidCommand(format!(
                    "Unknown QUERY: {}",
                    what
                )))
            }
        };
        Ok(CommandResult::with_message(msg))
    }

    #[cfg(feature = "rexx")]
    fn cmd_macro(&mut self, args: &str) -> Result<CommandResult> {
        let (macro_name, macro_args) = if let Some(pos) = args.find(char::is_whitespace) {
            (&args[..pos], args[pos..].trim())
        } else {
            (args, "")
        };

        let (_path, source) = self
            .find_macro(macro_name)
            .ok_or_else(|| XeditError::FileNotFound(format!("Macro not found: {}", macro_name)))?;

        crate::macro_engine::run_macro(self, &source, macro_args)?;
        Ok(CommandResult::ok())
    }

    fn cmd_help(&self) -> Result<CommandResult> {
        Ok(CommandResult::with_message(
            "Commands: UP DOWN TOP BOTTOM FORWARD BACKWARD LOCATE CHANGE INPUT DELETE FILE SAVE QUIT QQUIT SET QUERY",
        ))
    }

    // -- Prefix command execution --

    pub fn execute_prefix(
        &mut self,
        line_num: usize,
        cmd: &PrefixCommand,
    ) -> Result<CommandResult> {
        // Snapshot for undo on modifying prefix commands
        match cmd {
            PrefixCommand::SetCurrent => {}
            _ => self.snapshot_for_undo(),
        }
        match cmd {
            PrefixCommand::SetCurrent => {
                self.current_line = line_num;
                Ok(CommandResult::ok())
            }
            PrefixCommand::Delete => {
                self.buffer.delete(line_num);
                self.alt_count += 1;
                if self.current_line > self.buffer.len() {
                    self.current_line = self.buffer.len();
                }
                Ok(CommandResult::ok())
            }
            PrefixCommand::Insert(n) | PrefixCommand::Add(n) => {
                for _ in 0..*n {
                    self.buffer.insert_after(line_num, "");
                }
                self.alt_count += n;
                Ok(CommandResult::ok())
            }
            PrefixCommand::Duplicate(n) => {
                if let Some(line) = self.buffer.get(line_num) {
                    let text = line.text().to_string();
                    for i in 0..*n {
                        self.buffer.insert_after(line_num + i, text.clone());
                    }
                    self.alt_count += n;
                }
                Ok(CommandResult::ok())
            }
            PrefixCommand::ShiftRight(n) => {
                if let Some(line) = self.buffer.get_mut(line_num) {
                    let new_text = format!("{}{}", " ".repeat(*n), line.text());
                    line.set_text(new_text);
                    self.alt_count += 1;
                }
                Ok(CommandResult::ok())
            }
            PrefixCommand::ShiftLeft(n) => {
                if let Some(line) = self.buffer.get_mut(line_num) {
                    let text = line.text();
                    let spaces = text.len().min(*n);
                    if text[..spaces].chars().all(|c| c == ' ') {
                        let new_text = text[spaces..].to_string();
                        line.set_text(new_text);
                        self.alt_count += 1;
                    }
                }
                Ok(CommandResult::ok())
            }
            PrefixCommand::DeleteBlock => self.handle_block_marker(line_num, BlockType::Delete),
            PrefixCommand::CopyBlock => self.handle_block_marker(line_num, BlockType::Copy),
            PrefixCommand::MoveBlock => self.handle_block_marker(line_num, BlockType::Move),
            PrefixCommand::DuplicateBlock => {
                self.handle_block_marker(line_num, BlockType::Duplicate)
            }
            PrefixCommand::Copy => {
                self.pending_operation = Some(PendingOperation {
                    op_type: OperationType::Copy,
                    source_start: line_num,
                    source_end: line_num,
                });
                Ok(CommandResult::with_message(
                    "Copy pending — use F or P for destination",
                ))
            }
            PrefixCommand::Move => {
                self.pending_operation = Some(PendingOperation {
                    op_type: OperationType::Move,
                    source_start: line_num,
                    source_end: line_num,
                });
                Ok(CommandResult::with_message(
                    "Move pending — use F or P for destination",
                ))
            }
            PrefixCommand::Following => self.execute_pending_destination(line_num, true),
            PrefixCommand::Preceding => self.execute_pending_destination(line_num, false),
        }
    }

    fn handle_block_marker(
        &mut self,
        line_num: usize,
        block_type: BlockType,
    ) -> Result<CommandResult> {
        if let Some(pending) = self.pending_block.take() {
            if pending.command != block_type {
                self.pending_block = Some(pending);
                return Err(XeditError::PrefixError(
                    "Conflicting block operation pending".to_string(),
                ));
            }
            let (start, end) = if pending.start_line <= line_num {
                (pending.start_line, line_num)
            } else {
                (line_num, pending.start_line)
            };

            match block_type {
                BlockType::Delete => {
                    let count = end - start + 1;
                    self.buffer.delete_range(start, end);
                    self.alt_count += count;
                    if self.current_line > self.buffer.len() {
                        self.current_line = self.buffer.len();
                    }
                    Ok(CommandResult::with_message(format!(
                        "{} line(s) deleted",
                        count
                    )))
                }
                BlockType::Copy => {
                    self.pending_operation = Some(PendingOperation {
                        op_type: OperationType::Copy,
                        source_start: start,
                        source_end: end,
                    });
                    Ok(CommandResult::with_message(
                        "Block marked — use F or P for destination",
                    ))
                }
                BlockType::Move => {
                    self.pending_operation = Some(PendingOperation {
                        op_type: OperationType::Move,
                        source_start: start,
                        source_end: end,
                    });
                    Ok(CommandResult::with_message(
                        "Block marked — use F or P for destination",
                    ))
                }
                BlockType::Duplicate => {
                    let mut texts = Vec::new();
                    for i in start..=end {
                        if let Some(line) = self.buffer.get(i) {
                            texts.push(line.text().to_string());
                        }
                    }
                    self.buffer.insert_lines_after(end, texts);
                    self.alt_count += end - start + 1;
                    Ok(CommandResult::with_message("Block duplicated"))
                }
            }
        } else {
            self.pending_block = Some(PendingBlock {
                command: block_type,
                start_line: line_num,
            });
            Ok(CommandResult::ok())
        }
    }

    fn execute_pending_destination(
        &mut self,
        dest_line: usize,
        after: bool,
    ) -> Result<CommandResult> {
        let op = self
            .pending_operation
            .take()
            .ok_or_else(|| XeditError::PrefixError("No pending copy/move operation".to_string()))?;

        let mut texts = Vec::new();
        for i in op.source_start..=op.source_end {
            if let Some(line) = self.buffer.get(i) {
                texts.push(line.text().to_string());
            }
        }

        let insert_after = if after {
            dest_line
        } else {
            dest_line.saturating_sub(1)
        };

        // For move: delete source lines first (adjust dest if needed)
        if op.op_type == OperationType::Move {
            let count = op.source_end - op.source_start + 1;
            self.buffer.delete_range(op.source_start, op.source_end);

            // Adjust destination if it was after the deleted block
            let adjusted = if insert_after >= op.source_start {
                insert_after.saturating_sub(count)
            } else {
                insert_after
            };
            self.buffer.insert_lines_after(adjusted, texts);
            self.alt_count += count;

            if self.current_line > self.buffer.len() {
                self.current_line = self.buffer.len();
            }
            Ok(CommandResult::with_message(format!(
                "{} line(s) moved",
                count
            )))
        } else {
            let count = texts.len();
            self.buffer.insert_lines_after(insert_after, texts);
            self.alt_count += count;
            Ok(CommandResult::with_message(format!(
                "{} line(s) copied",
                count
            )))
        }
    }
}

/// Extract sort key from a line, optionally by column range (1-based).
/// Uses character-based indexing to avoid panics on multibyte UTF-8.
fn sort_key(line: &str, col_start: Option<usize>, col_end: Option<usize>) -> String {
    match (col_start, col_end) {
        (Some(start), Some(end)) => {
            let s = start.saturating_sub(1);
            line.chars().skip(s).take(end.saturating_sub(s)).collect()
        }
        (Some(start), None) => {
            let s = start.saturating_sub(1);
            line.chars().skip(s).collect()
        }
        _ => line.to_string(),
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with_lines(lines: &[&str]) -> Editor {
        let mut ed = Editor::new();
        ed.buffer = Buffer::from_lines(lines.iter().map(|s| s.to_string()).collect());
        ed.current_line = 1;
        ed
    }

    #[test]
    fn history_push_and_recall() {
        let mut ed = Editor::new();
        ed.push_history("LOCATE /foo/");
        ed.push_history("CHANGE /a/b/");
        assert_eq!(ed.history_len(), 2);
        assert_eq!(ed.last_command(), Some("CHANGE /a/b/"));
        assert_eq!(ed.history_get(0), Some("LOCATE /foo/"));
    }

    #[test]
    fn reserved_set_and_clear() {
        let mut ed = editor_with_lines(&["a"]);
        ed.execute(&Command::Set(SetCommand::Reserved(1, "My Header".into())))
            .unwrap();
        assert_eq!(ed.reserved_line(1), Some("My Header"));

        ed.execute(&Command::Set(SetCommand::ReservedOff(1)))
            .unwrap();
        assert!(ed.reserved_line(1).is_none());
    }

    #[test]
    fn color_override() {
        let mut ed = editor_with_lines(&["a"]);
        ed.execute(&Command::Set(SetCommand::Color(
            ColorArea::FileArea,
            "RED".into(),
        )))
        .unwrap();
        assert_eq!(ed.color_override("FileArea"), Some("RED"));
    }

    #[test]
    fn cursor_home() {
        let mut ed = editor_with_lines(&["a", "b"]);
        ed.execute(&Command::Cursor(CursorTarget::Home)).unwrap();
        assert!(ed.cursor_request().is_some());
        match ed.take_cursor_request().unwrap() {
            CursorRequest::Home => {}
            other => panic!("Expected Home, got {:?}", other),
        }
    }

    #[test]
    fn cursor_file() {
        let mut ed = editor_with_lines(&["a", "b"]);
        ed.execute(&Command::Cursor(CursorTarget::File { line: 2, col: 5 }))
            .unwrap();
        match ed.take_cursor_request().unwrap() {
            CursorRequest::File { line, col } => {
                assert_eq!(line, 2);
                assert_eq!(col, 5);
            }
            other => panic!("Expected File, got {:?}", other),
        }
    }

    #[test]
    fn sort_ascending() {
        let mut ed = editor_with_lines(&["cherry", "apple", "banana"]);
        ed.execute(&Command::Sort {
            target: None,
            ascending: true,
            col_start: None,
            col_end: None,
        })
        .unwrap();
        assert_eq!(ed.buffer().line_text(1), Some("apple"));
        assert_eq!(ed.buffer().line_text(2), Some("banana"));
        assert_eq!(ed.buffer().line_text(3), Some("cherry"));
    }

    #[test]
    fn sort_descending() {
        let mut ed = editor_with_lines(&["apple", "cherry", "banana"]);
        ed.execute(&Command::Sort {
            target: None,
            ascending: false,
            col_start: None,
            col_end: None,
        })
        .unwrap();
        assert_eq!(ed.buffer().line_text(1), Some("cherry"));
        assert_eq!(ed.buffer().line_text(2), Some("banana"));
        assert_eq!(ed.buffer().line_text(3), Some("apple"));
    }

    #[test]
    fn sort_column_range() {
        let mut ed = editor_with_lines(&["BBB_aaa", "AAA_ccc", "CCC_bbb"]);
        // Sort by columns 5-7 (the last 3 chars after _)
        ed.execute(&Command::Sort {
            target: None,
            ascending: true,
            col_start: Some(5),
            col_end: Some(7),
        })
        .unwrap();
        assert_eq!(ed.buffer().line_text(1), Some("BBB_aaa"));
        assert_eq!(ed.buffer().line_text(2), Some("CCC_bbb"));
        assert_eq!(ed.buffer().line_text(3), Some("AAA_ccc"));
    }

    #[test]
    fn all_filter_basic() {
        let mut ed = editor_with_lines(&["apple", "banana", "apricot", "cherry"]);
        ed.execute(&Command::All(Some(Target::StringForward("ap".into()))))
            .unwrap();
        assert!(ed.all_filter_active());
        assert!(ed.is_line_visible(1)); // apple
        assert!(!ed.is_line_visible(2)); // banana
        assert!(ed.is_line_visible(3)); // apricot
        assert!(!ed.is_line_visible(4)); // cherry
    }

    #[test]
    fn all_filter_reset() {
        let mut ed = editor_with_lines(&["apple", "banana"]);
        ed.execute(&Command::All(Some(Target::StringForward("ap".into()))))
            .unwrap();
        assert!(ed.all_filter_active());
        ed.execute(&Command::All(None)).unwrap();
        assert!(!ed.all_filter_active());
    }

    #[test]
    fn all_with_compound_target() {
        let mut ed = editor_with_lines(&["hello world", "hello there", "goodbye world"]);
        ed.execute(&Command::All(Some(
            Target::parse("/hello/&/world/").unwrap(),
        )))
        .unwrap();
        assert!(ed.is_line_visible(1)); // "hello world" has both
        assert!(!ed.is_line_visible(2)); // "hello there" missing "world"
        assert!(!ed.is_line_visible(3)); // "goodbye world" missing "hello"
    }

    #[test]
    fn undo_delete() {
        let mut ed = editor_with_lines(&["a", "b", "c"]);
        ed.current_line = 2;
        ed.execute(&Command::Delete(None)).unwrap();
        assert_eq!(ed.buffer().len(), 2);

        ed.execute(&Command::Undo).unwrap();
        assert_eq!(ed.buffer().len(), 3);
        assert_eq!(ed.buffer().line_text(2), Some("b"));
        assert_eq!(ed.current_line(), 2);
    }

    #[test]
    fn undo_change() {
        let mut ed = editor_with_lines(&["hello world"]);
        ed.execute(&Command::Change {
            from: "hello".into(),
            to: "hi".into(),
            target: None,
            count: None,
        })
        .unwrap();
        assert_eq!(ed.buffer().line_text(1), Some("hi world"));

        ed.execute(&Command::Undo).unwrap();
        assert_eq!(ed.buffer().line_text(1), Some("hello world"));
    }

    #[test]
    fn undo_nothing() {
        let mut ed = editor_with_lines(&["a"]);
        let result = ed.execute(&Command::Undo);
        assert!(result.is_err());
    }

    #[test]
    fn history_skips_special() {
        let mut ed = Editor::new();
        ed.push_history("?");
        ed.push_history("=");
        ed.push_history("");
        ed.push_history("  ");
        assert_eq!(ed.history_len(), 0);
    }

    #[test]
    fn navigate_up_down() {
        let mut ed = editor_with_lines(&["a", "b", "c", "d", "e"]);
        ed.current_line = 3;

        ed.execute(&Command::Up(2)).unwrap();
        assert_eq!(ed.current_line(), 1);

        ed.execute(&Command::Down(4)).unwrap();
        assert_eq!(ed.current_line(), 5);

        // Clamp at boundaries
        ed.execute(&Command::Down(10)).unwrap();
        assert_eq!(ed.current_line(), 5);

        ed.execute(&Command::Up(100)).unwrap();
        assert_eq!(ed.current_line(), 0); // TOF
    }

    #[test]
    fn top_and_bottom() {
        let mut ed = editor_with_lines(&["a", "b", "c"]);

        ed.execute(&Command::Bottom).unwrap();
        assert_eq!(ed.current_line(), 3);

        ed.execute(&Command::Top).unwrap();
        assert_eq!(ed.current_line(), 0);
    }

    #[test]
    fn locate_forward() {
        let mut ed = editor_with_lines(&["alpha", "beta", "gamma"]);
        ed.current_line = 0;

        ed.execute(&Command::Locate(Target::StringForward("beta".into())))
            .unwrap();
        assert_eq!(ed.current_line(), 2);
    }

    #[test]
    fn locate_not_found() {
        let mut ed = editor_with_lines(&["alpha", "beta"]);
        let result = ed.execute(&Command::Locate(Target::StringForward("xyz".into())));
        assert!(result.is_err());
    }

    #[test]
    fn change_text() {
        let mut ed = editor_with_lines(&["hello world", "hello there"]);

        ed.execute(&Command::Change {
            from: "hello".into(),
            to: "hi".into(),
            target: None,
            count: None,
        })
        .unwrap();

        assert_eq!(ed.buffer().line_text(1), Some("hi world"));
        // Only first occurrence changed (count=1 default)
        assert_eq!(ed.buffer().line_text(2), Some("hello there"));
    }

    #[test]
    fn delete_current_line() {
        let mut ed = editor_with_lines(&["a", "b", "c"]);
        ed.current_line = 2;

        ed.execute(&Command::Delete(None)).unwrap();
        assert_eq!(ed.buffer().len(), 2);
        assert_eq!(ed.buffer().line_text(2), Some("c"));
    }

    #[test]
    fn input_line() {
        let mut ed = editor_with_lines(&["first", "third"]);
        ed.current_line = 1;

        ed.execute(&Command::Input(Some("second".into()))).unwrap();
        assert_eq!(ed.buffer().len(), 3);
        assert_eq!(ed.buffer().line_text(2), Some("second"));
        assert_eq!(ed.current_line(), 2);
    }

    #[test]
    fn prefix_delete() {
        let mut ed = editor_with_lines(&["a", "b", "c"]);

        ed.execute_prefix(2, &PrefixCommand::Delete).unwrap();
        assert_eq!(ed.buffer().len(), 2);
        assert_eq!(ed.buffer().line_text(1), Some("a"));
        assert_eq!(ed.buffer().line_text(2), Some("c"));
    }

    #[test]
    fn prefix_duplicate() {
        let mut ed = editor_with_lines(&["alpha", "beta"]);

        ed.execute_prefix(1, &PrefixCommand::Duplicate(2)).unwrap();
        assert_eq!(ed.buffer().len(), 4);
        assert_eq!(ed.buffer().line_text(2), Some("alpha"));
        assert_eq!(ed.buffer().line_text(3), Some("alpha"));
    }

    #[test]
    fn prefix_block_delete() {
        let mut ed = editor_with_lines(&["a", "b", "c", "d", "e"]);

        ed.execute_prefix(2, &PrefixCommand::DeleteBlock).unwrap();
        ed.execute_prefix(4, &PrefixCommand::DeleteBlock).unwrap();

        assert_eq!(ed.buffer().len(), 2);
        assert_eq!(ed.buffer().line_text(1), Some("a"));
        assert_eq!(ed.buffer().line_text(2), Some("e"));
    }

    #[cfg(feature = "rexx")]
    #[test]
    fn profile_xedit_runs_on_load() {
        let dir = tempfile::TempDir::new().unwrap();

        let profile_path = dir.path().join("profile.xedit");
        let data_path = dir.path().join("test.txt");

        // Profile macro: set number off, move to bottom
        fs::write(&profile_path, "/* PROFILE */\n'SET NUMBER OFF'\n'BOTTOM'\n").unwrap();
        fs::write(&data_path, "line1\nline2\nline3\n").unwrap();

        let mut ed = Editor::new();
        ed.set_macro_path(vec![dir.path().to_path_buf()]);
        ed.load_file(&data_path).unwrap();
        ed.run_profile();

        // Profile should have turned off line numbers
        assert!(!ed.show_number());
        // Profile should have moved to bottom
        assert_eq!(ed.current_line(), 3);
    }

    #[cfg(feature = "rexx")]
    #[test]
    fn profile_missing_is_silent() {
        let dir = tempfile::TempDir::new().unwrap();

        let data_path = dir.path().join("test.txt");
        fs::write(&data_path, "hello\n").unwrap();

        let mut ed = Editor::new();
        ed.set_macro_path(vec![dir.path().to_path_buf()]);
        ed.load_file(&data_path).unwrap();
        ed.run_profile(); // should not error

        assert!(ed.message().is_none());
    }

    #[cfg(feature = "rexx")]
    #[test]
    fn profile_filetype_conditional() {
        let dir = tempfile::TempDir::new().unwrap();

        let profile_path = dir.path().join("profile.xedit");
        let data_path = dir.path().join("code.rs");

        // Profile that sets case respect only for .rs files
        let profile = r#"/* PROFILE */
if ftype.1 = 'RS' then
    'SET CASE RESPECT'
"#;
        fs::write(&profile_path, profile).unwrap();
        fs::write(&data_path, "fn main() {}\n").unwrap();

        let mut ed = Editor::new();
        ed.set_macro_path(vec![dir.path().to_path_buf()]);
        ed.load_file(&data_path).unwrap();
        ed.run_profile();

        // Verify case-sensitive locate: uppercase "FN" should NOT match lowercase "fn"
        ed.set_current_line(0);
        let upper = ed.execute(&Command::Locate(Target::StringForward("FN".into())));
        assert!(upper.is_err());

        // But lowercase "fn" should still match
        ed.set_current_line(0);
        let lower = ed.execute(&Command::Locate(Target::StringForward("fn".into())));
        assert!(lower.is_ok());
    }
}
