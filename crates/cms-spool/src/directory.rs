use std::fs;
use std::path::{Path, PathBuf};

use crate::backend::SpoolBackend;
use crate::device::{SpoolClass, SpoolDevice};
use crate::error::{Result, SpoolError};
use crate::spool_file::SpoolFile;

/// Remove a file, ignoring `NotFound` errors (the file may already be gone).
fn remove_file_ignore_not_found(path: &std::path::Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(SpoolError::Io(e)),
    }
}

/// Filesystem-backed spool backend.
///
/// Stores spool files under a base directory with subdirectories for each device:
/// ```text
/// <base>/
///   rdr/   # reader queue
///   prt/   # printer queue
///   pun/   # punch queue
/// ```
///
/// Each spool file is stored as two files:
/// - `<id>.data` — the file content
/// - `<id>.meta` — key=value metadata
///
/// The next spool ID counter is persisted in `.next_id`.
pub struct DirectoryBackend {
    base: PathBuf,
}

impl DirectoryBackend {
    /// Create a new directory backend, creating subdirectories if needed.
    pub fn new(base: &Path) -> Result<Self> {
        for device in &[
            SpoolDevice::Reader,
            SpoolDevice::Printer,
            SpoolDevice::Punch,
        ] {
            let dir = base.join(device.dir_name());
            if !dir.is_dir() {
                fs::create_dir_all(&dir)?;
            }
        }
        Ok(Self {
            base: base.to_path_buf(),
        })
    }

    /// Path to the base directory.
    pub fn base_path(&self) -> &Path {
        &self.base
    }

    fn device_dir(&self, device: SpoolDevice) -> PathBuf {
        self.base.join(device.dir_name())
    }

    fn next_id_path(&self) -> PathBuf {
        self.base.join(".next_id")
    }

    fn read_next_id(&self) -> Result<u64> {
        let path = self.next_id_path();
        match fs::read_to_string(&path) {
            Ok(content) => content.trim().parse().map_err(|_| {
                SpoolError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Corrupt .next_id file: {:?}", path),
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(1),
            Err(e) => Err(SpoolError::Io(e)),
        }
    }

    fn write_next_id(&self, id: u64) -> Result<()> {
        // Atomic write: write to temp file, then rename
        let path = self.next_id_path();
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, id.to_string())?;
        if let Err(e) = fs::rename(&tmp_path, &path) {
            let _ = fs::remove_file(&tmp_path); // best-effort cleanup
            return Err(SpoolError::Io(e));
        }
        Ok(())
    }

    /// Allocate the next spool ID. Not safe for concurrent writers —
    /// a file lock (e.g. flock) would be needed for multi-process use.
    fn allocate_id(&mut self) -> Result<u64> {
        let id = self.read_next_id()?;
        self.write_next_id(id + 1)?;
        Ok(id)
    }

    fn data_path(&self, device: SpoolDevice, id: u64) -> PathBuf {
        self.device_dir(device).join(format!("{}.data", id))
    }

    fn meta_path(&self, device: SpoolDevice, id: u64) -> PathBuf {
        self.device_dir(device).join(format!("{}.meta", id))
    }

    /// Read all spool entries in a device directory, sorted by spool ID.
    fn read_entries(&self, device: SpoolDevice) -> Result<Vec<(SpoolFile, PathBuf)>> {
        let dir = self.device_dir(device);
        let mut entries = Vec::new();

        if !dir.is_dir() {
            return Ok(entries);
        }

        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "meta" {
                    if let Some(stem) = path.file_stem() {
                        if let Some(id_str) = stem.to_str() {
                            if let Ok(file_id) = id_str.parse::<u64>() {
                                let meta_str = match fs::read_to_string(&path) {
                                    Ok(s) => s,
                                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                        continue; // removed between readdir and read
                                    }
                                    Err(e) => return Err(SpoolError::Io(e)),
                                };
                                match SpoolFile::from_meta_string(&meta_str) {
                                    Some(sf) if sf.spool_id == file_id => {
                                        entries.push((sf, path.clone()));
                                    }
                                    _ => {
                                        // Skip corrupt or mismatched metadata.
                                        // The entry is invisible but we don't
                                        // brick the entire queue.
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        entries.sort_by_key(|(sf, _)| sf.spool_id);
        Ok(entries)
    }
}

impl SpoolBackend for DirectoryBackend {
    fn enqueue(
        &mut self,
        device: SpoolDevice,
        filename: &str,
        filetype: &str,
        origin_user: &str,
        dest_user: &str,
        class: SpoolClass,
        data: &str,
    ) -> Result<u64> {
        let id = self.allocate_id()?;

        let mut sf = SpoolFile::new(id, filename, filetype, origin_user, device);
        sf.dest_user = dest_user.to_ascii_uppercase();
        sf.class = class;
        sf.records = data.lines().count();

        // Write .data first, then .meta. The create order is opposite to the
        // delete order (meta-first-on-delete) so that an interrupted write
        // leaves an invisible orphaned .data rather than a visible .meta
        // with no .data (which would block the queue).
        fs::write(self.data_path(device, id), data)?;
        fs::write(self.meta_path(device, id), sf.to_meta_string())?;

        Ok(id)
    }

    fn dequeue(&mut self, device: SpoolDevice) -> Result<(SpoolFile, String)> {
        let entries = self.read_entries(device)?;
        if entries.is_empty() {
            return Err(SpoolError::QueueEmpty(device));
        }

        // Skip entries whose .data is missing (orphaned .meta from corruption);
        // auto-purge the orphan so the queue recovers.
        for (sf, _) in &entries {
            let id = sf.spool_id;
            match fs::read_to_string(self.data_path(device, id)) {
                Ok(data) => {
                    let sf = sf.clone();
                    // Remove .meta first so orphaned .data is harmless.
                    // Best-effort .data removal — never lose the content
                    // that is already in memory.
                    let _ = fs::remove_file(self.meta_path(device, id));
                    let _ = remove_file_ignore_not_found(&self.data_path(device, id));
                    return Ok((sf, data));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Orphaned .meta — purge it and try next entry
                    let _ = fs::remove_file(self.meta_path(device, id));
                }
                Err(e) => return Err(SpoolError::Io(e)),
            }
        }

        Err(SpoolError::QueueEmpty(device))
    }

    fn list_queue(&self, device: SpoolDevice, class: Option<SpoolClass>) -> Result<Vec<SpoolFile>> {
        let entries = self.read_entries(device)?;
        let files: Vec<SpoolFile> = entries
            .into_iter()
            .filter(|(sf, _)| match class {
                Some(c) => c.matches(&sf.class),
                None => true,
            })
            .map(|(sf, _)| sf)
            .collect();
        Ok(files)
    }

    fn purge(&mut self, device: SpoolDevice, spool_id: u64) -> Result<()> {
        let data_path = self.data_path(device, spool_id);
        let meta_path = self.meta_path(device, spool_id);

        // Remove .meta first — map NotFound to FileNotFound(spool_id)
        match fs::remove_file(&meta_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SpoolError::FileNotFound(spool_id));
            }
            Err(e) => return Err(SpoolError::Io(e)),
        }
        // Ignore NotFound on .data (may be absent from interrupted enqueue)
        remove_file_ignore_not_found(&data_path)?;
        Ok(())
    }

    fn dequeue_by_id(&mut self, device: SpoolDevice, spool_id: u64) -> Result<(SpoolFile, String)> {
        let meta_path = self.meta_path(device, spool_id);
        let data_path = self.data_path(device, spool_id);

        // Read metadata — map NotFound to FileNotFound(spool_id)
        let meta_str = match fs::read_to_string(&meta_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SpoolError::FileNotFound(spool_id));
            }
            Err(e) => return Err(SpoolError::Io(e)),
        };
        let sf =
            SpoolFile::from_meta_string(&meta_str).ok_or(SpoolError::FileNotFound(spool_id))?;
        let data = match fs::read_to_string(&data_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SpoolError::FileNotFound(spool_id));
            }
            Err(e) => return Err(SpoolError::Io(e)),
        };

        // Remove .meta first so orphaned .data is harmless if interrupted
        match fs::remove_file(&meta_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(SpoolError::Io(e)),
        }
        remove_file_ignore_not_found(&data_path)?;

        Ok((sf, data))
    }

    fn purge_all(&mut self, device: SpoolDevice, class: Option<SpoolClass>) -> Result<usize> {
        let entries = self.read_entries(device)?;
        let mut count = 0;

        for (sf, _) in &entries {
            let matches = match class {
                Some(c) => c.matches(&sf.class),
                None => true,
            };
            if matches {
                // Best-effort removal: skip individual I/O failures so a
                // single problematic entry doesn't prevent purging the rest.
                match fs::remove_file(self.meta_path(device, sf.spool_id)) {
                    Ok(()) => {
                        let _ = remove_file_ignore_not_found(&self.data_path(device, sf.spool_id));
                        count += 1;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Already gone — still counts as purged
                        let _ = remove_file_ignore_not_found(&self.data_path(device, sf.spool_id));
                        count += 1;
                    }
                    Err(_) => {
                        // Skip this entry; continue with the rest
                    }
                }
            }
        }

        Ok(count)
    }

    fn transfer_to_reader(
        &mut self,
        from_device: SpoolDevice,
        spool_id: u64,
        dest_user: &str,
    ) -> Result<()> {
        let meta_path = self.meta_path(from_device, spool_id);
        let data_path = self.data_path(from_device, spool_id);

        // Read existing data and metadata — map NotFound to FileNotFound
        let meta_str = match fs::read_to_string(&meta_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SpoolError::FileNotFound(spool_id));
            }
            Err(e) => return Err(SpoolError::Io(e)),
        };
        let mut sf =
            SpoolFile::from_meta_string(&meta_str).ok_or(SpoolError::FileNotFound(spool_id))?;
        let data = fs::read_to_string(&data_path)?;

        // Allocate new ID BEFORE deleting source — prevents data loss if
        // allocate_id fails (e.g. disk full writing .next_id)
        let new_id = self.allocate_id()?;

        // Write destination — .data first so an interrupted write leaves
        // an invisible orphan rather than a visible meta-only entry.
        sf.spool_id = new_id;
        sf.device = SpoolDevice::Reader;
        sf.dest_user = dest_user.to_ascii_uppercase();

        fs::write(self.data_path(SpoolDevice::Reader, new_id), &data)?;
        fs::write(
            self.meta_path(SpoolDevice::Reader, new_id),
            sf.to_meta_string(),
        )?;

        // Remove source only after destination is fully written
        match fs::remove_file(&meta_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(SpoolError::Io(e)),
        }
        remove_file_ignore_not_found(&data_path)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_backend() -> (DirectoryBackend, TempDir) {
        let dir = TempDir::new().unwrap();
        let spool_dir = dir.path().join("spool");
        fs::create_dir(&spool_dir).unwrap();
        let backend = DirectoryBackend::new(&spool_dir).unwrap();
        (backend, dir)
    }

    #[test]
    fn creates_subdirectories() {
        let (backend, _dir) = make_backend();
        assert!(backend.device_dir(SpoolDevice::Reader).is_dir());
        assert!(backend.device_dir(SpoolDevice::Printer).is_dir());
        assert!(backend.device_dir(SpoolDevice::Punch).is_dir());
    }

    #[test]
    fn enqueue_creates_files() {
        let (mut backend, _dir) = make_backend();
        let id = backend
            .enqueue(
                SpoolDevice::Reader,
                "TEST",
                "DATA",
                "USER1",
                "",
                SpoolClass::default(),
                "hello\n",
            )
            .unwrap();

        assert!(backend.data_path(SpoolDevice::Reader, id).exists());
        assert!(backend.meta_path(SpoolDevice::Reader, id).exists());
    }

    #[test]
    fn enqueue_dequeue_roundtrip() {
        let (mut backend, _dir) = make_backend();
        backend
            .enqueue(
                SpoolDevice::Reader,
                "MYFILE",
                "EXEC",
                "USER1",
                "",
                SpoolClass('B'),
                "line1\nline2\n",
            )
            .unwrap();

        let (sf, data) = backend.dequeue(SpoolDevice::Reader).unwrap();
        assert_eq!(sf.filename, "MYFILE");
        assert_eq!(sf.filetype, "EXEC");
        assert_eq!(sf.class, SpoolClass('B'));
        assert_eq!(data, "line1\nline2\n");
    }

    #[test]
    fn dequeue_empty() {
        let (mut backend, _dir) = make_backend();
        let result = backend.dequeue(SpoolDevice::Reader);
        assert!(result.is_err());
    }

    #[test]
    fn dequeue_removes_files() {
        let (mut backend, _dir) = make_backend();
        let id = backend
            .enqueue(
                SpoolDevice::Reader,
                "A",
                "B",
                "U",
                "",
                SpoolClass::default(),
                "d",
            )
            .unwrap();
        backend.dequeue(SpoolDevice::Reader).unwrap();

        assert!(!backend.data_path(SpoolDevice::Reader, id).exists());
        assert!(!backend.meta_path(SpoolDevice::Reader, id).exists());
    }

    #[test]
    fn list_queue_returns_sorted() {
        let (mut backend, _dir) = make_backend();
        let id1 = backend
            .enqueue(
                SpoolDevice::Printer,
                "FIRST",
                "DATA",
                "U",
                "",
                SpoolClass::default(),
                "d1",
            )
            .unwrap();
        let id2 = backend
            .enqueue(
                SpoolDevice::Printer,
                "SECOND",
                "DATA",
                "U",
                "",
                SpoolClass::default(),
                "d2",
            )
            .unwrap();

        let files = backend.list_queue(SpoolDevice::Printer, None).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].spool_id, id1);
        assert_eq!(files[1].spool_id, id2);
    }

    #[test]
    fn list_queue_filters_by_class() {
        let (mut backend, _dir) = make_backend();
        backend
            .enqueue(
                SpoolDevice::Printer,
                "A",
                "B",
                "U",
                "",
                SpoolClass('A'),
                "d",
            )
            .unwrap();
        backend
            .enqueue(
                SpoolDevice::Printer,
                "C",
                "D",
                "U",
                "",
                SpoolClass('B'),
                "d",
            )
            .unwrap();

        let class_a = backend
            .list_queue(SpoolDevice::Printer, Some(SpoolClass('A')))
            .unwrap();
        assert_eq!(class_a.len(), 1);
        assert_eq!(class_a[0].filename, "A");
    }

    #[test]
    fn purge_specific() {
        let (mut backend, _dir) = make_backend();
        let id = backend
            .enqueue(
                SpoolDevice::Reader,
                "A",
                "B",
                "U",
                "",
                SpoolClass::default(),
                "d",
            )
            .unwrap();
        backend.purge(SpoolDevice::Reader, id).unwrap();

        let files = backend.list_queue(SpoolDevice::Reader, None).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn purge_not_found() {
        let (mut backend, _dir) = make_backend();
        let result = backend.purge(SpoolDevice::Reader, 999);
        assert!(result.is_err());
    }

    #[test]
    fn purge_all_returns_count() {
        let (mut backend, _dir) = make_backend();
        backend
            .enqueue(
                SpoolDevice::Printer,
                "A",
                "B",
                "U",
                "",
                SpoolClass::default(),
                "d",
            )
            .unwrap();
        backend
            .enqueue(
                SpoolDevice::Printer,
                "C",
                "D",
                "U",
                "",
                SpoolClass::default(),
                "d",
            )
            .unwrap();

        let count = backend.purge_all(SpoolDevice::Printer, None).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn purge_all_by_class() {
        let (mut backend, _dir) = make_backend();
        backend
            .enqueue(
                SpoolDevice::Printer,
                "A",
                "B",
                "U",
                "",
                SpoolClass('A'),
                "d",
            )
            .unwrap();
        backend
            .enqueue(
                SpoolDevice::Printer,
                "C",
                "D",
                "U",
                "",
                SpoolClass('B'),
                "d",
            )
            .unwrap();

        let count = backend
            .purge_all(SpoolDevice::Printer, Some(SpoolClass('A')))
            .unwrap();
        assert_eq!(count, 1);

        let remaining = backend.list_queue(SpoolDevice::Printer, None).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].filename, "C");
    }

    #[test]
    fn transfer_to_reader() {
        let (mut backend, _dir) = make_backend();
        let id = backend
            .enqueue(
                SpoolDevice::Printer,
                "FILE1",
                "DATA",
                "USER1",
                "",
                SpoolClass::default(),
                "content here\n",
            )
            .unwrap();

        backend
            .transfer_to_reader(SpoolDevice::Printer, id, "USER2")
            .unwrap();

        // Gone from printer
        let prt = backend.list_queue(SpoolDevice::Printer, None).unwrap();
        assert!(prt.is_empty());

        // In reader with updated dest
        let rdr = backend.list_queue(SpoolDevice::Reader, None).unwrap();
        assert_eq!(rdr.len(), 1);
        assert_eq!(rdr[0].dest_user, "USER2");
        assert_eq!(rdr[0].device, SpoolDevice::Reader);
    }

    #[test]
    fn id_counter_persists() {
        let dir = TempDir::new().unwrap();
        let spool_dir = dir.path().join("spool");
        fs::create_dir(&spool_dir).unwrap();

        let id1 = {
            let mut backend = DirectoryBackend::new(&spool_dir).unwrap();
            backend
                .enqueue(
                    SpoolDevice::Reader,
                    "A",
                    "B",
                    "U",
                    "",
                    SpoolClass::default(),
                    "d",
                )
                .unwrap()
        };

        // Create a new backend instance — ID should continue
        let mut backend = DirectoryBackend::new(&spool_dir).unwrap();
        let id2 = backend
            .enqueue(
                SpoolDevice::Reader,
                "C",
                "D",
                "U",
                "",
                SpoolClass::default(),
                "d",
            )
            .unwrap();

        assert_eq!(id2, id1 + 1);
    }

    #[test]
    fn interrupted_enqueue_data_only_is_invisible() {
        let (backend, _dir) = make_backend();
        // With data-first write order, an interrupted enqueue leaves .data
        // but no .meta — the entry is invisible to the queue.
        fs::write(backend.data_path(SpoolDevice::Reader, 99), "orphan data").unwrap();

        let files = backend.list_queue(SpoolDevice::Reader, None).unwrap();
        assert!(files.is_empty()); // invisible — correct behavior
    }

    #[test]
    fn legacy_meta_only_entry_can_be_purged() {
        let (backend, _dir) = make_backend();
        // A .meta without .data (from old code or corruption) is visible
        // but dequeue fails. purge can clean it up.
        let meta_content =
            "SPOOL_ID=99\nFILENAME=ORPHAN\nFILETYPE=DATA\nORIGIN_USER=U\nDEVICE=READER\n";
        fs::write(backend.meta_path(SpoolDevice::Reader, 99), meta_content).unwrap();

        let files = backend.list_queue(SpoolDevice::Reader, None).unwrap();
        assert_eq!(files.len(), 1);

        let mut backend = backend;
        let result = backend.dequeue_by_id(SpoolDevice::Reader, 99);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().rc(), 28); // FileNotFound, not Io

        backend.purge(SpoolDevice::Reader, 99).unwrap();
        let files = backend.list_queue(SpoolDevice::Reader, None).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn receive_multi_user_directory_backend() {
        use crate::spool::SpoolManager;

        let (mut backend, _dir) = make_backend();
        // Enqueue a file for user B, then one for user A
        backend
            .enqueue(
                SpoolDevice::Reader,
                "FORB",
                "DATA",
                "SENDER",
                "USERB",
                SpoolClass::default(),
                "b-data\n",
            )
            .unwrap();
        backend
            .enqueue(
                SpoolDevice::Reader,
                "FORA",
                "DATA",
                "SENDER",
                "USERA",
                SpoolClass::default(),
                "a-data\n",
            )
            .unwrap();

        // User A receives only their file
        let mut mgr = SpoolManager::new(backend, "USERA");
        let (sf, data) = mgr.receive().unwrap();
        assert_eq!(sf.filename, "FORA");
        assert_eq!(sf.dest_user, "USERA");
        assert_eq!(data, "a-data\n");

        // User B's file is still in the queue
        let remaining = mgr.query_device(SpoolDevice::Reader, None).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].filename, "FORB");
        assert_eq!(remaining[0].dest_user, "USERB");
    }
}
