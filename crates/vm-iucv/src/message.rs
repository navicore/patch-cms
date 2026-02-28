use crate::error::{IucvError, Result};
use crate::machine_id::MachineId;

/// CP SMSG maximum text length (236 bytes).
const SMSG_MAX_TEXT_LEN: usize = 236;

/// A fire-and-forget text message between machines (CP SMSG equivalent).
///
/// Fields are private to enforce the 236-byte text length limit at
/// construction time. Use [`SmsgMessage::new`] to create instances.
#[derive(Debug, Clone)]
pub struct SmsgMessage {
    from: MachineId,
    to: MachineId,
    text: String,
}

impl SmsgMessage {
    /// Create a new SMSG message.
    ///
    /// Returns `InvalidParameter` if `text` exceeds 236 bytes (the CP SMSG
    /// limit).
    pub fn new(from: MachineId, to: MachineId, text: &str) -> Result<Self> {
        if text.len() > SMSG_MAX_TEXT_LEN {
            return Err(IucvError::InvalidParameter(format!(
                "SMSG text exceeds {} bytes",
                SMSG_MAX_TEXT_LEN
            )));
        }
        Ok(SmsgMessage {
            from,
            to,
            text: text.to_string(),
        })
    }

    /// The sender's machine id.
    pub fn from(&self) -> &MachineId {
        &self.from
    }

    /// The recipient's machine id.
    pub fn to(&self) -> &MachineId {
        &self.to
    }

    /// The message text.
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_access() {
        let msg = SmsgMessage::new(
            MachineId::new("ALICE").unwrap(),
            MachineId::new("BOB").unwrap(),
            "Hello",
        )
        .unwrap();
        assert_eq!(msg.from().as_str(), "ALICE");
        assert_eq!(msg.to().as_str(), "BOB");
        assert_eq!(msg.text(), "Hello");
    }

    #[test]
    fn clone() {
        let msg = SmsgMessage::new(
            MachineId::new("ALICE").unwrap(),
            MachineId::new("BOB").unwrap(),
            "Hello",
        )
        .unwrap();
        let cloned = msg.clone();
        assert_eq!(cloned.from(), msg.from());
        assert_eq!(cloned.to(), msg.to());
        assert_eq!(cloned.text(), msg.text());
    }

    #[test]
    fn max_length_text() {
        let text = "x".repeat(SMSG_MAX_TEXT_LEN);
        let msg = SmsgMessage::new(
            MachineId::new("ALICE").unwrap(),
            MachineId::new("BOB").unwrap(),
            &text,
        );
        assert!(msg.is_ok());
    }

    #[test]
    fn reject_too_long_text() {
        let text = "x".repeat(SMSG_MAX_TEXT_LEN + 1);
        let err = SmsgMessage::new(
            MachineId::new("ALICE").unwrap(),
            MachineId::new("BOB").unwrap(),
            &text,
        )
        .unwrap_err();
        assert_eq!(err.rc(), 24);
    }
}
