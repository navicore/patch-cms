//! # Connection Gating — Accept/refuse IUCV connections
//!
//! Demonstrates custom `on_connection_pending` logic:
//! 1. IPL a SECURE machine that only accepts connections from an allowlist
//! 2. IPL TRUSTED and UNTRUSTED machines
//! 3. TRUSTED connects successfully
//! 4. UNTRUSTED gets `ConnectionRefused`

use vm_iucv::collector::collector;
use vm_iucv::error::IucvError;
use vm_iucv::handler::{MachineContext, MachineHandler};
use vm_iucv::machine_id::MachineId;
use vm_iucv::message::SmsgMessage;
use vm_iucv::path::PathId;
use vm_iucv::supervisor::Supervisor;

/// A handler that only accepts connections from machines in the allowlist.
struct SecureHandler {
    allowed: Vec<String>,
}

impl MachineHandler for SecureHandler {
    fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}

    fn on_connection_pending(
        &mut self,
        _ctx: &MachineContext,
        _path: PathId,
        from: &MachineId,
    ) -> bool {
        let accepted = self.allowed.iter().any(|a| a == from.as_str());
        if accepted {
            println!("[SECURE] Accepting connection from {}", from);
        } else {
            println!("[SECURE] Refusing connection from {}", from);
        }
        accepted
    }
}

#[tokio::main]
async fn main() {
    let sup = Supervisor::new();

    let secure = MachineId::new("SECURE").unwrap();
    let trusted = MachineId::new("TRUSTED").unwrap();
    let untrust = MachineId::new("UNTRUST").unwrap();

    // IPL the secure machine with TRUSTED in its allowlist.
    let secure_handler = SecureHandler {
        allowed: vec!["TRUSTED".to_string()],
    };
    sup.ipl(&secure, secure_handler).await.unwrap();

    // IPL the other machines with collectors.
    let (trusted_handler, _trusted_handle) = collector();
    let (untrust_handler, _untrust_handle) = collector();
    sup.ipl(&trusted, trusted_handler).await.unwrap();
    sup.ipl(&untrust, untrust_handler).await.unwrap();

    // TRUSTED connects — should succeed.
    match sup.connect(&trusted, &secure).await {
        Ok(path) => println!("TRUSTED connected: {}", path),
        Err(e) => println!("TRUSTED failed: {}", e),
    }

    // UNTRUSTED connects — should be refused.
    match sup.connect(&untrust, &secure).await {
        Ok(path) => println!("UNTRUSTED connected: {}", path),
        Err(IucvError::ConnectionRefused(msg)) => {
            println!("UNTRUSTED refused (as expected): {}", msg)
        }
        Err(e) => println!("UNTRUSTED unexpected error: {}", e),
    }

    // Clean shutdown.
    sup.logoff(&trusted).await.unwrap();
    sup.logoff(&untrust).await.unwrap();
    sup.logoff(&secure).await.unwrap();
    sup.shutdown().await;

    println!("Done.");
}
