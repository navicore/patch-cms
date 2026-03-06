# Quickstart

## Add the dependency

```toml
[dependencies]
vm-iucv = { git = "https://github.com/navicore/patch-xedit" }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

## Your first program

Create a simple program that IPLs two machines and sends a message:

```rust
use vm_iucv::collector::collector;
use vm_iucv::handler::{MachineContext, MachineHandler};
use vm_iucv::machine_id::MachineId;
use vm_iucv::message::SmsgMessage;
use vm_iucv::supervisor::Supervisor;

struct PrintHandler;

impl MachineHandler for PrintHandler {
    fn on_smsg(&mut self, _ctx: &MachineContext, msg: SmsgMessage) {
        println!("Received: {}", msg.text());
    }
}

#[tokio::main]
async fn main() {
    let sup = Supervisor::new();

    let alice = MachineId::new("ALICE").unwrap();
    let bob = MachineId::new("BOB").unwrap();

    let (handler, _handle) = collector();
    sup.ipl(&alice, handler).await.unwrap();
    sup.ipl(&bob, PrintHandler).await.unwrap();

    sup.smsg(&alice, &bob, "Hello from ALICE!").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    sup.logoff(&alice).await.unwrap();
    sup.logoff(&bob).await.unwrap();
    sup.shutdown().await;
}
```

## Run the examples

The repository includes several runnable examples:

```sh
# Clone the repository
git clone https://github.com/navicore/patch-xedit.git
cd patch-xedit

# Run the simplest example
cargo run -p vm-iucv --example hello_smsg --features examples

# Run the echo server example
cargo run -p vm-iucv --example echo_server --features examples

# Run the IUCV chat example
cargo run -p vm-iucv --example iucv_chat --features examples
```

## Run the CMS machine

The CMS machine provides an interactive console with REXX scripting, spool
commands, and pipelines:

```sh
# Create a disk directory and launch
mkdir -p /tmp/cms/a
cargo run -p cms-machine -- --userid ALICE --disk /tmp/cms

# At the CMS prompt, try:
# GLOBALV SET COLOR blue
# GLOBALV GET COLOR
# SP PRT CLASS B
# PIPE literal hello | console
# LOGOFF
```

## Next steps

- Read the [Examples](EXAMPLES.md) page for annotated walkthroughs
- See [vm-iucv Overview](vm-iucv/overview.md) for the actor framework
- See [cms-core Overview](cms-core/overview.md) for the CMS command processor
- See [cms-machine Overview](cms-machine/overview.md) for the interactive console
- Check the [API Quick Reference](reference/api-quick-reference.md) for method tables
