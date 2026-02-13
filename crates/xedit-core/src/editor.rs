use std::fs;
use std::path::{Path, PathBuf};

use crate::buffer::Buffer;
use crate::command::*;
use crate::error::{Result, XeditError};
use crate::prefix::*;
use crate::target::Target;

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

    // Operational state
    alt_count: usize,
    message: Option<String>,
    pending_block: Option<PendingBlock>,
    pending_operation: Option<PendingOperation>,
    /// Number of lines per page (set by TUI based on screen size)
    page_size: usize,
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
            alt_count: 0,
            message: None,
            pending_block: None,
            pending_operation: None,
            page_size: 20,
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
            Command::Set(subcmd) => self.cmd_set(subcmd),
            Command::Query(what) => self.cmd_query(what),
            Command::Refresh => Ok(CommandResult::refresh()),
            Command::Help => self.cmd_help(),
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
        self.buffer
            .insert_after(self.current_line, text);
        self.current_line += 1;
        self.alt_count += 1;
    }

    // -- Navigation --

    fn cmd_up(&mut self, n: usize) -> Result<CommandResult> {
        self.current_line = self.current_line.saturating_sub(n);
        Ok(CommandResult::ok())
    }

    fn cmd_down(&mut self, n: usize) -> Result<CommandResult> {
        self.current_line = (self.current_line + n).min(self.buffer.len());
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
            self.buffer
                .insert_after(self.current_line, text);
            self.current_line += 1;
            self.alt_count += 1;
            Ok(CommandResult::ok())
        } else {
            Ok(CommandResult::enter_input())
        }
    }

    fn cmd_delete(&mut self, target: Option<&Target>) -> Result<CommandResult> {
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
        let content = fs::read_to_string(path)
            .map_err(|_| XeditError::FileNotFound(filename.to_string()))?;
        let lines: Vec<String> = content.lines().map(String::from).collect();
        let count = lines.len();
        self.buffer
            .insert_lines_after(self.current_line, lines);
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
            SetCommand::MsgLine(_) => {}
            SetCommand::Verify(start, end) => {
                self.verify_start = *start;
                self.verify_end = *end;
            }
        }
        Ok(CommandResult::ok())
    }

    fn cmd_query(&self, what: &str) -> Result<CommandResult> {
        let what_upper = what.trim().to_uppercase();
        let msg = match what_upper.as_str() {
            s if s.is_empty() => format!(
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
            PrefixCommand::DeleteBlock => {
                self.handle_block_marker(line_num, BlockType::Delete)
            }
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
        let op = self.pending_operation.take().ok_or_else(|| {
            XeditError::PrefixError("No pending copy/move operation".to_string())
        })?;

        let mut texts = Vec::new();
        for i in op.source_start..=op.source_end {
            if let Some(line) = self.buffer.get(i) {
                texts.push(line.text().to_string());
            }
        }

        let insert_after = if after { dest_line } else { dest_line.saturating_sub(1) };

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
}
