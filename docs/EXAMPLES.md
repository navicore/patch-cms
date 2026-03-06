# Examples

## REXX Examples

These examples run inside the CMS machine interactive console. Launch
a machine and try them at the prompt:

```sh
mkdir -p /tmp/cms/a
cargo run -p cms-machine -- --userid ALICE --disk /tmp/cms
```

The example EXECs are in `crates/cms-machine/execs/` and are loaded
automatically from the A-disk.

### GREET — Send an SMSG

Send a greeting message to another machine. Demonstrates SMSG from
REXX with return code checking.

```rexx
/* GREET EXEC — at the CMS prompt: GREET BOB */
parse arg userid .
if userid = '' then do
    say 'Usage: GREET userid'
    exit 24
end

say 'Sending greeting to' userid '...'
'SMSG' userid 'Hello from CMS!'
if rc = 0 then
    say 'Greeting sent successfully.'
else
    say 'Could not reach' userid '— RC='rc
exit rc
```

```
Ready; T=0.01/0.01
GREET BOB
Sending greeting to BOB ...
Greeting sent successfully.
```

---

### COUNTER — Persistent State with GLOBALV

A counter that survives across EXEC invocations. Demonstrates GLOBALV
groups for persistent state management.

```rexx
/* COUNTER EXEC */
parse upper arg action .
'GLOBALV SELECT COUNTER'

if action = 'RESET' then do
    'GLOBALV SET COUNT 0'
    say 'Counter reset to 0.'
    exit 0
end

'GLOBALV GET COUNT'
if rc \= 0 then count = 0

count = count + 1
'GLOBALV SET COUNT' count
say 'Counter:' count
exit 0
```

```
Ready; T=0.01/0.01
COUNTER
Counter: 1
Ready; T=0.01/0.01
COUNTER
Counter: 2
Ready; T=0.01/0.01
COUNTER RESET
Counter reset to 0.
```

---

### UPPER — Data Transformation with PIPE

Uses CMS Pipelines to transform text. Demonstrates the `PIPE` command
from REXX.

```rexx
/* UPPER EXEC */
parse arg text
if text = '' then do
    say 'Usage: UPPER some text here'
    exit 24
end
'PIPE literal' text '| console'
```

```
Ready; T=0.01/0.01
UPPER hello world
hello world
```

---

### SPOOLQ — Spool Queue Status

Queries the virtual reader, printer, and punch queues. Demonstrates
spool commands from REXX.

```rexx
/* SPOOLQ EXEC */
parse upper arg device .
if device = '' then do
    say '--- Reader ---'
    'QUERY RDR'
    say ''
    say '--- Printer ---'
    'QUERY PRT'
    say ''
    say '--- Punch ---'
    'QUERY PUN'
end
```

```
Ready; T=0.01/0.01
SPOOLQ
--- Reader ---
No spool files.

--- Printer ---
No spool files.

--- Punch ---
No spool files.
```

---

### DISPATCH — Multi-EXEC Composition

Composes COUNTER, GREET, and SPOOLQ into a single workflow.
Demonstrates REXX EXECs calling other EXECs.

```rexx
/* DISPATCH EXEC */
parse arg userid .

say '=== Running COUNTER three times ==='
do i = 1 to 3
    'EXEC COUNTER'
end

if userid \= '' then do
    say ''
    say '=== Sending greeting to' userid '==='
    'EXEC GREET' userid
end

say ''
say '=== Spool status ==='
'EXEC SPOOLQ'

say ''
say '=== Done ==='
exit 0
```

```
Ready; T=0.01/0.01
DISPATCH BOB
=== Running COUNTER three times ===
Counter: 1
Counter: 2
Counter: 3

=== Sending greeting to BOB ===
Sending greeting to BOB ...
Greeting sent successfully.

=== Spool status ===
--- Reader ---
No spool files.
...

=== Done ===
```

---

## Rust Library Examples

These examples demonstrate the **vm-iucv library API** for embedding
the actor framework in your own Rust programs. They require Rust and
are run with `cargo`:

### hello_smsg

Basic machine lifecycle: IPL two machines, send an SMSG, shut down.

```sh
cargo run -p vm-iucv --example hello_smsg --features examples
```

### echo_server

Request/reply pattern: an ECHO machine replies to every SMSG with
the same text. Uses `ctx.try_send_smsg()` inside a handler callback.

```sh
cargo run -p vm-iucv --example echo_server --features examples
```

### multi_machine_pipeline

Multi-machine coordination: PRODUCER → TRANSFORM → SINK chain where
each machine forwards modified messages via SMSG.

```sh
cargo run -p vm-iucv --example multi_machine_pipeline --features examples
```

### connection_gating

IUCV connection security: a SECURE machine accepts connections only
from an allowlist. Demonstrates `on_connection_pending` callback.

```sh
cargo run -p vm-iucv --example connection_gating --features examples
```

### iucv_chat

Full IUCV path lifecycle: CONNECT → ESTABLISHED → SEND → SEVER
between two machines.

```sh
cargo run -p vm-iucv --example iucv_chat --features examples
```
