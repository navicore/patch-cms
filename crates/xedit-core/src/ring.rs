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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn new_ring_is_empty() {
        let ring = Ring::new();
        assert_eq!(ring.len(), 0);
        assert!(ring.is_empty());
        assert!(ring.current().is_none());
    }

    #[test]
    fn add_empty() {
        let mut ring = Ring::new();
        ring.add_empty();
        assert_eq!(ring.len(), 1);
        assert!(ring.current().is_some());
        assert_eq!(ring.current_index(), 0);
    }

    #[test]
    fn add_multiple_empty() {
        let mut ring = Ring::new();
        ring.add_empty();
        ring.add_empty();
        ring.add_empty();
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.current_index(), 2);
    }

    #[test]
    fn add_file_success() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "hello").unwrap();
        writeln!(tmp, "world").unwrap();
        tmp.flush().unwrap();

        let mut ring = Ring::new();
        ring.add_file(tmp.path()).unwrap();
        assert_eq!(ring.len(), 1);

        let editor = ring.current().unwrap();
        assert_eq!(editor.buffer().lines().len(), 2);
    }

    #[test]
    fn add_file_not_found() {
        let mut ring = Ring::new();
        let result = ring.add_file(Path::new("/tmp/nonexistent_xedit_test_file.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn cycle_next_wraps() {
        let mut ring = Ring::new();
        ring.add_empty();
        ring.add_empty();
        ring.add_empty();
        assert_eq!(ring.current_index(), 2);

        ring.cycle_next().unwrap();
        assert_eq!(ring.current_index(), 0);

        ring.cycle_next().unwrap();
        assert_eq!(ring.current_index(), 1);

        ring.cycle_next().unwrap();
        assert_eq!(ring.current_index(), 2);
    }

    #[test]
    fn cycle_next_empty_ring() {
        let mut ring = Ring::new();
        let result = ring.cycle_next();
        assert!(result.is_err());
    }

    #[test]
    fn prev_wraps() {
        let mut ring = Ring::new();
        ring.add_empty();
        ring.add_empty();
        ring.add_empty();
        // current_index is 2 after adding 3
        assert_eq!(ring.current_index(), 2);

        ring.prev().unwrap();
        assert_eq!(ring.current_index(), 1);

        ring.prev().unwrap();
        assert_eq!(ring.current_index(), 0);

        // wraps to last
        ring.prev().unwrap();
        assert_eq!(ring.current_index(), 2);
    }

    #[test]
    fn prev_empty_ring() {
        let mut ring = Ring::new();
        let result = ring.prev();
        assert!(result.is_err());
    }

    #[test]
    fn remove_current_middle() {
        let mut ring = Ring::new();
        ring.add_empty();
        ring.add_empty();
        ring.add_empty();
        // cycle to index 1
        ring.current = 1;
        ring.remove_current();
        assert_eq!(ring.len(), 2);
        assert!(ring.current_index() <= 1);
    }

    #[test]
    fn remove_current_last() {
        let mut ring = Ring::new();
        ring.add_empty();
        ring.add_empty();
        // current_index is 1 (last)
        assert_eq!(ring.current_index(), 1);
        ring.remove_current();
        assert_eq!(ring.len(), 1);
        assert_eq!(ring.current_index(), 0);
    }

    #[test]
    fn remove_current_empty() {
        let mut ring = Ring::new();
        ring.remove_current(); // should not panic
        assert_eq!(ring.len(), 0);
    }
}
