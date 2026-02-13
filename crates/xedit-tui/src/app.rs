use std::collections::HashMap;
use std::io;
use std::path::Path;

use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use xedit_core::command::{parse_command, Command, CommandAction};
use xedit_core::editor::Editor;
use xedit_core::prefix::PrefixCommand;

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
    editor: Editor,
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

    // Insert vs overtype mode for data area
    insert_mode: bool,

    // Input mode (from INPUT command)
    in_input_mode: bool,
    input_text: String,

    should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            editor: Editor::new(),
            focus: CursorFocus::CommandLine,
            command_text: String::new(),
            command_cursor: 0,
            file_line: 1,
            file_col: 7, // start in data area
            prefix_inputs: HashMap::new(),
            insert_mode: false,
            in_input_mode: false,
            input_text: String::new(),
            should_quit: false,
        }
    }

    pub fn load_file(&mut self, path: &Path) -> xedit_core::error::Result<()> {
        self.editor.load_file(path)?;
        self.file_line = self.editor.current_line().max(1);
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
            self.editor
                .set_page_size(size.height.saturating_sub(3) as usize);

            terminal.draw(|frame| {
                screen::render(
                    frame,
                    &self.editor,
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
                // Arrow up in command line: move current line up
                let _ = self.editor.execute(&Command::Up(1));
                self.sync_file_cursor_to_editor();
            }
            Action::ArrowDown => {
                let _ = self.editor.execute(&Command::Down(1));
                self.sync_file_cursor_to_editor();
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
                self.file_line = self.editor.current_line().max(1);
                self.file_col = 7; // first data column
            }
            Action::PageUp => {
                let _ = self.editor.execute(&Command::Backward(1));
                self.sync_file_cursor_to_editor();
            }
            Action::PageDown => {
                let _ = self.editor.execute(&Command::Forward(1));
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
        let buf_len = self.editor.buffer().len();

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
                if !in_prefix && self.file_line >= 1 && self.file_line <= buf_len {
                    let data_col = self.file_col.saturating_sub(PREFIX_COLS + 1);
                    self.editor.delete_char(self.file_line, data_col);
                }
            }
            Action::Enter => {
                self.process_enter();
            }
            Action::Tab => {
                // Tab: cycle prefix → data → command line
                if in_prefix {
                    self.file_col = 7; // jump to data area
                } else {
                    self.focus = CursorFocus::CommandLine;
                }
            }
            Action::BackTab => {
                // Shift-Tab: reverse cycle
                if in_prefix {
                    self.focus = CursorFocus::CommandLine;
                } else {
                    self.file_col = 1; // jump to prefix area
                }
            }
            Action::ArrowUp => {
                if self.file_line > 1 {
                    self.file_line -= 1;
                    self.editor.set_current_line(self.file_line);
                }
            }
            Action::ArrowDown => {
                if self.file_line < buf_len {
                    self.file_line += 1;
                    self.editor.set_current_line(self.file_line);
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
                } else if let Some(text) = self.editor.buffer().line_text(self.file_line) {
                    self.file_col = PREFIX_COLS + 1 + text.len();
                }
            }
            Action::PageUp => {
                let _ = self.editor.execute(&Command::Backward(1));
                self.sync_file_cursor_to_editor();
            }
            Action::PageDown => {
                let _ = self.editor.execute(&Command::Forward(1));
                self.sync_file_cursor_to_editor();
            }
            Action::InsertToggle => {
                self.insert_mode = !self.insert_mode;
                let mode = if self.insert_mode {
                    "Insert mode"
                } else {
                    "Overtype mode"
                };
                self.editor.set_message(mode);
            }
            Action::Escape => {
                // Escape in file area: return to command line, clear pending prefixes
                self.prefix_inputs.clear();
                self.focus = CursorFocus::CommandLine;
            }
            _ => {}
        }
    }

    // -- Prefix area editing --

    fn type_in_prefix(&mut self, c: char) {
        let line = self.file_line;
        if line == 0 || line > self.editor.buffer().len() {
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

    fn type_in_data(&mut self, c: char) {
        let line = self.file_line;
        if line == 0 || line > self.editor.buffer().len() {
            return;
        }

        let data_col = self.file_col.saturating_sub(PREFIX_COLS + 1);

        if self.insert_mode {
            self.editor.insert_char(line, data_col, c);
        } else {
            self.editor.overtype_char(line, data_col, c);
        }

        self.file_col += 1;
    }

    fn backspace_in_data(&mut self) {
        let line = self.file_line;
        if self.file_col <= PREFIX_COLS + 1 {
            // At start of data area — don't cross into prefix
            return;
        }

        self.file_col -= 1;
        let data_col = self.file_col.saturating_sub(PREFIX_COLS + 1);
        self.editor.delete_char(line, data_col);
    }

    // -- Enter processing (batch commit) --

    fn process_enter(&mut self) {
        // Collect prefix commands
        let mut prefixes: Vec<(usize, String)> = self.prefix_inputs.drain().collect();
        prefixes.sort_by_key(|(line, _)| *line);

        // Process prefix commands in line order
        for (line_num, text) in &prefixes {
            if let Some(cmd) = PrefixCommand::parse(text) {
                match self.editor.execute_prefix(*line_num, &cmd) {
                    Ok(result) => {
                        if let Some(msg) = result.message {
                            self.editor.set_message(msg);
                        }
                    }
                    Err(e) => {
                        self.editor.set_message(e.to_string());
                    }
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
                    self.editor.input_line(&text);
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

        if let Some(cmd_text) = self.editor.pf_key(num) {
            let cmd_text = cmd_text.to_string();
            self.execute_command_text(&cmd_text);
        } else {
            self.editor
                .set_message(format!("PF{} is not defined", num));
        }
    }

    // -- Helpers --

    fn execute_command_text(&mut self, text: &str) {
        match parse_command(text) {
            Ok(cmd) => match self.editor.execute(&cmd) {
                Ok(result) => match result.action {
                    CommandAction::Quit => self.should_quit = true,
                    CommandAction::EnterInput => {
                        self.in_input_mode = true;
                        self.input_text.clear();
                    }
                    CommandAction::Refresh | CommandAction::Continue => {}
                },
                Err(e) => {
                    self.editor.set_message(e.to_string());
                }
            },
            Err(e) => {
                self.editor.set_message(e);
            }
        }
        self.sync_file_cursor_to_editor();
    }

    /// Keep the file area cursor in sync with the editor's current line
    fn sync_file_cursor_to_editor(&mut self) {
        let current = self.editor.current_line();
        if current > 0 {
            self.file_line = current;
        } else {
            self.file_line = 1.min(self.editor.buffer().len());
        }
    }
}
