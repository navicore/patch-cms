use std::io;
use std::path::Path;

use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use xedit_core::command::{parse_command, CommandAction};
use xedit_core::editor::Editor;

use crate::input::{read_action, Action};
use crate::screen;

/// Application state
pub struct App {
    editor: Editor,
    command_text: String,
    input_text: String,
    in_input_mode: bool,
    should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            editor: Editor::new(),
            command_text: String::new(),
            input_text: String::new(),
            in_input_mode: false,
            should_quit: false,
        }
    }

    pub fn load_file(&mut self, path: &Path) -> xedit_core::error::Result<()> {
        self.editor.load_file(path)
    }

    /// Run the main event loop
    pub fn run(&mut self) -> io::Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        crossterm::execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Main loop
        let result = self.event_loop(&mut terminal);

        // Restore terminal
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
            // Update page size from terminal dimensions
            let size = terminal.size()?;
            self.editor
                .set_page_size(size.height.saturating_sub(3) as usize);

            // Render
            terminal.draw(|frame| {
                screen::render(
                    frame,
                    &self.editor,
                    &self.command_text,
                    self.in_input_mode,
                    &self.input_text,
                );
            })?;

            if self.should_quit {
                break;
            }

            // Handle input
            let current_text = if self.in_input_mode {
                &self.input_text
            } else {
                &self.command_text
            };

            let action = read_action(self.in_input_mode, current_text)?;
            self.handle_action(action);
        }

        Ok(())
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::SubmitCommand(text) => {
                if self.in_input_mode {
                    // Empty submit in input mode = exit input mode
                    self.in_input_mode = false;
                    self.input_text.clear();
                    return;
                }

                self.command_text.clear();
                if text.is_empty() {
                    return;
                }

                match parse_command(&text) {
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
            }
            Action::CommandChar(c) => {
                self.command_text.push(c);
            }
            Action::CommandBackspace => {
                self.command_text.pop();
            }
            Action::CursorUp => {
                let _ = self.editor.execute(&xedit_core::command::Command::Up(1));
            }
            Action::CursorDown => {
                let _ = self.editor.execute(&xedit_core::command::Command::Down(1));
            }
            Action::PageUp => {
                let _ = self.editor.execute(&xedit_core::command::Command::Backward(1));
            }
            Action::PageDown => {
                let _ = self.editor.execute(&xedit_core::command::Command::Forward(1));
            }
            Action::ForceQuit => {
                self.should_quit = true;
            }
            Action::InputSubmit(text) => {
                self.editor.input_line(&text);
                self.input_text.clear();
            }
            Action::InputChar(c) => {
                self.input_text.push(c);
            }
            Action::InputBackspace => {
                self.input_text.pop();
            }
            Action::None => {}
        }
    }
}
