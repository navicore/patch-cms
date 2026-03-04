//! # Multi-Machine Pipeline — SMSG forwarding chain
//!
//! Demonstrates multi-machine coordination via SMSG:
//! 1. IPL 3 machines: PRODUCER → TRANSFORM → SINK
//! 2. PRODUCER sends data via SMSG to TRANSFORM
//! 3. TRANSFORM modifies the text (uppercases it) and forwards via SMSG to SINK
//! 4. SINK collects results
//! 5. Print collected results and shut down

use vm_iucv::collector::collector;
use vm_iucv::handler::{MachineContext, MachineHandler};
use vm_iucv::machine_id::MachineId;
use vm_iucv::message::SmsgMessage;
use vm_iucv::supervisor::Supervisor;

/// A handler that transforms each SMSG (uppercase) and forwards to a target.
struct TransformHandler {
    forward_to: MachineId,
}

impl MachineHandler for TransformHandler {
    fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
        let transformed = msg.text().to_ascii_uppercase();
        let _ = ctx.try_send_smsg(&self.forward_to, &transformed);
    }
}

#[tokio::main]
async fn main() {
    let sup = Supervisor::new();

    let producer = MachineId::new("PRODUCER").unwrap();
    let xform = MachineId::new("XFORM").unwrap();
    let sink = MachineId::new("SINK").unwrap();

    // IPL PRODUCER with a collector (it doesn't receive messages in this example).
    let (prod_handler, _prod_handle) = collector();
    sup.ipl(&producer, prod_handler).await.unwrap();

    // IPL TRANSFORM — forwards uppercase text to SINK.
    let xform_handler = TransformHandler {
        forward_to: sink.clone(),
    };
    sup.ipl(&xform, xform_handler).await.unwrap();

    // IPL SINK with a collector to capture results.
    let (sink_handler, sink_handle) = collector();
    sup.ipl(&sink, sink_handler).await.unwrap();

    // PRODUCER sends data through the pipeline.
    let inputs = ["hello world", "vm iucv", "pipeline test"];
    for text in &inputs {
        sup.smsg(&producer, &xform, text).await.unwrap();
    }

    // Wait for messages to propagate through the pipeline.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Print results collected at the SINK.
    let results = sink_handle.messages();
    println!("SINK received {} messages:", results.len());
    for msg in &results {
        println!("  from={} text={}", msg.from(), msg.text());
    }

    // Clean shutdown.
    sup.logoff(&producer).await.unwrap();
    sup.logoff(&xform).await.unwrap();
    sup.logoff(&sink).await.unwrap();
    sup.shutdown().await;

    println!("Done.");
}
