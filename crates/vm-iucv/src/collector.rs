use crate::handler::{MachineContext, MachineHandler};
use crate::machine_id::MachineId;
use crate::message::SmsgMessage;
use crate::path::{IucvBuffer, PathId};
use std::sync::{Arc, Mutex};

/// Events observed on IUCV paths.
#[derive(Debug, Clone)]
pub enum PathEvent {
    Pending { path: PathId, from: MachineId },
    Complete { path: PathId, peer: MachineId },
    Severed { path: PathId, peer: MachineId },
    Data { path: PathId, data: IucvBuffer },
}

/// A test/inspection handler that collects all received messages.
///
/// Analogous to the Console stage in cms-pipelines — provides a split pair
/// where `CollectorHandler` is given to the supervisor and `CollectorHandle`
/// is kept by the test for inspection.
pub struct CollectorHandler {
    messages: Arc<Mutex<Vec<SmsgMessage>>>,
    path_events: Arc<Mutex<Vec<PathEvent>>>,
}

/// External handle for inspecting messages collected by a `CollectorHandler`.
#[derive(Clone)]
pub struct CollectorHandle {
    messages: Arc<Mutex<Vec<SmsgMessage>>>,
    path_events: Arc<Mutex<Vec<PathEvent>>>,
}

/// Create a paired collector handler and handle.
pub fn collector() -> (CollectorHandler, CollectorHandle) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let path_events = Arc::new(Mutex::new(Vec::new()));
    let handler = CollectorHandler {
        messages: Arc::clone(&messages),
        path_events: Arc::clone(&path_events),
    };
    let handle = CollectorHandle {
        messages,
        path_events,
    };
    (handler, handle)
}

impl MachineHandler for CollectorHandler {
    fn on_smsg(&mut self, _ctx: &MachineContext, msg: SmsgMessage) {
        self.messages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(msg);
    }

    fn on_connection_pending(
        &mut self,
        _ctx: &MachineContext,
        path: PathId,
        from: &MachineId,
    ) -> bool {
        self.path_events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(PathEvent::Pending {
                path,
                from: from.clone(),
            });
        true // accept by default
    }

    fn on_connection_complete(&mut self, _ctx: &MachineContext, path: PathId, peer: &MachineId) {
        self.path_events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(PathEvent::Complete {
                path,
                peer: peer.clone(),
            });
    }

    fn on_connection_severed(&mut self, _ctx: &MachineContext, path: PathId, peer: &MachineId) {
        self.path_events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(PathEvent::Severed {
                path,
                peer: peer.clone(),
            });
    }

    fn on_iucv_data(&mut self, _ctx: &MachineContext, path: PathId, data: IucvBuffer) {
        self.path_events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(PathEvent::Data { path, data });
    }
}

impl CollectorHandle {
    /// Return a snapshot of all collected messages.
    pub fn messages(&self) -> Vec<SmsgMessage> {
        self.messages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Return the number of collected messages.
    pub fn count(&self) -> usize {
        self.messages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    /// Return a snapshot of all collected path events.
    pub fn path_events(&self) -> Vec<PathEvent> {
        self.path_events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Return the number of collected path events.
    pub fn path_event_count(&self) -> usize {
        self.path_events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::PathCommand;

    fn test_ctx(name: &str) -> MachineContext {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let (pcmd_tx, _pcmd_rx) = tokio::sync::mpsc::channel::<PathCommand>(1);
        MachineContext::new(MachineId::new(name).unwrap(), tx, pcmd_tx)
    }

    #[test]
    fn initially_empty() {
        let (_handler, handle) = collector();
        assert_eq!(handle.count(), 0);
        assert!(handle.messages().is_empty());
        assert_eq!(handle.path_event_count(), 0);
    }

    #[test]
    fn receives_messages() {
        let (mut handler, handle) = collector();
        let ctx = test_ctx("TEST");
        let msg = SmsgMessage::new(
            MachineId::new("ALICE").unwrap(),
            MachineId::new("TEST").unwrap(),
            "Hello",
        )
        .unwrap();
        handler.on_smsg(&ctx, msg);
        assert_eq!(handle.count(), 1);
        assert_eq!(handle.messages()[0].text(), "Hello");
    }

    #[test]
    fn cloned_handle_shares_state() {
        let (mut handler, handle) = collector();
        let handle2 = handle.clone();
        let ctx = test_ctx("TEST");
        let msg = SmsgMessage::new(
            MachineId::new("ALICE").unwrap(),
            MachineId::new("TEST").unwrap(),
            "Shared",
        )
        .unwrap();
        handler.on_smsg(&ctx, msg);
        assert_eq!(handle.count(), 1);
        assert_eq!(handle2.count(), 1);
    }
}
