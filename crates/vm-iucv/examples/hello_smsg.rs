//! # Hello SMSG — Simplest possible vm-iucv example
//!
//! Demonstrates the basic machine lifecycle:
//! 1. Create a `Supervisor`
//! 2. IPL two machines (ALICE and BOB)
//! 3. ALICE sends an SMSG to BOB
//! 4. BOB's handler prints the message
//! 5. Logoff both machines and shut down

use vm_iucv::collector::collector;
use vm_iucv::handler::{MachineContext, MachineHandler};
use vm_iucv::machine_id::MachineId;
use vm_iucv::message::SmsgMessage;
use vm_iucv::supervisor::Supervisor;

/// A handler that prints every SMSG it receives.
struct PrintHandler;

impl MachineHandler for PrintHandler {
    fn on_smsg(&mut self, _ctx: &MachineContext, msg: SmsgMessage) {
        println!(
            "[{}] Received from {}: {}",
            msg.to(),
            msg.from(),
            msg.text()
        );
    }
}

#[tokio::main]
async fn main() {
    let sup = Supervisor::new();

    let alice = MachineId::new("ALICE").unwrap();
    let bob = MachineId::new("BOB").unwrap();

    // IPL both machines — ALICE uses a collector (we ignore its output),
    // BOB uses a handler that prints messages.
    let (alice_handler, _alice_handle) = collector();
    sup.ipl(&alice, alice_handler).await.unwrap();
    sup.ipl(&bob, PrintHandler).await.unwrap();

    // ALICE sends a message to BOB.
    sup.smsg(&alice, &bob, "Hello from ALICE!").await.unwrap();

    // Give the message time to be delivered and printed.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Clean shutdown.
    sup.logoff(&alice).await.unwrap();
    sup.logoff(&bob).await.unwrap();
    sup.shutdown().await;

    println!("Done.");
}
