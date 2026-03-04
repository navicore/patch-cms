//! # Echo Server — SMSG request/reply pattern
//!
//! Demonstrates using `ctx.try_send_smsg()` from within a handler callback
//! to implement request/reply messaging:
//! 1. IPL an ECHO machine whose handler replies to every SMSG with the same text
//! 2. IPL a CLIENT machine that sends 3 messages and collects replies
//! 3. Uses `CollectorHandler` for the client side
//! 4. Prints all collected replies

use vm_iucv::collector::collector;
use vm_iucv::handler::{MachineContext, MachineHandler};
use vm_iucv::machine_id::MachineId;
use vm_iucv::message::SmsgMessage;
use vm_iucv::supervisor::Supervisor;

/// A handler that echoes every SMSG back to the sender.
struct EchoHandler;

impl MachineHandler for EchoHandler {
    fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
        let reply = format!("ECHO: {}", msg.text());
        let _ = ctx.try_send_smsg(msg.from(), &reply);
    }
}

#[tokio::main]
async fn main() {
    let sup = Supervisor::new();

    let echo_id = MachineId::new("ECHO").unwrap();
    let client_id = MachineId::new("CLIENT").unwrap();

    // IPL the echo server.
    sup.ipl(&echo_id, EchoHandler).await.unwrap();

    // IPL the client with a collector to capture replies.
    let (client_handler, client_handle) = collector();
    sup.ipl(&client_id, client_handler).await.unwrap();

    // Send 3 messages from CLIENT to ECHO.
    let messages = ["Hello", "World", "Goodbye"];
    for text in &messages {
        sup.smsg(&client_id, &echo_id, text).await.unwrap();
    }

    // Wait for replies to arrive.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Print collected replies.
    let replies = client_handle.messages();
    println!("Client received {} replies:", replies.len());
    for msg in &replies {
        println!("  from={} text={}", msg.from(), msg.text());
    }

    // Clean shutdown.
    sup.logoff(&client_id).await.unwrap();
    sup.logoff(&echo_id).await.unwrap();
    sup.shutdown().await;

    println!("Done.");
}
