use std::collections::HashMap;
use std::io;

use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use xedit_core::command::{parse_command, Command, CommandAction};
use xedit_core::editor::{CursorRequest, Editor};
use xedit_core::filesystem::FileSystem;
use xedit_core::prefix::PrefixCommand;
use xedit_core::ring::Ring;

use crate::input::{read_action, Action};
use crate::screen;

/// Width of the prefix area in screen columns (5 chars + 1 space)
const PREFIX_COLS: usize = 6;

/// Where the cursor focus is
#[derive(Debug, Clone, PartialEq)]
pub enum CursorFocus {
    /// Cursor is in the ====> command line
    CommandLine,
    /// Cursor is in the file area (prefix or data depending on column)
    FileArea,
}

/// Application state
pub struct App {
    ring: Ring,
    focus: CursorFocus,

    // Command line state
    command_text: String,
    command_cursor: usize,

    // File area cursor (buffer coordinates)
    /// 1-based line number in buffer (0 = TOF, can be on TOF but can't edit it)
    file_line: usize,
    /// 1-based screen column (1-5 = prefix, 7+ = data, 6 = separator/skip)
    file_col: usize,

    // Pending prefix inputs: line_num -> typed text
    prefix_inputs: HashMap<usize, String>,

    // Command history browsing
    history_index: Option<usize>,

    // Insert vs overtype mode for data area
    insert_mode: bool,

    // Input mode (from INPUT command)
    in_input_mode: bool,
    input_text: String,

    should_quit: bool,

    // Track whether we've snapshotted for the current file-area editing session
    file_area_edited: bool,

    // CMS command processor (only present when built with --features cms)
    #[cfg(feature = "cms")]
    cms_processor: Option<cms_core::CommandProcessor>,
    #[cfg(feature = "cms")]
    cms_base_path: Option<String>,
}

impl App {
    pub fn new() -> Self {
        let mut ring = Ring::new();
        ring.add_empty();
        Self {
            ring,
            focus: CursorFocus::CommandLine,
            command_text: String::new(),
            command_cursor: 0,
            file_line: 1,
            file_col: 7, // start in data area
            prefix_inputs: HashMap::new(),
            history_index: None,
            insert_mode: false,
            in_input_mode: false,
            input_text: String::new(),
            should_quit: false,
            file_area_edited: false,
            #[cfg(feature = "cms")]
            cms_processor: None,
            #[cfg(feature = "cms")]
            cms_base_path: None,
        }
    }

    /// Construct an App in CMS mode with a command processor and CMS filesystem.
    #[cfg(feature = "cms")]
    pub fn with_cms(
        processor: cms_core::CommandProcessor,
        cms_fs: cms_core::CmsFs,
        base_path: String,
    ) -> Self {
        let mut ring = Ring::new();
        ring.add_empty_with_fs(Box::new(cms_fs));
        Self {
            ring,
            cms_processor: Some(processor),
            cms_base_path: Some(base_path),
            focus: CursorFocus::CommandLine,
            command_text: String::new(),
            command_cursor: 0,
            file_line: 1,
            file_col: 7,
            prefix_inputs: HashMap::new(),
            history_index: None,
            insert_mode: false,
            in_input_mode: false,
            input_text: String::new(),
            should_quit: false,
            file_area_edited: false,
        }
    }

    // -- Accessor helpers --

    fn editor(&self) -> &Editor {
        self.ring.current().expect("Ring is empty")
    }

    fn editor_mut(&mut self) -> &mut Editor {
        self.ring.current_mut().expect("Ring is empty")
    }

    pub fn load_file(&mut self, file_id: &str) -> xedit_core::error::Result<()> {
        self.editor_mut().load_file(file_id)?;
        // Run PROFILE XEDIT macro if it exists (customizes settings on file open)
        #[cfg(feature = "rexx")]
        self.editor_mut().run_profile();
        self.file_line = self.editor().current_line().max(1);
        Ok(())
    }

    pub fn run(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        crossterm::execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.event_loop(&mut terminal);

        disable_raw_mode()?;
        crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        result
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<()> {
        loop {
            let size = terminal.size()?;
            self.editor_mut()
                .set_page_size(size.height.saturating_sub(3) as usize);

            let editor = self.ring.current().expect("Ring is empty");
            let (ring_pos, ring_total) = self.ring.ring_position();

            terminal.draw(|frame| {
                screen::render(
                    frame,
                    editor,
                    ring_pos,
                    ring_total,
                    &self.command_text,
                    self.command_cursor,
                    &self.focus,
                    self.file_line,
                    self.file_col,
                    &self.prefix_inputs,
                    self.in_input_mode,
                    &self.input_text,
                    self.insert_mode,
                );
            })?;

            if self.should_quit {
                break;
            }

            let action = read_action()?;
            self.handle_action(action);
        }

        Ok(())
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::ForceQuit => {
                self.should_quit = true;
                return;
            }
            Action::None => return,
            // PF keys work regardless of focus — just like a real 3270
            Action::PfKey(num) => {
                self.handle_pf_key(num);
                return;
            }
            _ => {}
        }

        if self.in_input_mode {
            self.handle_input_mode(action);
            return;
        }

        match self.focus {
            CursorFocus::CommandLine => self.handle_command_line(action),
            CursorFocus::FileArea => self.handle_file_area(action),
        }
    }

    // -- Command line handling --

    fn handle_command_line(&mut self, action: Action) {
        match action {
            Action::Char(c) => {
                self.command_text.insert(self.command_cursor, c);
                self.command_cursor += 1;
            }
            Action::Backspace => {
                if self.command_cursor > 0 {
                    self.command_cursor -= 1;
                    self.command_text.remove(self.command_cursor);
                }
            }
            Action::Delete => {
                if self.command_cursor < self.command_text.len() {
                    self.command_text.remove(self.command_cursor);
                }
            }
            Action::Enter => {
                let text = self.command_text.clone();
                self.command_text.clear();
                self.command_cursor = 0;
                if !text.is_empty() {
                    self.execute_command_text(&text);
                }
            }
            Action::ArrowLeft => {
                self.command_cursor = self.command_cursor.saturating_sub(1);
            }
            Action::ArrowRight => {
                if self.command_cursor < self.command_text.len() {
                    self.command_cursor += 1;
                }
            }
            Action::ArrowUp => {
                if self.editor().history_len() > 0 {
                    // Browse history backward (older)
                    let new_idx = match self.history_index {
                        Some(idx) => idx.saturating_sub(1),
                        None => self.editor().history_len().saturating_sub(1),
                    };
                    self.history_index = Some(new_idx);
                    if let Some(cmd) = self.editor().history_get(new_idx) {
                        self.command_text = cmd.to_string();
                        self.command_cursor = self.command_text.len();
                    }
                } else {
                    let _ = self.editor_mut().execute(&Command::Up(1));
                    self.sync_file_cursor_to_editor();
                }
            }
            Action::ArrowDown => {
                if let Some(idx) = self.history_index {
                    // Browse history forward (newer)
                    let max_idx = self.editor().history_len().saturating_sub(1);
                    if idx < max_idx {
                        let new_idx = idx + 1;
                        self.history_index = Some(new_idx);
                        if let Some(cmd) = self.editor().history_get(new_idx) {
                            self.command_text = cmd.to_string();
                            self.command_cursor = self.command_text.len();
                        }
                    } else {
                        // Past the end of history: clear
                        self.history_index = None;
                        self.command_text.clear();
                        self.command_cursor = 0;
                    }
                } else {
                    let _ = self.editor_mut().execute(&Command::Down(1));
                    self.sync_file_cursor_to_editor();
                }
            }
            Action::Home => {
                self.command_cursor = 0;
            }
            Action::End => {
                self.command_cursor = self.command_text.len();
            }
            Action::Tab | Action::BackTab => {
                // Switch to file area
                self.focus = CursorFocus::FileArea;
                // Position cursor on current line, in data area
                self.file_line = self.editor().current_line().max(1);
                self.file_col = 7; // first data column
            }
            Action::PageUp => {
                let _ = self.editor_mut().execute(&Command::Backward(1));
                self.sync_file_cursor_to_editor();
            }
            Action::PageDown => {
                let _ = self.editor_mut().execute(&Command::Forward(1));
                self.sync_file_cursor_to_editor();
            }
            Action::Escape => {
                self.command_text.clear();
                self.command_cursor = 0;
            }
            _ => {}
        }
    }

    // -- File area handling --

    fn handle_file_area(&mut self, action: Action) {
        let in_prefix = self.file_col >= 1 && self.file_col <= 5;
        let buf_len = self.editor().buffer().len();

        match action {
            Action::Char(c) => {
                if in_prefix {
                    self.type_in_prefix(c);
                } else {
                    self.type_in_data(c);
                }
            }
            Action::Backspace => {
                if in_prefix {
                    self.backspace_in_prefix();
                } else {
                    self.backspace_in_data();
                }
            }
            Action::Delete => {
                let file_line = self.file_line;
                if !in_prefix && file_line >= 1 && file_line <= buf_len {
                    self.ensure_screen_edit_snapshot();
                    let data_col = self.file_col.saturating_sub(PREFIX_COLS + 1);
                    self.editor_mut().delete_char(file_line, data_col);
                }
            }
            Action::Enter => {
                self.process_enter();
                self.file_area_edited = false;
            }
            Action::Tab => {
                // Tab: cycle prefix → data → command line
                if in_prefix {
                    self.file_col = 7; // jump to data area
                } else {
                    self.focus = CursorFocus::CommandLine;
                    self.file_area_edited = false;
                }
            }
            Action::BackTab => {
                // Shift-Tab: reverse cycle
                if in_prefix {
                    self.focus = CursorFocus::CommandLine;
                    self.file_area_edited = false;
                } else {
                    self.file_col = 1; // jump to prefix area
                }
            }
            Action::ArrowUp => {
                if self.file_line > 1 {
                    self.file_line -= 1;
                    let fl = self.file_line;
                    self.editor_mut().set_current_line(fl);
                }
            }
            Action::ArrowDown => {
                if self.file_line < buf_len {
                    self.file_line += 1;
                    let fl = self.file_line;
                    self.editor_mut().set_current_line(fl);
                }
            }
            Action::ArrowLeft => {
                if self.file_col > 1 {
                    self.file_col -= 1;
                    // Skip the separator column (6)
                    if self.file_col == PREFIX_COLS {
                        self.file_col = 5;
                    }
                }
            }
            Action::ArrowRight => {
                self.file_col += 1;
                // Skip the separator column (6)
                if self.file_col == PREFIX_COLS {
                    self.file_col = 7;
                }
            }
            Action::Home => {
                if in_prefix {
                    self.file_col = 1;
                } else {
                    self.file_col = 7;
                }
            }
            Action::End => {
                if in_prefix {
                    self.file_col = 5;
                } else if let Some(text) = self.editor().buffer().line_text(self.file_line) {
                    self.file_col = PREFIX_COLS + 1 + text.len();
                }
            }
            Action::PageUp => {
                let _ = self.editor_mut().execute(&Command::Backward(1));
                self.sync_file_cursor_to_editor();
            }
            Action::PageDown => {
                let _ = self.editor_mut().execute(&Command::Forward(1));
                self.sync_file_cursor_to_editor();
            }
            Action::InsertToggle => {
                self.insert_mode = !self.insert_mode;
                let mode = if self.insert_mode {
                    "Insert mode"
                } else {
                    "Overtype mode"
                };
                self.editor_mut().set_message(mode);
            }
            Action::Escape => {
                // Escape in file area: return to command line, clear pending prefixes
                self.prefix_inputs.clear();
                self.focus = CursorFocus::CommandLine;
                self.file_area_edited = false;
            }
            _ => {}
        }
    }

    // -- Prefix area editing --

    fn type_in_prefix(&mut self, c: char) {
        let line = self.file_line;
        if line == 0 || line > self.editor().buffer().len() {
            return;
        }

        let col_idx = self.file_col - 1; // 0-based within prefix (0..5)
        if col_idx >= 5 {
            return;
        }

        let entry = self.prefix_inputs.entry(line).or_default();

        // Pad with spaces up to cursor position
        while entry.len() <= col_idx {
            entry.push(' ');
        }

        // Overtype character at position
        let mut chars: Vec<char> = entry.chars().collect();
        if col_idx < chars.len() {
            chars[col_idx] = c;
        }
        *entry = chars.into_iter().collect();

        // Advance cursor within prefix area
        if self.file_col < 5 {
            self.file_col += 1;
        }
    }

    fn backspace_in_prefix(&mut self) {
        let line = self.file_line;
        if self.file_col <= 1 {
            return;
        }

        self.file_col -= 1;
        let col_idx = self.file_col - 1;

        if let Some(entry) = self.prefix_inputs.get_mut(&line) {
            let mut chars: Vec<char> = entry.chars().collect();
            if col_idx < chars.len() {
                chars[col_idx] = ' ';
                *entry = chars.into_iter().collect();
            }
            // If all spaces, remove the entry
            if entry.trim().is_empty() {
                self.prefix_inputs.remove(&line);
            }
        }
    }

    // -- Data area editing --

    /// Snapshot once per file-area editing session (first keystroke)
    fn ensure_screen_edit_snapshot(&mut self) {
        if !self.file_area_edited {
            self.editor_mut().snapshot_for_undo();
            self.file_area_edited = true;
        }
    }

    fn type_in_data(&mut self, c: char) {
        let line = self.file_line;
        if line == 0 || line > self.editor().buffer().len() {
            return;
        }

        self.ensure_screen_edit_snapshot();
        let data_col = self.file_col.saturating_sub(PREFIX_COLS + 1);

        if self.insert_mode {
            self.editor_mut().insert_char(line, data_col, c);
        } else {
            self.editor_mut().overtype_char(line, data_col, c);
        }

        self.file_col += 1;
    }

    fn backspace_in_data(&mut self) {
        let line = self.file_line;
        if self.file_col <= PREFIX_COLS + 1 {
            // At start of data area — don't cross into prefix
            return;
        }

        self.ensure_screen_edit_snapshot();
        self.file_col -= 1;
        let data_col = self.file_col.saturating_sub(PREFIX_COLS + 1);
        self.editor_mut().delete_char(line, data_col);
    }

    // -- Enter processing (batch commit) --

    fn process_enter(&mut self) {
        // Collect and parse prefix commands
        let mut parsed: Vec<(usize, PrefixCommand)> = self
            .prefix_inputs
            .drain()
            .filter_map(|(line, text)| PrefixCommand::parse(&text).map(|cmd| (line, cmd)))
            .collect();

        // Sort by priority, then by line number.
        // Priority ordering ensures correct IBM XEDIT semantics:
        //   0: SetCurrent (/) — non-modifying, safe first
        //   1: Block markers (dd, cc, mm, "") — pair up before singles shift lines
        //   2: Single-line modifying (d, i, a, ", >, <) — descending line order to
        //      avoid line-number shifting
        //   3: Pending copy/move (c, m) — sets up pending operation
        //   4: Destination markers (f, p) — must run after the op they target
        parsed.sort_by(|(line_a, cmd_a), (line_b, cmd_b)| {
            let pri_a = prefix_priority(cmd_a);
            let pri_b = prefix_priority(cmd_b);
            pri_a.cmp(&pri_b).then_with(|| {
                if pri_a == 2 {
                    // Descending for single-line modifying ops (avoids line shift issues)
                    line_b.cmp(line_a)
                } else {
                    line_a.cmp(line_b)
                }
            })
        });

        // Execute in priority order
        for (line_num, cmd) in &parsed {
            match self.editor_mut().execute_prefix(*line_num, cmd) {
                Ok(result) => {
                    if let Some(msg) = result.message {
                        self.editor_mut().set_message(msg);
                    }
                }
                Err(e) => {
                    self.editor_mut().set_message(e.to_string());
                }
            }
        }

        // Also execute any pending command line text
        if !self.command_text.is_empty() {
            let text = self.command_text.clone();
            self.command_text.clear();
            self.command_cursor = 0;
            self.execute_command_text(&text);
        }

        // Sync cursor to editor state after prefix processing
        self.sync_file_cursor_to_editor();
    }

    // -- Input mode handling --

    fn handle_input_mode(&mut self, action: Action) {
        match action {
            Action::Char(c) => {
                self.input_text.push(c);
            }
            Action::Backspace => {
                self.input_text.pop();
            }
            Action::Enter => {
                if self.input_text.is_empty() {
                    // Empty line exits input mode
                    self.in_input_mode = false;
                } else {
                    let text = self.input_text.clone();
                    self.input_text.clear();
                    self.editor_mut().input_line(&text);
                    self.sync_file_cursor_to_editor();
                }
            }
            Action::Escape => {
                self.in_input_mode = false;
                self.input_text.clear();
            }
            _ => {}
        }
    }

    // -- PF key handling --

    fn handle_pf_key(&mut self, num: usize) {
        // If in input mode, PF keys exit input mode first
        if self.in_input_mode {
            self.in_input_mode = false;
            self.input_text.clear();
        }

        // Process any pending prefix commands first (like pressing Enter)
        if self.focus == CursorFocus::FileArea && !self.prefix_inputs.is_empty() {
            self.process_enter();
        }

        if let Some(cmd_text) = self.editor().pf_key(num) {
            let cmd_text = cmd_text.to_string();
            self.execute_command_text(&cmd_text);
        } else {
            self.editor_mut()
                .set_message(format!("PF{} is not defined", num));
        }
    }

    // -- Ring-aware helpers --

    /// Create a filesystem for new ring editors (CMS or native)
    fn create_fs(&self) -> Box<dyn xedit_core::filesystem::FileSystem> {
        #[cfg(feature = "cms")]
        if let Some(ref base_path) = self.cms_base_path {
            if let Ok(cms_fs) = crate::cms_support::create_cms_fs(base_path) {
                return Box::new(cms_fs);
            }
        }
        Box::new(xedit_core::filesystem::NativeFs)
    }

    /// Reset UI state for the current editor after switching ring files
    fn reset_for_current_editor(&mut self) {
        self.file_line = self.editor().current_line().max(1);
        self.file_col = 7;
        self.prefix_inputs.clear();
        self.command_text.clear();
        self.command_cursor = 0;
        self.history_index = None;
        self.file_area_edited = false;
        self.in_input_mode = false;
        self.input_text.clear();
        self.focus = CursorFocus::CommandLine;
    }

    /// Normalize a file identifier without creating a full filesystem.
    ///
    /// In CMS mode, filespecs pass through unchanged (CmsFs handles them
    /// natively).  In native mode, CMS-style input is converted via
    /// `NativeFs` (zero-sized, no I/O).
    fn normalize_file_id<'a>(&self, file_id: &'a str) -> std::borrow::Cow<'a, str> {
        #[cfg(feature = "cms")]
        if self.cms_base_path.is_some() {
            return std::borrow::Cow::Borrowed(file_id);
        }
        xedit_core::filesystem::NativeFs.normalize_file_id(file_id)
    }

    /// Open a file in the ring (or cycle/switch if already open)
    fn open_file_in_ring(&mut self, file_id: &str) {
        if file_id.is_empty() {
            // Bare "X" — cycle to next file
            if self.ring.len() > 1 {
                let _ = self.ring.cycle_next();
                self.reset_for_current_editor();
                let (pos, total) = self.ring.ring_position();
                self.editor_mut()
                    .set_message(format!("Ring {}/{}", pos, total));
            } else {
                self.editor_mut().set_message("Only one file in ring");
            }
            return;
        }

        // Normalize CMS-style filespecs (e.g. "PROFILE EXEC A" → "profile.exec")
        // without creating an expensive filesystem object.
        let normalized = self.normalize_file_id(file_id);

        // Check if file already in ring
        if self.ring.switch_to_file(&normalized) {
            self.reset_for_current_editor();
            self.editor_mut()
                .set_message(format!("Switched to {}", normalized));
            return;
        }

        // Only create the filesystem when we actually need to open a new file
        let fs = self.create_fs();
        if let Err(e) = self.ring.add_file_with_fs(&normalized, fs) {
            self.editor_mut().set_message(e.to_string());
            return;
        }
        #[cfg(feature = "rexx")]
        self.editor_mut().run_profile();
        self.reset_for_current_editor();
    }

    // -- Command execution --

    fn execute_command_text(&mut self, text: &str) {
        let trimmed = text.trim();

        // `?` (PF6): recall last command into command line
        if trimmed == "?" {
            if let Some(last) = self.editor().last_command() {
                self.command_text = last.to_string();
                self.command_cursor = self.command_text.len();
            } else {
                self.editor_mut().set_message("No commands in history");
            }
            return;
        }

        // `=` (PF9): re-execute last command
        if trimmed == "=" {
            if let Some(last) = self.editor().last_command() {
                let last = last.to_string();
                self.execute_command_text(&last);
            } else {
                self.editor_mut().set_message("No commands in history");
            }
            return;
        }

        // Record in history before executing
        self.editor_mut().push_history(trimmed);
        self.history_index = None;

        match parse_command(text) {
            Ok(cmd) => match self.editor_mut().execute(&cmd) {
                Ok(result) => match result.action {
                    CommandAction::Quit => {
                        self.ring.remove_current();
                        if self.ring.is_empty() {
                            self.should_quit = true;
                        } else {
                            self.reset_for_current_editor();
                        }
                    }
                    CommandAction::OpenFile(file_id) => {
                        self.open_file_in_ring(&file_id);
                    }
                    CommandAction::EnterInput => {
                        self.in_input_mode = true;
                        self.input_text.clear();
                    }
                    CommandAction::Transfer(target, count) => {
                        match self.ring.execute_transfer(&target, count) {
                            Ok(msg) => self.editor_mut().set_message(msg),
                            Err(e) => self.editor_mut().set_message(e.to_string()),
                        }
                    }
                    CommandAction::Refresh | CommandAction::Continue => {}
                },
                Err(e) => {
                    self.editor_mut().set_message(e.to_string());
                }
            },
            Err(xedit_err) => {
                if !self.try_cms_command(text) {
                    self.editor_mut().set_message(xedit_err);
                }
            }
        }
        if !self.should_quit && !self.ring.is_empty() {
            self.sync_file_cursor_to_editor();
            self.apply_cursor_request();
        }
    }

    /// Try to execute a command via the CMS command processor.
    /// Returns true if CMS recognized and handled the command.
    #[cfg(feature = "cms")]
    fn try_cms_command(&mut self, text: &str) -> bool {
        use cms_core::command::parse_cms_command;
        if let Some(ref mut proc) = self.cms_processor {
            if parse_cms_command(text).is_ok() {
                let result = proc.execute(text);
                let msg = if result.messages.is_empty() {
                    format!("RC={}", result.rc)
                } else {
                    result.messages.join(" | ")
                };
                self.editor_mut().set_message(msg);
                return true;
            }
        }
        false
    }

    #[cfg(not(feature = "cms"))]
    fn try_cms_command(&mut self, _text: &str) -> bool {
        false
    }

    fn apply_cursor_request(&mut self) {
        if let Some(req) = self.editor_mut().take_cursor_request() {
            match req {
                CursorRequest::Home => {
                    self.focus = CursorFocus::CommandLine;
                    self.file_area_edited = false;
                    self.command_cursor = 0;
                }
                CursorRequest::File { line, col } => {
                    let buf_len = self.editor().buffer().len();
                    self.focus = CursorFocus::FileArea;
                    self.file_line = line.max(1).min(buf_len);
                    self.file_col = PREFIX_COLS + 1 + col.saturating_sub(1);
                    let fl = self.file_line;
                    self.editor_mut().set_current_line(fl);
                }
            }
        }
    }

    /// Keep the file area cursor in sync with the editor's current line
    fn sync_file_cursor_to_editor(&mut self) {
        let current = self.editor().current_line();
        if current > 0 {
            self.file_line = current;
        } else {
            self.file_line = 1.min(self.editor().buffer().len());
        }
    }
}

/// Returns the execution priority for a prefix command.
/// Lower numbers execute first. See `process_enter()` for rationale.
fn prefix_priority(cmd: &PrefixCommand) -> u8 {
    match cmd {
        PrefixCommand::SetCurrent => 0,
        PrefixCommand::DeleteBlock
        | PrefixCommand::CopyBlock
        | PrefixCommand::MoveBlock
        | PrefixCommand::DuplicateBlock => 1,
        PrefixCommand::Delete
        | PrefixCommand::Insert(_)
        | PrefixCommand::Add(_)
        | PrefixCommand::Duplicate(_)
        | PrefixCommand::ShiftRight(_)
        | PrefixCommand::ShiftLeft(_) => 2,
        PrefixCommand::Copy | PrefixCommand::Move => 3,
        PrefixCommand::Following | PrefixCommand::Preceding => 4,
    }
}
