use std::fmt;

/// Errors from the VM/CMS IUCV subsystem.
#[derive(Debug)]
pub enum IucvError {
    /// Invalid machine id format (RC=24)
    InvalidMachineId(String),
    /// Machine already running (RC=8)
    AlreadyRunning(String),
    /// Target machine not logged on (RC=12).
    /// Shares RC=12 with `AlreadyLoggedOff` — matches CP convention where
    /// "not found" and "already logged off" are the same return code.
    /// Callers can distinguish via variant matching or Display text.
    MachineNotFound(String),
    /// Machine already logged off (RC=12).
    /// Shares RC=12 with `MachineNotFound` — see above.
    AlreadyLoggedOff(String),
    /// Message delivery failed — channel closed (RC=16)
    DeliveryFailed(String),
    /// Router channel full — transient backpressure (RC=20)
    ChannelBusy(String),
    /// Machine task panicked (RC=28)
    MachinePanicked(String),
    /// Supervisor has shut down (RC=32)
    SupervisorDown,
}

impl fmt::Display for IucvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IucvError::InvalidMachineId(s) => {
                write!(f, "DMSIUC024E Invalid machine id - {}", s)
            }
            IucvError::AlreadyRunning(s) => {
                write!(f, "DMSIUC008E Machine already running - {}", s)
            }
            IucvError::MachineNotFound(s) => {
                write!(f, "DMSIUC012E Machine not found - {}", s)
            }
            IucvError::AlreadyLoggedOff(s) => {
                write!(f, "DMSIUC012E Machine already logged off - {}", s)
            }
            IucvError::DeliveryFailed(s) => {
                write!(f, "DMSIUC016E Delivery failed - {}", s)
            }
            IucvError::ChannelBusy(s) => {
                write!(f, "DMSIUC020W Channel busy - {}", s)
            }
            IucvError::MachinePanicked(s) => {
                write!(f, "DMSIUC028E Machine task panicked - {}", s)
            }
            IucvError::SupervisorDown => {
                write!(f, "DMSIUC032E Supervisor has shut down")
            }
        }
    }
}

impl std::error::Error for IucvError {}

impl IucvError {
    /// Return the CMS-style return code for this error.
    pub fn rc(&self) -> i32 {
        match self {
            IucvError::InvalidMachineId(_) => 24,
            IucvError::AlreadyRunning(_) => 8,
            IucvError::MachineNotFound(_) => 12,
            IucvError::AlreadyLoggedOff(_) => 12,
            IucvError::DeliveryFailed(_) => 16,
            IucvError::ChannelBusy(_) => 20,
            IucvError::MachinePanicked(_) => 28,
            IucvError::SupervisorDown => 32,
        }
    }
}

pub type Result<T> = std::result::Result<T, IucvError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_invalid_machine_id() {
        let e = IucvError::InvalidMachineId("TOOLONGID".to_string());
        assert!(e.to_string().contains("TOOLONGID"));
        assert_eq!(e.rc(), 24);
    }

    #[test]
    fn error_display_already_running() {
        let e = IucvError::AlreadyRunning("OPERATOR".to_string());
        assert!(e.to_string().contains("OPERATOR"));
        assert_eq!(e.rc(), 8);
    }

    #[test]
    fn error_display_machine_not_found() {
        let e = IucvError::MachineNotFound("GHOST".to_string());
        assert!(e.to_string().contains("GHOST"));
        assert_eq!(e.rc(), 12);
    }

    #[test]
    fn error_display_already_logged_off() {
        let e = IucvError::AlreadyLoggedOff("OLDUSER".to_string());
        assert!(e.to_string().contains("OLDUSER"));
        assert_eq!(e.rc(), 12);
    }

    #[test]
    fn error_display_delivery_failed() {
        let e = IucvError::DeliveryFailed("TARGET".to_string());
        assert!(e.to_string().contains("TARGET"));
        assert_eq!(e.rc(), 16);
    }

    #[test]
    fn error_display_channel_busy() {
        let e = IucvError::ChannelBusy("BUSY".to_string());
        assert!(e.to_string().contains("BUSY"));
        assert_eq!(e.rc(), 20);
    }

    #[test]
    fn error_display_machine_panicked() {
        let e = IucvError::MachinePanicked("CRASH".to_string());
        assert!(e.to_string().contains("CRASH"));
        assert_eq!(e.rc(), 28);
    }

    #[test]
    fn error_display_supervisor_down() {
        let e = IucvError::SupervisorDown;
        assert!(e.to_string().contains("shut down"));
        assert_eq!(e.rc(), 32);
    }

    #[test]
    fn error_messages_have_ibm_prefix() {
        let errors: Vec<IucvError> = vec![
            IucvError::InvalidMachineId("X".to_string()),
            IucvError::AlreadyRunning("X".to_string()),
            IucvError::MachineNotFound("X".to_string()),
            IucvError::AlreadyLoggedOff("X".to_string()),
            IucvError::DeliveryFailed("X".to_string()),
            IucvError::ChannelBusy("X".to_string()),
            IucvError::MachinePanicked("X".to_string()),
            IucvError::SupervisorDown,
        ];
        for e in &errors {
            let msg = e.to_string();
            assert!(
                msg.starts_with("DMSIUC"),
                "Error '{}' missing DMSIUC prefix",
                msg
            );
        }
    }
}
