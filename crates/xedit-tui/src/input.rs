use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

/// High-level input actions for the XEDIT TUI
#[derive(Debug)]
pub enum Action {
    /// Submit the command line text
    SubmitCommand(String),
    /// A character typed into the command line
    CommandChar(char),
    /// Backspace in command line
    CommandBackspace,
    /// Move cursor up (arrow key)
    CursorUp,
    /// Move cursor down (arrow key)
    CursorDown,
    /// Page up (PF7 / PgUp)
    PageUp,
    /// Page down (PF8 / PgDn)
    PageDown,
    /// Quit (Ctrl-C)
    ForceQuit,
    /// Submit input mode line
    InputSubmit(String),
    /// Input mode: character typed
    InputChar(char),
    /// Input mode: backspace
    InputBackspace,
    /// No action (timeout)
    None,
}

/// Read a key event and translate to an Action.
///
/// `in_command_line`: whether we're editing the command line
/// `in_input_mode`: whether we're in INPUT mode
/// `current_text`: the current text in the active input area
pub fn read_action(
    in_input_mode: bool,
    current_text: &str,
) -> std::io::Result<Action> {
    if !event::poll(Duration::from_millis(100))? {
        return Ok(Action::None);
    }

    match event::read()? {
        Event::Key(KeyEvent {
            code,
            modifiers,
            ..
        }) => {
            // Ctrl-C always force quits
            if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(Action::ForceQuit);
            }

            if in_input_mode {
                match code {
                    KeyCode::Enter => {
                        if current_text.is_empty() {
                            // Empty line exits input mode
                            return Ok(Action::SubmitCommand(String::new()));
                        }
                        Ok(Action::InputSubmit(current_text.to_string()))
                    }
                    KeyCode::Char(c) => Ok(Action::InputChar(c)),
                    KeyCode::Backspace => Ok(Action::InputBackspace),
                    KeyCode::Esc => Ok(Action::SubmitCommand(String::new())), // exit input mode
                    _ => Ok(Action::None),
                }
            } else {
                // Command line mode
                match code {
                    KeyCode::Enter => Ok(Action::SubmitCommand(current_text.to_string())),
                    KeyCode::Char(c) => Ok(Action::CommandChar(c)),
                    KeyCode::Backspace => Ok(Action::CommandBackspace),
                    KeyCode::Up => Ok(Action::CursorUp),
                    KeyCode::Down => Ok(Action::CursorDown),
                    KeyCode::PageUp => Ok(Action::PageUp),
                    KeyCode::PageDown => Ok(Action::PageDown),
                    KeyCode::Esc => Ok(Action::ForceQuit),
                    _ => Ok(Action::None),
                }
            }
        }
        Event::Resize(_, _) => Ok(Action::None), // ratatui handles resize
        _ => Ok(Action::None),
    }
}
