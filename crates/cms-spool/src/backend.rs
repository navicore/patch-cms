use crate::device::{SpoolClass, SpoolDevice};
use crate::error::Result;
use crate::spool_file::SpoolFile;

/// Trait for spool storage backends.
///
/// Implementations handle the actual storage of spool data and metadata.
/// The trait is generic enough to support in-memory (for testing),
/// filesystem-backed, or network-backed implementations.
pub trait SpoolBackend {
    /// Enqueue a file into the given device queue.
    /// Returns the assigned spool ID.
    #[allow(clippy::too_many_arguments)]
    fn enqueue(
        &mut self,
        device: SpoolDevice,
        filename: &str,
        filetype: &str,
        origin_user: &str,
        dest_user: &str,
        class: SpoolClass,
        data: &str,
    ) -> Result<u64>;

    /// Dequeue the next file from the given device queue.
    /// Returns the spool file metadata and its content.
    fn dequeue(&mut self, device: SpoolDevice) -> Result<(SpoolFile, String)>;

    /// List files in a device queue, optionally filtered by class.
    fn list_queue(&self, device: SpoolDevice, class: Option<SpoolClass>) -> Result<Vec<SpoolFile>>;

    /// Remove a specific file from a device queue by spool ID.
    fn purge(&mut self, device: SpoolDevice, spool_id: u64) -> Result<()>;

    /// Remove all files from a device queue, optionally filtered by class.
    /// Returns the number of files purged.
    fn purge_all(&mut self, device: SpoolDevice, class: Option<SpoolClass>) -> Result<usize>;

    /// Re-enqueue a file at the front of a device queue, preserving FIFO position.
    /// Used to restore a file after a failed RECEIVE write.
    /// Returns the assigned spool ID.
    #[allow(clippy::too_many_arguments)]
    fn requeue_front(
        &mut self,
        device: SpoolDevice,
        filename: &str,
        filetype: &str,
        origin_user: &str,
        dest_user: &str,
        class: SpoolClass,
        data: &str,
    ) -> Result<u64>;

    /// Transfer a file from one device queue to another (e.g., printer → reader for SENDFILE).
    /// This is an internal operation: enqueue on target, dequeue from source.
    fn transfer_to_reader(
        &mut self,
        from_device: SpoolDevice,
        spool_id: u64,
        dest_user: &str,
    ) -> Result<()>;
}

/// In-memory spool backend for testing.
pub struct InMemoryBackend {
    next_id: u64,
    queues: std::collections::HashMap<SpoolDevice, std::collections::VecDeque<(SpoolFile, String)>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        use std::collections::VecDeque;
        let mut queues = std::collections::HashMap::new();
        queues.insert(SpoolDevice::Reader, VecDeque::new());
        queues.insert(SpoolDevice::Printer, VecDeque::new());
        queues.insert(SpoolDevice::Punch, VecDeque::new());
        Self { next_id: 1, queues }
    }
}

impl InMemoryBackend {
    /// Access the raw queues (e.g., to set hold flags directly).
    pub fn queues_mut(
        &mut self,
    ) -> &mut std::collections::HashMap<SpoolDevice, std::collections::VecDeque<(SpoolFile, String)>>
    {
        &mut self.queues
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SpoolBackend for InMemoryBackend {
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
        let id = self.next_id;
        self.next_id += 1;

        let mut sf = SpoolFile::new(id, filename, filetype, origin_user, device);
        sf.dest_user = dest_user.to_ascii_uppercase();
        sf.class = class;
        sf.records = data.lines().count();

        self.queues
            .entry(device)
            .or_default()
            .push_back((sf, data.to_string()));
        Ok(id)
    }

    fn dequeue(&mut self, device: SpoolDevice) -> Result<(SpoolFile, String)> {
        let queue = self.queues.entry(device).or_default();
        queue
            .pop_front()
            .ok_or(crate::error::SpoolError::ReaderEmpty)
    }

    fn list_queue(&self, device: SpoolDevice, class: Option<SpoolClass>) -> Result<Vec<SpoolFile>> {
        let empty = std::collections::VecDeque::new();
        let queue = self.queues.get(&device).unwrap_or(&empty);
        let files: Vec<SpoolFile> = queue
            .iter()
            .filter(|(sf, _)| match class {
                Some(c) => c.matches(&sf.class),
                None => true,
            })
            .map(|(sf, _)| sf.clone())
            .collect();
        Ok(files)
    }

    fn requeue_front(
        &mut self,
        device: SpoolDevice,
        filename: &str,
        filetype: &str,
        origin_user: &str,
        dest_user: &str,
        class: SpoolClass,
        data: &str,
    ) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let mut sf = SpoolFile::new(id, filename, filetype, origin_user, device);
        sf.dest_user = dest_user.to_ascii_uppercase();
        sf.class = class;
        sf.records = data.lines().count();

        self.queues
            .entry(device)
            .or_default()
            .push_front((sf, data.to_string()));
        Ok(id)
    }

    fn purge(&mut self, device: SpoolDevice, spool_id: u64) -> Result<()> {
        let queue = self.queues.entry(device).or_default();
        let pos = queue.iter().position(|(sf, _)| sf.spool_id == spool_id);
        match pos {
            Some(idx) => {
                let _ = queue.remove(idx);
                Ok(())
            }
            None => Err(crate::error::SpoolError::FileNotFound(spool_id)),
        }
    }

    fn purge_all(&mut self, device: SpoolDevice, class: Option<SpoolClass>) -> Result<usize> {
        let queue = self.queues.entry(device).or_default();
        let before = queue.len();
        match class {
            Some(c) => queue.retain(|(sf, _)| !c.matches(&sf.class)),
            None => queue.clear(),
        }
        Ok(before - queue.len())
    }

    fn transfer_to_reader(
        &mut self,
        from_device: SpoolDevice,
        spool_id: u64,
        dest_user: &str,
    ) -> Result<()> {
        let queue = self.queues.entry(from_device).or_default();
        let pos = queue.iter().position(|(sf, _)| sf.spool_id == spool_id);
        match pos {
            Some(idx) => {
                let (mut sf, data) = queue
                    .remove(idx)
                    .expect("index from position should be valid");
                // Allocate a fresh ID (consistent with DirectoryBackend)
                let new_id = self.next_id;
                self.next_id += 1;
                sf.spool_id = new_id;
                sf.device = SpoolDevice::Reader;
                sf.dest_user = dest_user.to_ascii_uppercase();
                self.queues
                    .entry(SpoolDevice::Reader)
                    .or_default()
                    .push_back((sf, data));
                Ok(())
            }
            None => Err(crate::error::SpoolError::FileNotFound(spool_id)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_enqueue_dequeue() {
        let mut backend = InMemoryBackend::new();
        let id = backend
            .enqueue(
                SpoolDevice::Reader,
                "TEST",
                "DATA",
                "USER1",
                "",
                SpoolClass::default(),
                "line1\nline2\n",
            )
            .unwrap();
        assert_eq!(id, 1);

        let (sf, data) = backend.dequeue(SpoolDevice::Reader).unwrap();
        assert_eq!(sf.spool_id, 1);
        assert_eq!(sf.filename, "TEST");
        assert_eq!(data, "line1\nline2\n");
    }

    #[test]
    fn in_memory_dequeue_empty() {
        let mut backend = InMemoryBackend::new();
        let result = backend.dequeue(SpoolDevice::Reader);
        assert!(result.is_err());
    }

    #[test]
    fn in_memory_list_queue() {
        let mut backend = InMemoryBackend::new();
        backend
            .enqueue(
                SpoolDevice::Reader,
                "FILE1",
                "DATA",
                "USER1",
                "",
                SpoolClass('A'),
                "data1",
            )
            .unwrap();
        backend
            .enqueue(
                SpoolDevice::Reader,
                "FILE2",
                "DATA",
                "USER1",
                "",
                SpoolClass('B'),
                "data2",
            )
            .unwrap();

        let all = backend.list_queue(SpoolDevice::Reader, None).unwrap();
        assert_eq!(all.len(), 2);

        let class_a = backend
            .list_queue(SpoolDevice::Reader, Some(SpoolClass('A')))
            .unwrap();
        assert_eq!(class_a.len(), 1);
        assert_eq!(class_a[0].filename, "FILE1");
    }

    #[test]
    fn in_memory_purge() {
        let mut backend = InMemoryBackend::new();
        let id = backend
            .enqueue(
                SpoolDevice::Reader,
                "FILE1",
                "DATA",
                "USER1",
                "",
                SpoolClass::default(),
                "data",
            )
            .unwrap();
        backend.purge(SpoolDevice::Reader, id).unwrap();

        let list = backend.list_queue(SpoolDevice::Reader, None).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn in_memory_purge_not_found() {
        let mut backend = InMemoryBackend::new();
        let result = backend.purge(SpoolDevice::Reader, 999);
        assert!(result.is_err());
    }

    #[test]
    fn in_memory_purge_all() {
        let mut backend = InMemoryBackend::new();
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
    fn in_memory_transfer_to_reader() {
        let mut backend = InMemoryBackend::new();
        let id = backend
            .enqueue(
                SpoolDevice::Printer,
                "FILE1",
                "DATA",
                "USER1",
                "",
                SpoolClass::default(),
                "content",
            )
            .unwrap();
        backend
            .transfer_to_reader(SpoolDevice::Printer, id, "USER2")
            .unwrap();

        let prt = backend.list_queue(SpoolDevice::Printer, None).unwrap();
        assert!(prt.is_empty());

        let rdr = backend.list_queue(SpoolDevice::Reader, None).unwrap();
        assert_eq!(rdr.len(), 1);
        assert_eq!(rdr[0].dest_user, "USER2");
    }

    #[test]
    fn in_memory_ids_increment() {
        let mut backend = InMemoryBackend::new();
        let id1 = backend
            .enqueue(
                SpoolDevice::Reader,
                "A",
                "B",
                "U",
                "",
                SpoolClass::default(),
                "",
            )
            .unwrap();
        let id2 = backend
            .enqueue(
                SpoolDevice::Reader,
                "C",
                "D",
                "U",
                "",
                SpoolClass::default(),
                "",
            )
            .unwrap();
        assert_eq!(id2, id1 + 1);
    }
}
