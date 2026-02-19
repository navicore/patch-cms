use crate::editor::Editor;
use crate::error::{Result, XeditError};
use crate::filesystem::FileSystem;

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
    pub fn add_file(&mut self, file_id: &str) -> Result<&mut Editor> {
        let mut editor = Editor::new();
        editor.load_file(file_id)?;
        self.editors.push(editor);
        self.current = self.editors.len() - 1;
        Ok(&mut self.editors[self.current])
    }

    /// Add a file to the ring with a custom filesystem implementation
    pub fn add_file_with_fs(
        &mut self,
        file_id: &str,
        fs: Box<dyn FileSystem>,
    ) -> Result<&mut Editor> {
        let mut editor = Editor::with_fs(fs);
        editor.load_file(file_id)?;
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
            if self.editors.is_empty() {
                self.current = 0;
            } else if self.current >= self.editors.len() {
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

    /// Find a file in the ring by file_id and switch to it
    pub fn switch_to_file(&mut self, file_id: &str) -> bool {
        for (i, editor) in self.editors.iter().enumerate() {
            if editor.file_id() == Some(file_id) {
                self.current = i;
                return true;
            }
        }
        false
    }

    /// Return (1-based position, total) for status display
    pub fn ring_position(&self) -> (usize, usize) {
        if self.editors.is_empty() {
            (0, 0)
        } else {
            (self.current + 1, self.editors.len())
        }
    }

    /// Execute a TRANSFER command: copy lines from current editor to a target editor.
    /// Note: this intentionally copies without deleting from the source, diverging
    /// from IBM XEDIT where TRANSFER is a destructive move.
    pub fn execute_transfer(&mut self, target_file_id: &str, count: usize) -> Result<String> {
        if self.editors.is_empty() {
            return Err(XeditError::NoFile);
        }

        let source = &self.editors[self.current];
        if source.current_line() == 0 {
            return Err(XeditError::InvalidCommand(
                "Cannot TRANSFER at Top of File".to_string(),
            ));
        }
        if source.current_line() > source.buffer().len() {
            return Err(XeditError::InvalidCommand(
                "No lines to transfer".to_string(),
            ));
        }

        // Collect lines from source
        let available = source.buffer().len() - source.current_line() + 1;
        let actual_count = count.min(available);
        let mut lines = Vec::with_capacity(actual_count);
        for i in source.current_line()..source.current_line() + actual_count {
            let text = source
                .buffer()
                .line_text(i)
                .ok_or_else(|| {
                    XeditError::InvalidCommand(format!(
                        "TRANSFER: internal error — line {} missing",
                        i
                    ))
                })?
                .to_string();
            lines.push(text);
        }

        // Find target editor (excluding current)
        let target_idx = self
            .editors
            .iter()
            .enumerate()
            .find(|(i, ed)| *i != self.current && ed.file_id() == Some(target_file_id))
            .map(|(i, _)| i);

        let target_idx = match target_idx {
            Some(idx) => idx,
            None => {
                return Err(XeditError::InvalidCommand(format!(
                    "Target file not found in ring: {}",
                    target_file_id
                )));
            }
        };

        // Insert into target
        let actual_count = lines.len();
        let target = &mut self.editors[target_idx];
        let after_line = target.current_line();
        target.insert_lines_externally(after_line, lines);

        Ok(format!(
            "{} line(s) copied to {} (source unchanged)",
            actual_count, target_file_id
        ))
    }

    /// Add a new empty editor with a custom filesystem to the ring
    pub fn add_empty_with_fs(&mut self, fs: Box<dyn FileSystem>) -> &mut Editor {
        self.editors.push(Editor::with_fs(fs));
        self.current = self.editors.len() - 1;
        &mut self.editors[self.current]
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
        ring.add_file(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(ring.len(), 1);

        let editor = ring.current().unwrap();
        assert_eq!(editor.buffer().lines().len(), 2);
    }

    #[test]
    fn add_file_not_found() {
        let mut ring = Ring::new();
        let result = ring.add_file("/tmp/nonexistent_xedit_test_file.txt");
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
        // navigate to index 1 via public API
        ring.cycle_next().unwrap(); // 2 -> 0
        ring.cycle_next().unwrap(); // 0 -> 1
        assert_eq!(ring.current_index(), 1);
        ring.remove_current();
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.current_index(), 1);
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

    #[test]
    fn current_mut_modifies_editor() {
        let mut ring = Ring::new();
        ring.add_empty();
        assert!(ring.current_mut().is_some());

        let mut empty_ring = Ring::new();
        assert!(empty_ring.current_mut().is_none());
    }

    #[test]
    fn switch_to_file_found() {
        let mut tmp1 = NamedTempFile::new().unwrap();
        writeln!(tmp1, "file1").unwrap();
        tmp1.flush().unwrap();
        let mut tmp2 = NamedTempFile::new().unwrap();
        writeln!(tmp2, "file2").unwrap();
        tmp2.flush().unwrap();

        let mut ring = Ring::new();
        ring.add_file(tmp1.path().to_str().unwrap()).unwrap();
        ring.add_file(tmp2.path().to_str().unwrap()).unwrap();
        assert_eq!(ring.current_index(), 1);

        // Switch back to first file
        let found = ring.switch_to_file(tmp1.path().to_str().unwrap());
        assert!(found);
        assert_eq!(ring.current_index(), 0);
    }

    #[test]
    fn switch_to_file_not_found() {
        let mut ring = Ring::new();
        ring.add_empty();
        assert!(!ring.switch_to_file("nonexistent"));
    }

    #[test]
    fn ring_position_empty() {
        let ring = Ring::new();
        assert_eq!(ring.ring_position(), (0, 0));
    }

    #[test]
    fn ring_position_with_files() {
        let mut ring = Ring::new();
        ring.add_empty();
        ring.add_empty();
        ring.add_empty();
        assert_eq!(ring.ring_position(), (3, 3)); // current is last added
        ring.cycle_next().unwrap(); // wraps to 0
        assert_eq!(ring.ring_position(), (1, 3));
    }

    #[test]
    fn add_empty_with_fs() {
        let mut ring = Ring::new();
        let fs = Box::new(crate::filesystem::NativeFs);
        ring.add_empty_with_fs(fs);
        assert_eq!(ring.len(), 1);
        assert!(ring.current().is_some());
    }

    #[test]
    fn remove_all_then_add() {
        let mut ring = Ring::new();
        ring.add_empty();
        ring.add_empty();
        ring.remove_current();
        ring.remove_current();
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.current_index(), 0);

        ring.add_empty();
        assert_eq!(ring.current_index(), 0);
        assert!(ring.current().is_some());
    }

    // -- TRANSFER tests --

    #[test]
    fn transfer_basic() {
        let mut tmp1 = NamedTempFile::new().unwrap();
        writeln!(tmp1, "alpha").unwrap();
        writeln!(tmp1, "beta").unwrap();
        writeln!(tmp1, "gamma").unwrap();
        tmp1.flush().unwrap();
        let mut tmp2 = NamedTempFile::new().unwrap();
        writeln!(tmp2, "one").unwrap();
        writeln!(tmp2, "two").unwrap();
        tmp2.flush().unwrap();

        let path1 = tmp1.path().to_str().unwrap().to_string();
        let path2 = tmp2.path().to_str().unwrap().to_string();

        let mut ring = Ring::new();
        ring.add_file(&path1).unwrap();
        ring.add_file(&path2).unwrap();
        // Switch to source (file 1)
        ring.cycle_next().unwrap();
        assert_eq!(ring.current_index(), 0);

        // Transfer 1 line from source to target
        let msg = ring.execute_transfer(&path2, 1).unwrap();
        assert!(msg.contains("1 line(s) copied to"));

        // Check target got the line with correct content and position
        let target = &ring.editors[1];
        assert_eq!(target.buffer().len(), 3); // was 2, now 3
                                              // Source was at line 1 ("alpha"), target was at line 1 ("one"),
                                              // so "alpha" is inserted after target's current line 1
        assert_eq!(target.buffer().line_text(2), Some("alpha"));
        // current_line should advance to last inserted line
        assert_eq!(target.current_line(), 2);
    }

    #[test]
    fn transfer_multiple_lines() {
        let mut tmp1 = NamedTempFile::new().unwrap();
        writeln!(tmp1, "alpha").unwrap();
        writeln!(tmp1, "beta").unwrap();
        writeln!(tmp1, "gamma").unwrap();
        tmp1.flush().unwrap();
        let mut tmp2 = NamedTempFile::new().unwrap();
        writeln!(tmp2, "one").unwrap();
        tmp2.flush().unwrap();

        let path1 = tmp1.path().to_str().unwrap().to_string();
        let path2 = tmp2.path().to_str().unwrap().to_string();

        let mut ring = Ring::new();
        ring.add_file(&path1).unwrap();
        ring.add_file(&path2).unwrap();
        ring.cycle_next().unwrap();

        let msg = ring.execute_transfer(&path2, 2).unwrap();
        assert!(msg.contains("2 line(s) copied to"));

        let target = &ring.editors[1];
        assert_eq!(target.buffer().len(), 3); // 1 original + 2 transferred
    }

    #[test]
    fn transfer_clamps_to_available() {
        let mut tmp1 = NamedTempFile::new().unwrap();
        writeln!(tmp1, "alpha").unwrap();
        tmp1.flush().unwrap();
        let mut tmp2 = NamedTempFile::new().unwrap();
        writeln!(tmp2, "one").unwrap();
        tmp2.flush().unwrap();

        let path1 = tmp1.path().to_str().unwrap().to_string();
        let path2 = tmp2.path().to_str().unwrap().to_string();

        let mut ring = Ring::new();
        ring.add_file(&path1).unwrap();
        ring.add_file(&path2).unwrap();
        ring.cycle_next().unwrap();

        // Request 100 but only 1 available
        let msg = ring.execute_transfer(&path2, 100).unwrap();
        assert!(msg.contains("1 line(s) copied to"));
    }

    #[test]
    fn transfer_target_not_found() {
        let mut tmp1 = NamedTempFile::new().unwrap();
        writeln!(tmp1, "alpha").unwrap();
        tmp1.flush().unwrap();

        let path1 = tmp1.path().to_str().unwrap().to_string();

        let mut ring = Ring::new();
        ring.add_file(&path1).unwrap();

        let result = ring.execute_transfer("nonexistent.txt", 1);
        assert!(result.is_err());
    }

    #[test]
    fn transfer_at_tof_errors() {
        let mut tmp1 = NamedTempFile::new().unwrap();
        writeln!(tmp1, "alpha").unwrap();
        tmp1.flush().unwrap();
        let mut tmp2 = NamedTempFile::new().unwrap();
        writeln!(tmp2, "one").unwrap();
        tmp2.flush().unwrap();

        let path1 = tmp1.path().to_str().unwrap().to_string();
        let path2 = tmp2.path().to_str().unwrap().to_string();

        let mut ring = Ring::new();
        ring.add_file(&path1).unwrap();
        ring.add_file(&path2).unwrap();
        ring.cycle_next().unwrap();

        // Move source to TOF
        ring.current_mut().unwrap().set_current_line(0);
        let result = ring.execute_transfer(&path2, 1);
        assert!(result.is_err());
    }

    #[test]
    fn transfer_does_not_remove_from_source() {
        let mut tmp1 = NamedTempFile::new().unwrap();
        writeln!(tmp1, "alpha").unwrap();
        writeln!(tmp1, "beta").unwrap();
        tmp1.flush().unwrap();
        let mut tmp2 = NamedTempFile::new().unwrap();
        writeln!(tmp2, "one").unwrap();
        tmp2.flush().unwrap();

        let path1 = tmp1.path().to_str().unwrap().to_string();
        let path2 = tmp2.path().to_str().unwrap().to_string();

        let mut ring = Ring::new();
        ring.add_file(&path1).unwrap();
        ring.add_file(&path2).unwrap();
        ring.cycle_next().unwrap();

        ring.execute_transfer(&path2, 1).unwrap();

        // Source should still have its lines
        let source = ring.current().unwrap();
        assert_eq!(source.buffer().len(), 2);
        assert_eq!(source.buffer().line_text(1), Some("alpha"));
    }
}
