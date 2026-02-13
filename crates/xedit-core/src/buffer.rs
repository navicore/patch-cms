/// Record format of the file
#[derive(Debug, Clone, PartialEq)]
pub enum RecordFormat {
    /// Variable length records
    Variable,
    /// Fixed length records (padded to LRECL)
    Fixed,
}

/// A single line in the buffer
#[derive(Debug, Clone)]
pub struct Line {
    text: String,
}

impl Line {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// The text buffer containing all lines of a file.
///
/// Line numbers are 1-based. Line 0 represents "Top of File" (TOF),
/// a virtual position before all lines.
#[derive(Debug)]
pub struct Buffer {
    lines: Vec<Line>,
    recfm: RecordFormat,
    lrecl: usize,
    modified: bool,
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            recfm: RecordFormat::Variable,
            lrecl: 80,
            modified: false,
        }
    }

    pub fn from_lines(lines: Vec<String>) -> Self {
        let max_len = lines.iter().map(|l| l.len()).max().unwrap_or(80);
        Self {
            lines: lines.into_iter().map(Line::new).collect(),
            recfm: RecordFormat::Variable,
            lrecl: max_len.max(80),
            modified: false,
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Get a line by 1-based line number
    pub fn get(&self, line_num: usize) -> Option<&Line> {
        if line_num == 0 || line_num > self.lines.len() {
            None
        } else {
            Some(&self.lines[line_num - 1])
        }
    }

    /// Get a mutable line by 1-based line number
    pub fn get_mut(&mut self, line_num: usize) -> Option<&mut Line> {
        if line_num == 0 || line_num > self.lines.len() {
            None
        } else {
            self.modified = true;
            Some(&mut self.lines[line_num - 1])
        }
    }

    /// Insert a line after the given 1-based line number (0 = insert at top)
    pub fn insert_after(&mut self, after_line: usize, text: impl Into<String>) {
        let idx = after_line.min(self.lines.len());
        self.lines.insert(idx, Line::new(text));
        self.modified = true;
    }

    /// Insert multiple lines after the given position
    pub fn insert_lines_after(&mut self, after_line: usize, texts: Vec<String>) {
        let idx = after_line.min(self.lines.len());
        for (i, text) in texts.into_iter().enumerate() {
            self.lines.insert(idx + i, Line::new(text));
        }
        if !self.lines.is_empty() {
            self.modified = true;
        }
    }

    /// Delete a line by 1-based line number
    pub fn delete(&mut self, line_num: usize) -> Option<Line> {
        if line_num == 0 || line_num > self.lines.len() {
            None
        } else {
            self.modified = true;
            Some(self.lines.remove(line_num - 1))
        }
    }

    /// Delete a range of lines (inclusive, 1-based)
    pub fn delete_range(&mut self, from: usize, to: usize) -> Vec<Line> {
        if from == 0 || from > self.lines.len() || to < from {
            return Vec::new();
        }
        let to = to.min(self.lines.len());
        self.modified = true;
        self.lines.drain((from - 1)..to).collect()
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn clear_modified(&mut self) {
        self.modified = false;
    }

    pub fn recfm(&self) -> &RecordFormat {
        &self.recfm
    }

    pub fn lrecl(&self) -> usize {
        self.lrecl
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Get line text by 1-based line number
    pub fn line_text(&self, line_num: usize) -> Option<&str> {
        self.get(line_num).map(|l| l.text())
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer() {
        let buf = Buffer::new();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert!(buf.get(0).is_none());
        assert!(buf.get(1).is_none());
    }

    #[test]
    fn from_lines() {
        let buf = Buffer::from_lines(vec!["hello".into(), "world".into()]);
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.line_text(1), Some("hello"));
        assert_eq!(buf.line_text(2), Some("world"));
        assert!(buf.line_text(0).is_none());
        assert!(buf.line_text(3).is_none());
    }

    #[test]
    fn insert_and_delete() {
        let mut buf = Buffer::new();
        buf.insert_after(0, "first");
        buf.insert_after(1, "second");
        buf.insert_after(1, "middle");
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.line_text(1), Some("first"));
        assert_eq!(buf.line_text(2), Some("middle"));
        assert_eq!(buf.line_text(3), Some("second"));

        buf.delete(2);
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.line_text(2), Some("second"));
    }

    #[test]
    fn delete_range() {
        let mut buf = Buffer::from_lines(vec![
            "a".into(),
            "b".into(),
            "c".into(),
            "d".into(),
            "e".into(),
        ]);
        let removed = buf.delete_range(2, 4);
        assert_eq!(removed.len(), 3);
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.line_text(1), Some("a"));
        assert_eq!(buf.line_text(2), Some("e"));
    }
}
