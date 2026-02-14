use crate::editor::Editor;
use crate::error::{Result, XeditError};
use std::path::Path;

/// The file ring — XEDIT's model for multiple open files.
///
/// In VM/CMS XEDIT, you could have multiple files open simultaneously
/// and cycle through them. Each file maintains its own editor state.
#[derive(Debug)]
pub struct Ring {
    editors: Vec<Editor>,
    current: usize,
}

impl Ring {
    pub fn new() -> Self {
        Self {
            editors: Vec::new(),
            current: 0,
        }
    }

    /// Add a new empty editor to the ring
    pub fn add_empty(&mut self) -> &mut Editor {
        self.editors.push(Editor::new());
        self.current = self.editors.len() - 1;
        &mut self.editors[self.current]
    }

    /// Add a file to the ring
    pub fn add_file(&mut self, path: &Path) -> Result<&mut Editor> {
        let mut editor = Editor::new();
        editor.load_file(path)?;
        self.editors.push(editor);
        self.current = self.editors.len() - 1;
        Ok(&mut self.editors[self.current])
    }

    /// Get the current editor
    pub fn current(&self) -> Option<&Editor> {
        self.editors.get(self.current)
    }

    /// Get the current editor mutably
    pub fn current_mut(&mut self) -> Option<&mut Editor> {
        self.editors.get_mut(self.current)
    }

    /// Cycle to the next file in the ring
    pub fn cycle_next(&mut self) -> Result<()> {
        if self.editors.is_empty() {
            return Err(XeditError::NoFile);
        }
        self.current = (self.current + 1) % self.editors.len();
        Ok(())
    }

    /// Cycle to the previous file in the ring
    pub fn prev(&mut self) -> Result<()> {
        if self.editors.is_empty() {
            return Err(XeditError::NoFile);
        }
        if self.current == 0 {
            self.current = self.editors.len() - 1;
        } else {
            self.current -= 1;
        }
        Ok(())
    }

    /// Remove the current editor from the ring
    pub fn remove_current(&mut self) {
        if !self.editors.is_empty() {
            self.editors.remove(self.current);
            if self.current >= self.editors.len() && !self.editors.is_empty() {
                self.current = self.editors.len() - 1;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.editors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.editors.is_empty()
    }

    pub fn current_index(&self) -> usize {
        self.current
    }
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}
