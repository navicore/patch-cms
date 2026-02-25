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
///
/// # Concurrency
///
/// This backend is **not safe for concurrent writers**. The ID counter
/// uses a read-modify-write sequence without file locking; concurrent
/// processes sharing the same spool directory can produce duplicate IDs
/// and data loss. Use a single `DirectoryBackend` instance per process.
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
            fs::create_dir_all(&dir)?;
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

    /// Allocate the next spool ID. **Not safe for concurrent writers** —
    /// two processes can both read the same ID and silently overwrite each
    /// other's `.data` file (data loss). Multi-process use requires
    /// `O_CREAT|O_EXCL` on the data file or an advisory lock (`flock`).
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
        let mut data_ids = std::collections::HashSet::new();

        if !dir.is_dir() {
            return Ok(entries);
        }

        // Single pass: collect valid .meta entries and track all .data IDs
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if let Some(stem) = path.file_stem() {
                    if let Some(id_str) = stem.to_str() {
                        if let Ok(file_id) = id_str.parse::<u64>() {
                            if ext == "data" {
                                data_ids.insert(file_id);
                            } else if ext == "meta" {
                                let meta_str = match fs::read_to_string(&path) {
                                    Ok(s) => s,
                                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                        continue;
                                    }
                                    Err(e) => return Err(SpoolError::Io(e)),
                                };
                                match SpoolFile::from_meta_string(&meta_str) {
                                    Some(sf) if sf.spool_id == file_id && sf.device == device => {
                                        entries.push((sf, path.clone()));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        // Filter entries to those with .data; auto-purge orphaned .meta
        let valid_ids: std::collections::HashSet<u64> = {
            let mut valid = std::collections::HashSet::new();
            entries.retain(|(sf, meta_path)| {
                if data_ids.contains(&sf.spool_id) {
                    valid.insert(sf.spool_id);
                    true
                } else {
                    let _ = fs::remove_file(meta_path);
                    false
                }
            });
            valid
        };

        // Clean up orphaned .data files (no corresponding valid .meta)
        for &data_id in &data_ids {
            if !valid_ids.contains(&data_id) {
                let _ = fs::remove_file(self.device_dir(device).join(format!("{}.data", data_id)));
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
        hold: bool,
        data: &str,
    ) -> Result<u64> {
        // Reject strings that would corrupt the key=value .meta format
        fn has_newline(s: &str) -> bool {
            s.contains('\n') || s.contains('\r')
        }
        if filename.is_empty()
            || filetype.is_empty()
            || origin_user.is_empty()
            || has_newline(filename)
            || has_newline(filetype)
            || has_newline(origin_user)
            || has_newline(dest_user)
        {
            return Err(SpoolError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "spool metadata fields must not be empty or contain newlines",
            )));
        }

        let id = self.allocate_id()?;

        let mut sf = SpoolFile::new(id, filename, filetype, origin_user, device);
        sf.dest_user = dest_user.to_ascii_uppercase();
        sf.class = class;
        sf.hold = hold;
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
                    // Remove .meta first — must succeed to prevent
                    // duplicate delivery on next dequeue call.
                    fs::remove_file(self.meta_path(device, id))?;
                    // Best-effort .data removal — content is already in
                    // memory, so never lose it over a .data delete error.
                    let _ = remove_file_ignore_not_found(&self.data_path(device, id));
                    return Ok((sf, data));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // .data removed between read_entries and here (TOCTOU) —
                    // purge the now-orphaned .meta and try next entry
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
        // Best-effort .data removal — entry is already invisible after
        // .meta deletion, so any .data error is non-fatal.
        let _ = remove_file_ignore_not_found(&data_path);
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
        // Validate parsed metadata matches caller-supplied device and ID
        if sf.spool_id != spool_id || sf.device != device {
            return Err(SpoolError::FileNotFound(spool_id));
        }
        let data = match fs::read_to_string(&data_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Orphaned .meta (no .data) — auto-purge so it doesn't
                // permanently block the queue on repeated receive() calls
                let _ = fs::remove_file(&meta_path);
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
        // Best-effort .data removal — content is already in memory,
        // so never lose it over a .data delete error.
        let _ = remove_file_ignore_not_found(&data_path);

        Ok((sf, data))
    }

    fn purge_all(&mut self, device: SpoolDevice, class: Option<SpoolClass>) -> Result<usize> {
        let entries = self.read_entries(device)?;
        let mut count = 0;
        let mut first_err: Option<std::io::Error> = None;

        for (sf, _) in &entries {
            let matches = match class {
                Some(c) => c.matches(&sf.class),
                None => true,
            };
            if matches {
                match fs::remove_file(self.meta_path(device, sf.spool_id)) {
                    Ok(()) => {
                        let _ = remove_file_ignore_not_found(&self.data_path(device, sf.spool_id));
                        count += 1;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Already gone — clean up .data but don't count
                        let _ = remove_file_ignore_not_found(&self.data_path(device, sf.spool_id));
                    }
                    Err(e) => {
                        if first_err.is_none() {
                            first_err = Some(e);
                        }
                    }
                }
            }
        }

        // Surface the first I/O error after purging as many as possible
        if let Some(e) = first_err {
            return Err(SpoolError::Io(e));
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
        let data = match fs::read_to_string(&data_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SpoolError::FileNotFound(spool_id));
            }
            Err(e) => return Err(SpoolError::Io(e)),
        };

        // Allocate new ID BEFORE deleting source — prevents data loss if
        // allocate_id fails (e.g. disk full writing .next_id)
        let new_id = self.allocate_id()?;

        // Write destination — .data first so an interrupted write leaves
        // an invisible orphan rather than a visible meta-only entry.
        sf.spool_id = new_id;
        sf.device = SpoolDevice::Reader;
        sf.dest_user = dest_user.to_ascii_uppercase();

        let dest_data = self.data_path(SpoolDevice::Reader, new_id);
        fs::write(&dest_data, &data)?;
        if let Err(e) = fs::write(
            self.meta_path(SpoolDevice::Reader, new_id),
            sf.to_meta_string(),
        ) {
            // Clean up orphaned .data — source is still intact
            let _ = fs::remove_file(&dest_data);
            return Err(SpoolError::Io(e));
        }

        // Remove source after destination is written. Propagate non-NotFound
        // errors on .meta removal to avoid silent duplicate entries.
        // Note: if this fails, the destination (new_id) exists in the reader
        // but the caller only knows the source spool_id. A QUERY READER
        // would reveal the duplicate for manual cleanup.
        match fs::remove_file(&meta_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(SpoolError::Io(e)),
        }
        let _ = remove_file_ignore_not_found(&data_path);

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
                false,
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
                false,
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
                false,
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
                false,
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
                false,
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
                false,
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
                false,
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
                false,
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
                false,
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
                false,
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
                false,
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
                false,
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
                false,
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
                    false,
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
                false,
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
    fn orphaned_meta_auto_purged_by_list_queue() {
        let (backend, _dir) = make_backend();
        // A .meta without .data is auto-purged during read_entries,
        // so list_queue never returns phantom entries.
        let meta_content =
            "SPOOL_ID=99\nFILENAME=ORPHAN\nFILETYPE=DATA\nORIGIN_USER=U\nDEVICE=READER\n";
        fs::write(backend.meta_path(SpoolDevice::Reader, 99), meta_content).unwrap();
        assert!(backend.meta_path(SpoolDevice::Reader, 99).exists());

        // list_queue auto-purges the orphan — returns empty
        let files = backend.list_queue(SpoolDevice::Reader, None).unwrap();
        assert!(files.is_empty());

        // .meta was cleaned up
        assert!(!backend.meta_path(SpoolDevice::Reader, 99).exists());
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
                false,
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
                false,
                "a-data\n",
            )
            .unwrap();

        // User A receives only their file
        let mut mgr = SpoolManager::new(backend, "USERA");
        let (sf, data) = mgr.receive().unwrap();
        assert_eq!(sf.filename, "FORA");
        assert_eq!(sf.dest_user, "USERA");
        assert_eq!(data, "a-data\n");

        // User A sees no more files (user-scoped query)
        let remaining = mgr.query_device(SpoolDevice::Reader, None).unwrap();
        assert!(remaining.is_empty());

        // User B's file is still in the backend
        let all = mgr.backend().list_queue(SpoolDevice::Reader, None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].filename, "FORB");
    }

    #[test]
    fn dequeue_auto_purges_orphaned_meta() {
        let (mut backend, _dir) = make_backend();
        // Create an orphaned .meta (no .data) with spool ID 1
        let meta_content =
            "SPOOL_ID=1\nFILENAME=ORPHAN\nFILETYPE=DATA\nORIGIN_USER=U\nDEVICE=READER\n";
        fs::write(backend.meta_path(SpoolDevice::Reader, 1), meta_content).unwrap();

        // Enqueue a valid file (gets ID from counter, not 1)
        backend
            .enqueue(
                SpoolDevice::Reader,
                "VALID",
                "FILE",
                "U",
                "",
                SpoolClass::default(),
                false,
                "valid data\n",
            )
            .unwrap();

        // dequeue skips the orphan and returns the valid file
        let (sf, data) = backend.dequeue(SpoolDevice::Reader).unwrap();
        assert_eq!(sf.filename, "VALID");
        assert_eq!(data, "valid data\n");

        // The orphaned .meta should have been auto-purged
        assert!(!backend.meta_path(SpoolDevice::Reader, 1).exists());
    }
}
