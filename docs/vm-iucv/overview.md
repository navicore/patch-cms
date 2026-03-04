# Overview

`vm-iucv` is a lightweight actor framework inspired by IBM's VM/CMS
inter-machine communication facilities. Each actor is a **machine** — an
isolated unit with its own message handler — coordinated by a **Supervisor**
that manages lifecycle, messaging, and IUCV paths.

## Architecture

```
┌──────────────────────────────────────────┐
│                Supervisor                │
│                                          │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐   │
│  │ ALICE   │  │ BOB     │  │ CHARLIE │   │
│  │ handler │  │ handler │  │ handler │   │
│  └────┬────┘  └────┬────┘  └────┬────┘   │
│       │            │            │        │
│  ┌────┴────────────┴────────────┴────┐   │
│  │         Router (SMSG)             │   │
│  └───────────────────────────────────┘   │
│  ┌───────────────────────────────────┐   │
│  │     Path Command Loop (IUCV)      │   │
│  └───────────────────────────────────┘   │
└──────────────────────────────────────────┘
```

## VM/CMS Analogy

| VM/CMS Concept | vm-iucv Equivalent |
|---|---|
| Virtual machine | `MachineId` + `MachineHandler` |
| CP (Control Program) | `Supervisor` |
| IPL (boot) | `supervisor.ipl()` |
| LOGOFF | `supervisor.logoff()` |
| CP SMSG | `supervisor.smsg()` / `ctx.try_send_smsg()` |
| IUCV CONNECT | `supervisor.connect()` |
| IUCV SEVER | `supervisor.sever()` / `ctx.sever_path()` |
| IUCV SEND | `ctx.iucv_send()` |
| IUCV RECEIVE | `on_iucv_data()` callback |

## Key Design Principles

**Fire-and-forget messaging.** SMSG delivery is best-effort, matching the
real CP SMSG semantics. If a target machine's channel is full, the message
is silently dropped.

**Synchronous handlers.** All `MachineHandler` callbacks run synchronously
on the machine's Tokio task. This keeps the programming model simple —
handlers never need to be `async`. Use `ctx.try_send_smsg()` or
`ctx.iucv_send()` to communicate without blocking.

**Path-based connections.** IUCV paths provide a bidirectional data channel
between two machines, with explicit connect/accept/sever lifecycle. Paths
carry binary data (`IucvBuffer`) rather than text.

## Crate Features

| Feature | Description |
|---|---|
| `test-util` | Enables `CollectorHandler` / `CollectorHandle` for testing |
| `examples` | Pulls in tokio runtime features needed for examples |
