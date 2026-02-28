use crate::handler::{MachineContext, MachineHandler};
use crate::message::SmsgMessage;
use std::sync::{Arc, Mutex};

/// A test/inspection handler that collects all received messages.
///
/// Analogous to the Console stage in cms-pipelines — provides a split pair
/// where `CollectorHandler` is given to the supervisor and `CollectorHandle`
/// is kept by the test for inspection.
pub struct CollectorHandler {
    messages: Arc<Mutex<Vec<SmsgMessage>>>,
}

/// External handle for inspecting messages collected by a `CollectorHandler`.
#[derive(Clone)]
pub struct CollectorHandle {
    messages: Arc<Mutex<Vec<SmsgMessage>>>,
}

/// Create a paired collector handler and handle.
pub fn collector() -> (CollectorHandler, CollectorHandle) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let handler = CollectorHandler {
        messages: Arc::clone(&messages),
    };
    let handle = CollectorHandle { messages };
    (handler, handle)
}

impl MachineHandler for CollectorHandler {
    fn on_smsg(&mut self, _ctx: &MachineContext, msg: SmsgMessage) {
        self.messages.lock().unwrap().push(msg);
    }
}

impl CollectorHandle {
    /// Return a snapshot of all collected messages.
    pub fn messages(&self) -> Vec<SmsgMessage> {
        self.messages.lock().unwrap().clone()
    }

    /// Return the number of collected messages.
    pub fn count(&self) -> usize {
        self.messages.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_id::MachineId;

    #[test]
    fn initially_empty() {
        let (_handler, handle) = collector();
        assert_eq!(handle.count(), 0);
        assert!(handle.messages().is_empty());
    }

    #[test]
    fn receives_messages() {
        let (mut handler, handle) = collector();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let ctx = MachineContext::new(MachineId::new("TEST").unwrap(), tx);
        let msg = SmsgMessage {
            from: MachineId::new("ALICE").unwrap(),
            to: MachineId::new("TEST").unwrap(),
            text: "Hello".to_string(),
        };
        handler.on_smsg(&ctx, msg);
        assert_eq!(handle.count(), 1);
        assert_eq!(handle.messages()[0].text, "Hello");
    }

    #[test]
    fn cloned_handle_shares_state() {
        let (mut handler, handle) = collector();
        let handle2 = handle.clone();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let ctx = MachineContext::new(MachineId::new("TEST").unwrap(), tx);
        let msg = SmsgMessage {
            from: MachineId::new("ALICE").unwrap(),
            to: MachineId::new("TEST").unwrap(),
            text: "Shared".to_string(),
        };
        handler.on_smsg(&ctx, msg);
        assert_eq!(handle.count(), 1);
        assert_eq!(handle2.count(), 1);
    }
}
