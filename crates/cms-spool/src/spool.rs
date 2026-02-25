use crate::backend::SpoolBackend;
use crate::device::{DeviceConfig, SpoolClass, SpoolDevice};
use crate::error::Result;
use crate::spool_file::SpoolFile;

/// Result type returned from spool command execution.
#[derive(Debug)]
pub struct SpoolCommandResult {
    pub rc: i32,
    pub messages: Vec<String>,
}

impl SpoolCommandResult {
    pub fn ok() -> Self {
        Self {
            rc: 0,
            messages: Vec::new(),
        }
    }

    pub fn ok_with(messages: Vec<String>) -> Self {
        Self { rc: 0, messages }
    }

    pub fn error(rc: i32, msg: impl Into<String>) -> Self {
        Self {
            rc,
            messages: vec![msg.into()],
        }
    }
}

/// The spool manager orchestrates spool operations across devices.
///
/// Generic over the storage backend so that tests can use `InMemoryBackend`
/// while production uses `DirectoryBackend`.
pub struct SpoolManager<B: SpoolBackend> {
    backend: B,
    user_id: String,
    reader_config: DeviceConfig,
    printer_config: DeviceConfig,
    punch_config: DeviceConfig,
}

impl<B: SpoolBackend> SpoolManager<B> {
    /// Create a new spool manager with the given backend and user ID.
    pub fn new(backend: B, user_id: &str) -> Self {
        Self {
            backend,
            user_id: user_id.to_ascii_uppercase(),
            reader_config: DeviceConfig::new(SpoolDevice::Reader),
            printer_config: DeviceConfig::new(SpoolDevice::Printer),
            punch_config: DeviceConfig::new(SpoolDevice::Punch),
        }
    }

    /// Get the current user ID.
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// Get the device configuration for the given device.
    pub fn device_config(&self, device: SpoolDevice) -> &DeviceConfig {
        match device {
            SpoolDevice::Reader => &self.reader_config,
            SpoolDevice::Printer => &self.printer_config,
            SpoolDevice::Punch => &self.punch_config,
        }
    }

    /// Configure a device property.
    pub fn configure_device(
        &mut self,
        device: SpoolDevice,
        class: Option<SpoolClass>,
        dest: Option<&str>,
        hold: Option<bool>,
        continuous: Option<bool>,
        copies: Option<u32>,
    ) {
        let cfg = match device {
            SpoolDevice::Reader => &mut self.reader_config,
            SpoolDevice::Printer => &mut self.printer_config,
            SpoolDevice::Punch => &mut self.punch_config,
        };
        if let Some(c) = class {
            cfg.class = c;
        }
        if let Some(d) = dest {
            cfg.dest = d.to_ascii_uppercase();
        }
        if let Some(h) = hold {
            cfg.hold = h;
        }
        if let Some(cont) = continuous {
            cfg.continuous = cont;
        }
        if let Some(n) = copies {
            cfg.copies = n;
        }
    }

    /// Send a file to a user's reader (SENDFILE).
    ///
    /// The file is enqueued directly on the recipient's reader queue
    /// using the punch device configuration (class, hold). The file
    /// does not transit the punch queue itself.
    pub fn send_file(
        &mut self,
        filename: &str,
        filetype: &str,
        data: &str,
        dest_user: Option<&str>,
    ) -> Result<u64> {
        let dest = dest_user
            .map(|s| s.to_ascii_uppercase())
            .unwrap_or_else(|| self.user_id.clone());
        let class = self.punch_config.class;
        let hold = self.punch_config.hold;

        self.backend.enqueue(
            SpoolDevice::Reader,
            filename,
            filetype,
            &self.user_id,
            &dest,
            class,
            hold,
            data,
        )
    }

    /// Receive the next file from the reader (RECEIVE).
    ///
    /// Skips held files and dequeues the first non-held entry by ID,
    /// leaving all other entries (including held ones) in place.
    /// Returns `AllHeld` (RC=4) if only held files remain.
    /// Returns `QueueEmpty` (RC=2) if the queue is empty.
    pub fn receive(&mut self) -> Result<(SpoolFile, String)> {
        let files = self.backend.list_queue(SpoolDevice::Reader, None)?;
        if files.is_empty() {
            return Err(crate::error::SpoolError::QueueEmpty(SpoolDevice::Reader));
        }
        // Filter to files addressed to this user. Files with empty dest_user
        // are treated as addressed to their origin_user (not "any user").
        let user_id = &self.user_id;
        let my_files: Vec<_> = files
            .iter()
            .filter(|sf| {
                if sf.dest_user.is_empty() {
                    sf.origin_user == *user_id
                } else {
                    sf.dest_user == *user_id
                }
            })
            .collect();
        if my_files.is_empty() {
            return Err(crate::error::SpoolError::QueueEmpty(SpoolDevice::Reader));
        }
        // Find first non-held file among the user's files
        match my_files.iter().find(|sf| !sf.hold) {
            None => Err(crate::error::SpoolError::AllHeld),
            Some(sf) => self
                .backend
                .dequeue_by_id(SpoolDevice::Reader, sf.spool_id)
                .map_err(|e| match e {
                    // File removed between list_queue and dequeue_by_id
                    crate::error::SpoolError::FileNotFound(_) => {
                        crate::error::SpoolError::QueueEmpty(SpoolDevice::Reader)
                    }
                    other => other,
                }),
        }
    }

    /// Print a file (enqueue on printer).
    pub fn print_file(&mut self, filename: &str, filetype: &str, data: &str) -> Result<u64> {
        let class = self.printer_config.class;
        let dest = self.printer_config.dest.clone();
        let hold = self.printer_config.hold;
        self.backend.enqueue(
            SpoolDevice::Printer,
            filename,
            filetype,
            &self.user_id,
            &dest,
            class,
            hold,
            data,
        )
    }

    /// Punch a file (enqueue on punch).
    pub fn punch_file(&mut self, filename: &str, filetype: &str, data: &str) -> Result<u64> {
        let class = self.punch_config.class;
        let dest = self.punch_config.dest.clone();
        let hold = self.punch_config.hold;
        self.backend.enqueue(
            SpoolDevice::Punch,
            filename,
            filetype,
            &self.user_id,
            &dest,
            class,
            hold,
            data,
        )
    }

    /// Query files in a device queue.
    pub fn query_device(
        &self,
        device: SpoolDevice,
        class: Option<SpoolClass>,
    ) -> Result<Vec<SpoolFile>> {
        self.backend.list_queue(device, class)
    }

    /// Purge a specific file from a device queue.
    pub fn purge_file(&mut self, device: SpoolDevice, spool_id: u64) -> Result<()> {
        self.backend.purge(device, spool_id)
    }

    /// Purge all files from a device queue.
    pub fn purge_all(&mut self, device: SpoolDevice, class: Option<SpoolClass>) -> Result<usize> {
        self.backend.purge_all(device, class)
    }

    /// Get a reference to the backend.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Get a mutable reference to the backend.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::InMemoryBackend;

    fn make_manager() -> SpoolManager<InMemoryBackend> {
        SpoolManager::new(InMemoryBackend::new(), "TESTUSER")
    }

    #[test]
    fn new_manager_has_user_id() {
        let mgr = make_manager();
        assert_eq!(mgr.user_id(), "TESTUSER");
    }

    #[test]
    fn default_device_configs() {
        let mgr = make_manager();
        let cfg = mgr.device_config(SpoolDevice::Printer);
        assert_eq!(cfg.class, SpoolClass::default());
        assert!(!cfg.hold);
    }

    #[test]
    fn configure_device() {
        let mut mgr = make_manager();
        mgr.configure_device(
            SpoolDevice::Printer,
            Some(SpoolClass('B')),
            Some("OPERATOR"),
            Some(true),
            None,
            Some(3),
        );
        let cfg = mgr.device_config(SpoolDevice::Printer);
        assert_eq!(cfg.class, SpoolClass('B'));
        assert_eq!(cfg.dest, "OPERATOR");
        assert!(cfg.hold);
        assert_eq!(cfg.copies, 3);
    }

    #[test]
    fn send_and_receive() {
        let mut mgr = make_manager();
        let id = mgr
            .send_file("MYFILE", "DATA", "hello world\n", None)
            .unwrap();
        assert!(id > 0);

        let (sf, content) = mgr.receive().unwrap();
        assert_eq!(sf.filename, "MYFILE");
        assert_eq!(sf.filetype, "DATA");
        assert_eq!(content, "hello world\n");
    }

    #[test]
    fn receive_empty_reader() {
        let mut mgr = make_manager();
        let result = mgr.receive();
        assert!(result.is_err());
    }

    #[test]
    fn print_file() {
        let mut mgr = make_manager();
        let id = mgr
            .print_file("REPORT", "LISTING", "line1\nline2\n")
            .unwrap();
        assert!(id > 0);

        let files = mgr.query_device(SpoolDevice::Printer, None).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "REPORT");
    }

    #[test]
    fn punch_file() {
        let mut mgr = make_manager();
        let id = mgr.punch_file("DECK", "FORTRAN", "program\n").unwrap();
        assert!(id > 0);

        let files = mgr.query_device(SpoolDevice::Punch, None).unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn query_by_class() {
        let mut mgr = make_manager();
        mgr.configure_device(
            SpoolDevice::Printer,
            Some(SpoolClass('A')),
            None,
            None,
            None,
            None,
        );
        mgr.print_file("FILE1", "DATA", "d1").unwrap();

        mgr.configure_device(
            SpoolDevice::Printer,
            Some(SpoolClass('B')),
            None,
            None,
            None,
            None,
        );
        mgr.print_file("FILE2", "DATA", "d2").unwrap();

        let class_a = mgr
            .query_device(SpoolDevice::Printer, Some(SpoolClass('A')))
            .unwrap();
        assert_eq!(class_a.len(), 1);
        assert_eq!(class_a[0].filename, "FILE1");
    }

    #[test]
    fn purge_specific_file() {
        let mut mgr = make_manager();
        let id = mgr.print_file("TEMP", "DATA", "data").unwrap();
        mgr.purge_file(SpoolDevice::Printer, id).unwrap();

        let files = mgr.query_device(SpoolDevice::Printer, None).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn purge_all_files() {
        let mut mgr = make_manager();
        mgr.print_file("A", "B", "d").unwrap();
        mgr.print_file("C", "D", "d").unwrap();

        let count = mgr.purge_all(SpoolDevice::Printer, None).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn send_file_uses_punch_class() {
        let mut mgr = make_manager();
        mgr.configure_device(
            SpoolDevice::Punch,
            Some(SpoolClass('Z')),
            None,
            None,
            None,
            None,
        );
        mgr.send_file("FILE1", "DATA", "content\n", None).unwrap();

        let files = mgr.query_device(SpoolDevice::Reader, None).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].class, SpoolClass('Z'));
    }

    #[test]
    fn send_file_uses_punch_hold() {
        let mut mgr = make_manager();
        mgr.configure_device(SpoolDevice::Punch, None, None, Some(true), None, None);
        mgr.send_file("FILE1", "DATA", "content\n", None).unwrap();

        let files = mgr.query_device(SpoolDevice::Reader, None).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].hold);
    }

    #[test]
    fn spool_command_result_ok() {
        let r = SpoolCommandResult::ok();
        assert_eq!(r.rc, 0);
        assert!(r.messages.is_empty());
    }

    #[test]
    fn spool_command_result_error() {
        let r = SpoolCommandResult::error(24, "bad param");
        assert_eq!(r.rc, 24);
        assert_eq!(r.messages[0], "bad param");
    }

    // -- Phase 8e: edge case tests --

    #[test]
    fn send_file_to_self() {
        let mut mgr = make_manager();
        // Sending to self (no dest) should work — file goes to own reader
        let id = mgr.send_file("MYFILE", "DATA", "content\n", None).unwrap();
        assert!(id > 0);

        let files = mgr.query_device(SpoolDevice::Reader, None).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].origin_user, "TESTUSER");
    }

    #[test]
    fn send_file_with_dest() {
        let mut mgr = make_manager();
        let id = mgr
            .send_file("MYFILE", "DATA", "content\n", Some("JONES"))
            .unwrap();
        assert!(id > 0);

        // Verify dest_user is actually set on the queued file
        let files = mgr.query_device(SpoolDevice::Reader, None).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].dest_user, "JONES");
    }

    #[test]
    fn receive_when_empty_returns_error() {
        let mut mgr = make_manager();
        let result = mgr.receive();
        assert!(result.is_err());
        match result {
            Err(ref e) => {
                assert_eq!(e.rc(), 2);
                assert!(e.to_string().contains("No files"));
            }
            _ => panic!("Expected QueueEmpty error"),
        }
    }

    #[test]
    fn receive_all_held_returns_rc4() {
        use crate::backend::SpoolBackend;
        let mut backend = InMemoryBackend::new();
        backend
            .enqueue(
                SpoolDevice::Reader,
                "HELD",
                "FILE",
                "TESTUSER",
                "TESTUSER",
                SpoolClass::default(),
                false,
                "data\n",
            )
            .unwrap();
        if let Some(queue) = backend.queues_mut().get_mut(&SpoolDevice::Reader) {
            if let Some((sf, _)) = queue.front_mut() {
                sf.hold = true;
            }
        }
        let mut mgr = SpoolManager::new(backend, "TESTUSER");
        let result = mgr.receive();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.rc(), 4);
        assert!(err.to_string().contains("HOLD"));
    }

    #[test]
    fn receive_skips_held_to_find_unheld() {
        use crate::backend::SpoolBackend;
        let mut backend = InMemoryBackend::new();
        // Enqueue two files: first held, second not
        backend
            .enqueue(
                SpoolDevice::Reader,
                "HELD",
                "FILE",
                "TESTUSER",
                "TESTUSER",
                SpoolClass::default(),
                false,
                "held data\n",
            )
            .unwrap();
        backend
            .enqueue(
                SpoolDevice::Reader,
                "FREE",
                "FILE",
                "TESTUSER",
                "TESTUSER",
                SpoolClass::default(),
                false,
                "free data\n",
            )
            .unwrap();
        // Hold the first file
        if let Some(queue) = backend.queues_mut().get_mut(&SpoolDevice::Reader) {
            if let Some((sf, _)) = queue.front_mut() {
                sf.hold = true;
            }
        }
        let mut mgr = SpoolManager::new(backend, "TESTUSER");
        let (sf, data) = mgr.receive().unwrap();
        assert_eq!(sf.filename, "FREE");
        assert_eq!(data, "free data\n");
        // Held file should still be in queue with hold flag preserved
        let remaining = mgr.query_device(SpoolDevice::Reader, None).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].filename, "HELD");
        assert!(remaining[0].hold);
    }

    #[test]
    fn receive_other_user_files_returns_queue_empty() {
        use crate::backend::SpoolBackend;
        let mut backend = InMemoryBackend::new();
        // Enqueue a file addressed to a different user
        backend
            .enqueue(
                SpoolDevice::Reader,
                "FILE1",
                "DATA",
                "JONES",
                "JONES",
                SpoolClass::default(),
                false,
                "data\n",
            )
            .unwrap();
        let mut mgr = SpoolManager::new(backend, "SMITH");
        let result = mgr.receive();
        assert!(result.is_err());
        // Should be RC=2 (queue empty from this user's perspective), not RC=4
        assert_eq!(result.unwrap_err().rc(), 2);
    }

    #[test]
    fn purge_nonexistent_returns_error() {
        let mut mgr = make_manager();
        let result = mgr.purge_file(SpoolDevice::Reader, 99999);
        assert!(result.is_err());
        match result {
            Err(ref e) => {
                assert_eq!(e.rc(), 28);
                assert!(e.to_string().contains("99999"));
            }
            _ => panic!("Expected FileNotFound error"),
        }
    }

    #[test]
    fn purge_all_empty_queue() {
        let mut mgr = make_manager();
        let count = mgr.purge_all(SpoolDevice::Reader, None).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn purge_by_class_filter() {
        let mut mgr = make_manager();
        mgr.configure_device(
            SpoolDevice::Printer,
            Some(SpoolClass('A')),
            None,
            None,
            None,
            None,
        );
        mgr.print_file("FILE1", "DATA", "d1").unwrap();

        mgr.configure_device(
            SpoolDevice::Printer,
            Some(SpoolClass('B')),
            None,
            None,
            None,
            None,
        );
        mgr.print_file("FILE2", "DATA", "d2").unwrap();

        let count = mgr
            .purge_all(SpoolDevice::Printer, Some(SpoolClass('A')))
            .unwrap();
        assert_eq!(count, 1);

        let remaining = mgr.query_device(SpoolDevice::Printer, None).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].filename, "FILE2");
    }

    #[test]
    fn configure_device_preserves_unset_fields() {
        let mut mgr = make_manager();
        mgr.configure_device(
            SpoolDevice::Printer,
            Some(SpoolClass('B')),
            None,
            None,
            None,
            None,
        );
        mgr.configure_device(SpoolDevice::Printer, None, Some("OPER"), None, None, None);

        let cfg = mgr.device_config(SpoolDevice::Printer);
        assert_eq!(cfg.class, SpoolClass('B')); // preserved from first call
        assert_eq!(cfg.dest, "OPER"); // set by second call
    }

    #[test]
    fn send_multiple_receive_fifo_order() {
        let mut mgr = make_manager();
        mgr.send_file("FIRST", "DATA", "1\n", None).unwrap();
        mgr.send_file("SECOND", "DATA", "2\n", None).unwrap();
        mgr.send_file("THIRD", "DATA", "3\n", None).unwrap();

        let (sf1, _) = mgr.receive().unwrap();
        let (sf2, _) = mgr.receive().unwrap();
        let (sf3, _) = mgr.receive().unwrap();

        assert_eq!(sf1.filename, "FIRST");
        assert_eq!(sf2.filename, "SECOND");
        assert_eq!(sf3.filename, "THIRD");
    }

    #[test]
    fn query_wildcard_class() {
        let mut mgr = make_manager();
        mgr.configure_device(
            SpoolDevice::Printer,
            Some(SpoolClass('A')),
            None,
            None,
            None,
            None,
        );
        mgr.print_file("F1", "D", "d").unwrap();
        mgr.configure_device(
            SpoolDevice::Printer,
            Some(SpoolClass('Z')),
            None,
            None,
            None,
            None,
        );
        mgr.print_file("F2", "D", "d").unwrap();

        // Wildcard class matches all
        let all = mgr
            .query_device(SpoolDevice::Printer, Some(SpoolClass::ALL))
            .unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn user_id_uppercased() {
        let mgr = SpoolManager::new(InMemoryBackend::new(), "lowercase");
        assert_eq!(mgr.user_id(), "LOWERCASE");
    }
}
