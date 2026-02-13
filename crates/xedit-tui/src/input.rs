use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

/// Raw input actions — focus-agnostic.
/// The app layer decides what these mean based on current focus.
#[derive(Debug)]
pub enum Action {
    Char(char),
    Backspace,
    Delete,
    Enter,
    Tab,
    BackTab,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    PageUp,
    PageDown,
    Home,
    End,
    InsertToggle,
    Escape,
    ForceQuit,
    None,
}

/// Read a key event and translate to a raw Action.
pub fn read_action() -> std::io::Result<Action> {
    if !event::poll(Duration::from_millis(100))? {
        return Ok(Action::None);
    }

    match event::read()? {
        Event::Key(KeyEvent {
            code, modifiers, ..
        }) => {
            if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(Action::ForceQuit);
            }

            match code {
                KeyCode::Char(c) => Ok(Action::Char(c)),
                KeyCode::Backspace => Ok(Action::Backspace),
                KeyCode::Delete => Ok(Action::Delete),
                KeyCode::Enter => Ok(Action::Enter),
                KeyCode::Tab => {
                    if modifiers.contains(KeyModifiers::SHIFT) {
                        Ok(Action::BackTab)
                    } else {
                        Ok(Action::Tab)
                    }
                }
                KeyCode::Up => Ok(Action::ArrowUp),
                KeyCode::Down => Ok(Action::ArrowDown),
                KeyCode::Left => Ok(Action::ArrowLeft),
                KeyCode::Right => Ok(Action::ArrowRight),
                KeyCode::PageUp => Ok(Action::PageUp),
                KeyCode::PageDown => Ok(Action::PageDown),
                KeyCode::Home => Ok(Action::Home),
                KeyCode::End => Ok(Action::End),
                KeyCode::Insert => Ok(Action::InsertToggle),
                KeyCode::Esc => Ok(Action::Escape),
                _ => Ok(Action::None),
            }
        }
        _ => Ok(Action::None),
    }
}
