use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;

use crate::error::{CmsError, Result};
use crate::filespec::FileSpec;
use crate::minidisk::{AccessMode, Minidisk};

/// Metadata about a file on a CMS minidisk.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub spec: FileSpec,
    pub size_bytes: u64,
    pub line_count: usize,
}

/// Count lines in a file using buffered I/O (avoids reading entire file into memory).
/// Lines that fail to decode (e.g. invalid UTF-8) are skipped rather than counted.
fn count_lines(path: &Path) -> std::io::Result<usize> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    Ok(reader.lines().map_while(|l| l.ok()).count())
}

/// Returns true if the path is a regular file (not a symlink).
fn is_regular_file(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type().is_file(),
        Err(_) => false,
    }
}

/// Multi-disk CMS file system.
///
/// Manages a set of minidisks (A-Z) and provides CMS-style file operations.
/// When a filemode letter is `*`, disks are searched in A-Z order.
pub struct CmsFileSystem {
    disks: BTreeMap<char, Minidisk>,
}

impl CmsFileSystem {
    /// Create an empty filesystem with no disks accessed.
    pub fn new() -> Self {
        CmsFileSystem {
            disks: BTreeMap::new(),
        }
    }

    /// Create a filesystem with a default read-write disk A at `base_path/a/`.
    pub fn with_default_disk(base_path: &Path) -> Result<Self> {
        let mut fs = Self::new();
        let a_path = base_path.join("a");
        fs.access_disk('A', a_path, AccessMode::ReadWrite)?;
        Ok(fs)
    }

    /// Mount a disk: map a filemode letter to a directory with an access mode.
    pub fn access_disk(
        &mut self,
        letter: char,
        path: impl Into<std::path::PathBuf>,
        access: AccessMode,
    ) -> Result<()> {
        if !letter.is_ascii_alphabetic() {
            return Err(CmsError::InvalidFileSpec(format!(
                "Disk letter must be A-Z, got '{}'",
                letter
            )));
        }
        let letter = letter.to_ascii_uppercase();
        let disk = Minidisk::new(letter, path.into(), access);
        disk.ensure_dir()?;
        self.disks.insert(letter, disk);
        Ok(())
    }

    /// Unmount a disk.
    pub fn release_disk(&mut self, letter: char) {
        self.disks.remove(&letter.to_ascii_uppercase());
    }

    /// Get a reference to a mounted disk.
    pub fn disk(&self, letter: char) -> Option<&Minidisk> {
        self.disks.get(&letter.to_ascii_uppercase())
    }

    /// Read file contents. If the filemode is `*`, search disks A-Z.
    pub fn read_file(&self, spec: &FileSpec) -> Result<String> {
        let path = self.resolve_file(spec)?;
        let content = std::fs::read_to_string(&path)?;
        Ok(content)
    }

    /// Write file contents (create or overwrite).
    pub fn write_file(&self, spec: &FileSpec, content: &str) -> Result<()> {
        if spec.has_wildcards() {
            return Err(CmsError::InvalidFileSpec(
                "Cannot write to a wildcard filespec".into(),
            ));
        }
        let disk = self.get_writable_disk(spec.mode_letter())?;
        let path = disk.file_path(spec.filename(), spec.filetype());
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Get file info (existence check + metadata).
    pub fn state(&self, spec: &FileSpec) -> Result<FileInfo> {
        let path = self.resolve_file(spec)?;
        let metadata = std::fs::metadata(&path)?;
        let line_count = count_lines(&path)?;
        let resolved_spec = self.resolve_spec(spec)?;

        Ok(FileInfo {
            spec: resolved_spec,
            size_bytes: metadata.len(),
            line_count,
        })
    }

    /// List files matching a pattern. Wildcards `*` match any component.
    pub fn listfile(&self, pattern: &FileSpec) -> Result<Vec<FileInfo>> {
        let mut results = Vec::new();

        let disks_to_search = self.disks_for_letter(pattern.mode_letter());

        for disk in disks_to_search {
            let entries = match std::fs::read_dir(disk.path()) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if !is_regular_file(&path) {
                    continue;
                }

                if let Some(spec) = self.path_to_spec(&path, disk.letter()) {
                    if pattern.matches(&spec) {
                        let metadata = std::fs::metadata(&path)?;
                        let line_count = count_lines(&path)?;
                        results.push(FileInfo {
                            spec,
                            size_bytes: metadata.len(),
                            line_count,
                        });
                    }
                }
            }
        }

        // Sort by filename then filetype for deterministic output
        results.sort_by(|a, b| {
            a.spec
                .filename()
                .cmp(b.spec.filename())
                .then_with(|| a.spec.filetype().cmp(b.spec.filetype()))
                .then_with(|| a.spec.mode_letter().cmp(&b.spec.mode_letter()))
        });

        Ok(results)
    }

    /// Delete a file.
    pub fn erase(&self, spec: &FileSpec) -> Result<()> {
        if spec.has_wildcards() {
            return Err(CmsError::InvalidFileSpec(
                "Cannot erase with wildcard filespec".into(),
            ));
        }
        let disk = self.get_writable_disk(spec.mode_letter())?;
        let path = disk.file_path(spec.filename(), spec.filetype());
        if !is_regular_file(&path) {
            return Err(CmsError::FileNotFound(spec.to_string()));
        }
        std::fs::remove_file(&path)?;
        Ok(())
    }

    /// Copy a file from one spec to another. Supports cross-disk copies.
    pub fn copyfile(&self, from: &FileSpec, to: &FileSpec) -> Result<()> {
        if from.has_wildcards() || to.has_wildcards() {
            return Err(CmsError::InvalidFileSpec(
                "Cannot copy with wildcard filespec".into(),
            ));
        }
        let source_path = self.resolve_file(from)?;
        let dest_disk = self.get_writable_disk(to.mode_letter())?;
        let dest_path = dest_disk.file_path(to.filename(), to.filetype());
        std::fs::copy(&source_path, &dest_path)?;
        Ok(())
    }

    /// Rename a file. Both specs must be on the same disk.
    ///
    /// Note: The destination-exists check is best-effort (TOCTOU race with
    /// concurrent processes). Acceptable for a single-user CMS environment.
    pub fn rename(&self, from: &FileSpec, to: &FileSpec) -> Result<()> {
        if from.has_wildcards() || to.has_wildcards() {
            return Err(CmsError::InvalidFileSpec(
                "Cannot rename with wildcard filespec".into(),
            ));
        }
        if from.mode_letter() != to.mode_letter() {
            return Err(CmsError::InvalidFileSpec(
                "RENAME requires both files on the same disk".into(),
            ));
        }
        let disk = self.get_writable_disk(from.mode_letter())?;
        let src = disk.file_path(from.filename(), from.filetype());
        if !is_regular_file(&src) {
            return Err(CmsError::FileNotFound(from.to_string()));
        }
        let dst = disk.file_path(to.filename(), to.filetype());
        if is_regular_file(&dst) {
            return Err(CmsError::FileExists(to.to_string()));
        }
        std::fs::rename(&src, &dst)?;
        Ok(())
    }

    // --- internal helpers ---

    /// Get the list of disks to search for a given filemode letter.
    /// `*` means all disks in A-Z order; a specific letter means just that disk.
    fn disks_for_letter(&self, letter: char) -> Vec<&Minidisk> {
        if letter == '*' {
            self.disks.values().collect()
        } else {
            self.disks.get(&letter).into_iter().collect()
        }
    }

    /// Find the on-disk path of a file, searching multiple disks if needed.
    fn resolve_file(&self, spec: &FileSpec) -> Result<std::path::PathBuf> {
        if spec.has_wildcards() {
            return Err(CmsError::InvalidFileSpec(
                "Cannot resolve a wildcard filespec to a single file".into(),
            ));
        }

        let disks = self.disks_for_letter(spec.mode_letter());
        if disks.is_empty() {
            return Err(CmsError::DiskNotAccessed(spec.mode_letter()));
        }

        for disk in &disks {
            let path = disk.file_path(spec.filename(), spec.filetype());
            if is_regular_file(&path) {
                return Ok(path);
            }
        }

        Err(CmsError::FileNotFound(spec.to_string()))
    }

    /// Resolve a filespec to one with a concrete disk letter (after searching).
    fn resolve_spec(&self, spec: &FileSpec) -> Result<FileSpec> {
        let disks = self.disks_for_letter(spec.mode_letter());
        if disks.is_empty() {
            return Err(CmsError::DiskNotAccessed(spec.mode_letter()));
        }

        for disk in &disks {
            if disk.file_exists(spec.filename(), spec.filetype()) {
                return FileSpec::new(
                    spec.filename(),
                    spec.filetype(),
                    &format!("{}{}", disk.letter(), spec.mode_number()),
                );
            }
        }

        Err(CmsError::FileNotFound(spec.to_string()))
    }

    /// Get a writable disk or return ReadOnly error.
    fn get_writable_disk(&self, letter: char) -> Result<&Minidisk> {
        let letter = letter.to_ascii_uppercase();
        let disk = self
            .disks
            .get(&letter)
            .ok_or(CmsError::DiskNotAccessed(letter))?;
        if !disk.is_writable() {
            return Err(CmsError::ReadOnly(letter));
        }
        Ok(disk)
    }

    /// Try to parse an on-disk path back into a FileSpec.
    fn path_to_spec(&self, path: &Path, disk_letter: char) -> Option<FileSpec> {
        let stem = path.file_stem()?.to_str()?;
        let ext = path.extension()?.to_str()?;
        FileSpec::new(stem, ext, &format!("{}1", disk_letter)).ok()
    }
}

impl Default for CmsFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_fs() -> (TempDir, CmsFileSystem) {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        (dir, fs)
    }

    #[test]
    fn write_then_read() {
        let (_dir, fs) = setup_fs();
        let spec = FileSpec::parse("PROFILE EXEC A").unwrap();
        fs.write_file(&spec, "Hello CMS\n").unwrap();
        let content = fs.read_file(&spec).unwrap();
        assert_eq!(content, "Hello CMS\n");
    }

    #[test]
    fn read_nonexistent() {
        let (_dir, fs) = setup_fs();
        let spec = FileSpec::parse("NOFILE DATA A").unwrap();
        let err = fs.read_file(&spec).unwrap_err();
        assert!(matches!(err, CmsError::FileNotFound(_)));
    }

    #[test]
    fn write_to_readonly_disk() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('B', dir.path().join("b"), AccessMode::ReadOnly)
            .unwrap();
        let spec = FileSpec::parse("TEST DATA B").unwrap();
        let err = fs.write_file(&spec, "data").unwrap_err();
        assert!(matches!(err, CmsError::ReadOnly('B')));
    }

    #[test]
    fn state_existing_file() {
        let (_dir, fs) = setup_fs();
        let spec = FileSpec::parse("MYFILE DATA A").unwrap();
        fs.write_file(&spec, "line one\nline two\n").unwrap();
        let info = fs.state(&spec).unwrap();
        assert_eq!(info.spec.filename(), "MYFILE");
        assert_eq!(info.line_count, 2);
        assert!(info.size_bytes > 0);
    }

    #[test]
    fn state_nonexistent() {
        let (_dir, fs) = setup_fs();
        let spec = FileSpec::parse("NOFILE DATA A").unwrap();
        let err = fs.state(&spec).unwrap_err();
        assert!(matches!(err, CmsError::FileNotFound(_)));
    }

    #[test]
    fn listfile_exact() {
        let (_dir, fs) = setup_fs();
        let spec = FileSpec::parse("FILE1 DATA A").unwrap();
        fs.write_file(&spec, "data").unwrap();
        let pattern = FileSpec::parse("FILE1 DATA A").unwrap();
        let files = fs.listfile(&pattern).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].spec.filename(), "FILE1");
    }

    #[test]
    fn listfile_wildcard_filename() {
        let (_dir, fs) = setup_fs();
        fs.write_file(&FileSpec::parse("FILE1 EXEC A").unwrap(), "a")
            .unwrap();
        fs.write_file(&FileSpec::parse("FILE2 EXEC A").unwrap(), "b")
            .unwrap();
        fs.write_file(&FileSpec::parse("FILE3 DATA A").unwrap(), "c")
            .unwrap();
        let pattern = FileSpec::parse("* EXEC A").unwrap();
        let files = fs.listfile(&pattern).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn listfile_wildcard_filetype() {
        let (_dir, fs) = setup_fs();
        fs.write_file(&FileSpec::parse("PROFILE EXEC A").unwrap(), "a")
            .unwrap();
        fs.write_file(&FileSpec::parse("PROFILE DATA A").unwrap(), "b")
            .unwrap();
        let pattern = FileSpec::parse("PROFILE * A").unwrap();
        let files = fs.listfile(&pattern).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn listfile_both_wildcards() {
        let (_dir, fs) = setup_fs();
        fs.write_file(&FileSpec::parse("FILE1 EXEC A").unwrap(), "a")
            .unwrap();
        fs.write_file(&FileSpec::parse("FILE2 DATA A").unwrap(), "b")
            .unwrap();
        let pattern = FileSpec::parse("* * A").unwrap();
        let files = fs.listfile(&pattern).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn listfile_empty_result() {
        let (_dir, fs) = setup_fs();
        let pattern = FileSpec::parse("NOFILE DATA A").unwrap();
        let files = fs.listfile(&pattern).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn erase_existing() {
        let (_dir, fs) = setup_fs();
        let spec = FileSpec::parse("MYFILE DATA A").unwrap();
        fs.write_file(&spec, "data").unwrap();
        fs.erase(&spec).unwrap();
        assert!(matches!(
            fs.read_file(&spec).unwrap_err(),
            CmsError::FileNotFound(_)
        ));
    }

    #[test]
    fn erase_nonexistent() {
        let (_dir, fs) = setup_fs();
        let spec = FileSpec::parse("NOFILE DATA A").unwrap();
        let err = fs.erase(&spec).unwrap_err();
        assert!(matches!(err, CmsError::FileNotFound(_)));
    }

    #[test]
    fn copyfile_same_disk() {
        let (_dir, fs) = setup_fs();
        let src = FileSpec::parse("FILE1 DATA A").unwrap();
        fs.write_file(&src, "content").unwrap();
        let dst = FileSpec::parse("FILE2 DATA A").unwrap();
        fs.copyfile(&src, &dst).unwrap();
        assert_eq!(fs.read_file(&dst).unwrap(), "content");
    }

    #[test]
    fn copyfile_cross_disk() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        fs.access_disk('B', dir.path().join("b"), AccessMode::ReadWrite)
            .unwrap();
        let src = FileSpec::parse("FILE1 DATA A").unwrap();
        fs.write_file(&src, "cross-disk").unwrap();
        let dst = FileSpec::parse("FILE1 DATA B").unwrap();
        fs.copyfile(&src, &dst).unwrap();
        assert_eq!(fs.read_file(&dst).unwrap(), "cross-disk");
    }

    #[test]
    fn copyfile_to_readonly_fails() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        fs.access_disk('B', dir.path().join("b"), AccessMode::ReadOnly)
            .unwrap();
        let src = FileSpec::parse("FILE1 DATA A").unwrap();
        fs.write_file(&src, "data").unwrap();
        let dst = FileSpec::parse("FILE1 DATA B").unwrap();
        let err = fs.copyfile(&src, &dst).unwrap_err();
        assert!(matches!(err, CmsError::ReadOnly('B')));
    }

    #[test]
    fn rename_same_disk() {
        let (_dir, fs) = setup_fs();
        let src = FileSpec::parse("OLD DATA A").unwrap();
        fs.write_file(&src, "renamed").unwrap();
        let dst = FileSpec::parse("NEW DATA A").unwrap();
        fs.rename(&src, &dst).unwrap();
        assert_eq!(fs.read_file(&dst).unwrap(), "renamed");
        assert!(matches!(
            fs.read_file(&src).unwrap_err(),
            CmsError::FileNotFound(_)
        ));
    }

    #[test]
    fn rename_cross_disk_fails() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        fs.access_disk('B', dir.path().join("b"), AccessMode::ReadWrite)
            .unwrap();
        let src = FileSpec::parse("FILE1 DATA A").unwrap();
        fs.write_file(&src, "data").unwrap();
        let dst = FileSpec::parse("FILE1 DATA B").unwrap();
        let err = fs.rename(&src, &dst).unwrap_err();
        assert!(matches!(err, CmsError::InvalidFileSpec(_)));
    }

    #[test]
    fn access_release_disk() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        assert!(fs.disk('A').is_some());
        fs.release_disk('A');
        assert!(fs.disk('A').is_none());
    }

    #[test]
    fn multi_disk_search_order() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        fs.access_disk('B', dir.path().join("b"), AccessMode::ReadWrite)
            .unwrap();

        // Write same filename to both disks with different content
        let spec_a = FileSpec::parse("FILE1 DATA A").unwrap();
        fs.write_file(&spec_a, "from A").unwrap();
        let spec_b = FileSpec::parse("FILE1 DATA B").unwrap();
        fs.write_file(&spec_b, "from B").unwrap();

        // Wildcard search should find A first
        let spec_star = FileSpec::parse("FILE1 DATA *").unwrap();
        let content = fs.read_file(&spec_star).unwrap();
        assert_eq!(content, "from A");
    }

    #[test]
    fn disk_not_accessed_error() {
        let fs = CmsFileSystem::new();
        let spec = FileSpec::parse("FILE DATA Z").unwrap();
        let err = fs.read_file(&spec).unwrap_err();
        assert!(matches!(err, CmsError::DiskNotAccessed('Z')));
    }

    #[test]
    fn with_default_disk() {
        let dir = TempDir::new().unwrap();
        let fs = CmsFileSystem::with_default_disk(dir.path()).unwrap();
        assert!(fs.disk('A').is_some());
        assert!(fs.disk('A').unwrap().is_writable());
    }

    #[test]
    fn rename_to_existing_fails() {
        let (_dir, fs) = setup_fs();
        let src = FileSpec::parse("FILE1 DATA A").unwrap();
        let dst = FileSpec::parse("FILE2 DATA A").unwrap();
        fs.write_file(&src, "one").unwrap();
        fs.write_file(&dst, "two").unwrap();
        let err = fs.rename(&src, &dst).unwrap_err();
        assert!(matches!(err, CmsError::FileExists(_)));
    }

    #[test]
    fn state_empty_file() {
        let (_dir, fs) = setup_fs();
        let spec = FileSpec::parse("EMPTY DATA A").unwrap();
        fs.write_file(&spec, "").unwrap();
        let info = fs.state(&spec).unwrap();
        assert_eq!(info.line_count, 0);
        assert_eq!(info.size_bytes, 0);
    }

    #[test]
    fn state_no_trailing_newline() {
        let (_dir, fs) = setup_fs();
        let spec = FileSpec::parse("NOTAIL DATA A").unwrap();
        fs.write_file(&spec, "line one\nline two").unwrap();
        let info = fs.state(&spec).unwrap();
        assert_eq!(info.line_count, 2);
    }

    #[test]
    fn access_disk_rejects_non_alpha() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        let err = fs
            .access_disk('1', dir.path().join("x"), AccessMode::ReadWrite)
            .unwrap_err();
        assert!(matches!(err, CmsError::InvalidFileSpec(_)));
    }

    #[test]
    fn listfile_wildcard_disk() {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        fs.access_disk('B', dir.path().join("b"), AccessMode::ReadWrite)
            .unwrap();
        fs.write_file(&FileSpec::parse("FILE1 EXEC A").unwrap(), "a")
            .unwrap();
        fs.write_file(&FileSpec::parse("FILE1 EXEC B").unwrap(), "b")
            .unwrap();
        let pattern = FileSpec::parse("FILE1 EXEC *").unwrap();
        let files = fs.listfile(&pattern).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_ignored() {
        let (_dir, fs) = setup_fs();
        // Write a real file
        let real = FileSpec::parse("REAL DATA A").unwrap();
        fs.write_file(&real, "real content").unwrap();

        // Create a symlink in the same disk directory
        let disk = fs.disk('A').unwrap();
        let link_path = disk.file_path("LINK", "DATA");
        let real_path = disk.file_path("REAL", "DATA");
        std::os::unix::fs::symlink(&real_path, &link_path).unwrap();

        // Symlink should not be visible via read_file
        let link_spec = FileSpec::parse("LINK DATA A").unwrap();
        assert!(fs.read_file(&link_spec).is_err());

        // Symlink should not appear in listfile
        let pattern = FileSpec::parse("* DATA A").unwrap();
        let files = fs.listfile(&pattern).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].spec.filename(), "REAL");
    }
}
