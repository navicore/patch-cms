use std::fs;
use std::path::{Path, PathBuf};

use crate::backend::SpoolBackend;
use crate::device::{SpoolClass, SpoolDevice};
use crate::error::{Result, SpoolError};
use crate::spool_file::SpoolFile;

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
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            content.trim().parse().map_err(|_| {
                SpoolError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Corrupt .next_id file: {:?}", path),
                ))
            })
        } else {
            Ok(1)
        }
    }

    fn write_next_id(&self, id: u64) -> Result<()> {
        // Atomic write: write to temp file, then rename
        let path = self.next_id_path();
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, id.to_string())?;
        fs::rename(&tmp_path, &path)?;
        Ok(())
    }

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
                            if let Ok(_id) = id_str.parse::<u64>() {
                                let meta_str = fs::read_to_string(&path)?;
                                if let Some(sf) = SpoolFile::from_meta_string(&meta_str) {
                                    entries.push((sf, path.clone()));
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

        fs::write(self.data_path(device, id), data)?;
        fs::write(self.meta_path(device, id), sf.to_meta_string())?;

        Ok(id)
    }

    fn dequeue(&mut self, device: SpoolDevice) -> Result<(SpoolFile, String)> {
        let entries = self.read_entries(device)?;
        if entries.is_empty() {
            return Err(SpoolError::ReaderEmpty);
        }

        let (sf, _meta_path) = &entries[0];
        let id = sf.spool_id;
        let data = fs::read_to_string(self.data_path(device, id))?;
        let sf = sf.clone();

        // Remove .meta first so orphaned .data is harmless if interrupted
        fs::remove_file(self.meta_path(device, id))?;
        fs::remove_file(self.data_path(device, id))?;

        Ok((sf, data))
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

        if !meta_path.exists() {
            return Err(SpoolError::FileNotFound(spool_id));
        }

        // Remove .meta first so orphaned .data is harmless if interrupted
        fs::remove_file(meta_path)?;
        fs::remove_file(data_path)?;
        Ok(())
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
                // Remove .meta first so orphaned .data is harmless if interrupted
                fs::remove_file(self.meta_path(device, sf.spool_id))?;
                fs::remove_file(self.data_path(device, sf.spool_id))?;
                count += 1;
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
        if !meta_path.exists() {
            return Err(SpoolError::FileNotFound(spool_id));
        }

        // Read existing data and metadata
        let data = fs::read_to_string(self.data_path(from_device, spool_id))?;
        let meta_str = fs::read_to_string(&meta_path)?;
        let mut sf =
            SpoolFile::from_meta_string(&meta_str).ok_or(SpoolError::FileNotFound(spool_id))?;

        // Remove from source — .meta first so orphaned .data is harmless
        fs::remove_file(self.meta_path(from_device, spool_id))?;
        fs::remove_file(self.data_path(from_device, spool_id))?;

        // Allocate a fresh ID to avoid collision with existing reader files
        let new_id = self.allocate_id()?;

        // Update metadata and write to reader with new ID
        sf.spool_id = new_id;
        sf.device = SpoolDevice::Reader;
        sf.dest_user = dest_user.to_ascii_uppercase();

        fs::write(self.data_path(SpoolDevice::Reader, new_id), &data)?;
        fs::write(
            self.meta_path(SpoolDevice::Reader, new_id),
            sf.to_meta_string(),
        )?;

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
}
