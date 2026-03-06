# Quickstart

## Launch a CMS Machine

```sh
# Clone and build
git clone https://github.com/navicore/patch-cms.git
cd patch-cms
cargo build -p cms-machine --release

# Create a disk directory and launch
mkdir -p /tmp/cms/a
cargo run -p cms-machine -- --userid ALICE --disk /tmp/cms
```

You get an interactive CMS prompt with REXX scripting, spool commands,
pipelines, and inter-machine messaging.

## Your First REXX Program

Create a file `/tmp/cms/a/GREET.exec`:

```rexx
/* REXX — send a greeting to another machine */
parse arg userid .
if userid = '' then do
    say 'Usage: GREET userid'
    exit 24
end
'SMSG' userid 'Hello from CMS!'
if rc = 0 then say 'Sent.'
else say 'Failed, RC='rc
```

At the CMS prompt:

```
GREET BOB
```

Any file named `*.exec` on your A-disk is callable as a command.

## Try the Built-in Commands

```
GLOBALV SET COLOR blue        Set a persistent variable
GLOBALV GET COLOR             Retrieve it
PIPE literal hello | console  Run a pipeline
SP PRT CLASS B                Configure the printer spool
QUERY PRT                    Show printer queue
LISTFILE * EXEC A            List all EXECs on A-disk
```

## Persistent State with GLOBALV

REXX EXECs get a fresh interpreter each time, but GLOBALV variables
persist across invocations:

```rexx
/* COUNTER EXEC */
'GLOBALV SELECT COUNTER'
'GLOBALV GET COUNT'
if rc \= 0 then count = 0
count = count + 1
'GLOBALV SET COUNT' count
say 'Counter:' count
```

Run `COUNTER` multiple times — the value increments.

## Composing EXECs

EXECs can call other EXECs:

```rexx
/* DISPATCH EXEC */
do i = 1 to 3
    'EXEC COUNTER'
end
'EXEC GREET BOB'
```

## Embedding the Library (Rust API)

For embedding vm-iucv as a Rust library:

```toml
[dependencies]
vm-iucv = { git = "https://github.com/navicore/patch-cms" }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```rust
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

    sup.ipl(&alice, vm_iucv::collector::collector().0).await.unwrap();
    sup.ipl(&bob, PrintHandler).await.unwrap();

    sup.smsg(&alice, &bob, "Hello from ALICE!").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    sup.logoff(&alice).await.unwrap();
    sup.logoff(&bob).await.unwrap();
    sup.shutdown().await;
}
```

See the [Examples](EXAMPLES.md) page for more Rust library examples
(echo server, IUCV chat, connection gating, multi-machine pipeline).

## Next steps

- [Examples](EXAMPLES.md) — REXX and Rust examples with walkthroughs
- [vm-iucv Overview](vm-iucv/overview.md) — the actor framework
- [cms-core Overview](cms-core/overview.md) — CMS command processor
- [cms-machine Overview](cms-machine/overview.md) — interactive console
- [API Quick Reference](reference/api-quick-reference.md) — method tables
