use crate::machine_id::MachineId;

/// A fire-and-forget text message between machines (CP SMSG equivalent).
#[derive(Debug, Clone)]
pub struct SmsgMessage {
    pub from: MachineId,
    pub to: MachineId,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_access() {
        let msg = SmsgMessage {
            from: MachineId::new("ALICE").unwrap(),
            to: MachineId::new("BOB").unwrap(),
            text: "Hello".to_string(),
        };
        assert_eq!(msg.from.as_str(), "ALICE");
        assert_eq!(msg.to.as_str(), "BOB");
        assert_eq!(msg.text, "Hello");
    }

    #[test]
    fn clone() {
        let msg = SmsgMessage {
            from: MachineId::new("ALICE").unwrap(),
            to: MachineId::new("BOB").unwrap(),
            text: "Hello".to_string(),
        };
        let cloned = msg.clone();
        assert_eq!(cloned.from, msg.from);
        assert_eq!(cloned.to, msg.to);
        assert_eq!(cloned.text, msg.text);
    }
}
