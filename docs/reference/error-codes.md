# Error Codes

All errors use the `IucvError` enum and display with the IBM `DMSIUC` message
prefix, following VM/CMS conventions.

## Error Variants

| Variant | RC | Message Prefix | Description |
|---|---|---|---|
| `AlreadyRunning(id)` | 8 | `DMSIUC008E` | Machine is already IPL'd |
| `MachineNotFound(id)` | 12 | `DMSIUC012E` | Target machine not logged on |
| `AlreadyLoggedOff(id)` | 12 | `DMSIUC012E` | Machine already logged off |
| `DeliveryFailed(id)` | 16 | `DMSIUC016E` | Message channel closed |
| `ChannelBusy(ch)` | 20 | `DMSIUC020W` | Channel full (transient) |
| `InvalidParameter(msg)` | 24 | `DMSIUC024E` | Invalid parameter value |
| `InvalidMachineId(msg)` | 24 | `DMSIUC024E` | Invalid machine ID format |
| `MachinePanicked(id)` | 28 | `DMSIUC028E` | Machine task panicked |
| `SupervisorDown` | 32 | `DMSIUC032E` | Supervisor has shut down |
| `PathNotFound(id)` | 36 | `DMSIUC036E` | IUCV path not found |
| `ConnectionRefused(msg)` | 40 | `DMSIUC040E` | Target refused connection |

## Using Return Codes

Every error carries a CMS-style numeric return code:

```rust
match supervisor.smsg(&from, &to, "Hello").await {
    Ok(()) => println!("Sent"),
    Err(e) => println!("Failed with RC={}: {}", e.rc(), e),
}
```

## Common Error Scenarios

**IPL a machine twice:**
```
DMSIUC008E Machine already running - ALICE
```

**Send to a machine that isn't logged on:**
```
DMSIUC012E Machine not found - GHOST
```

**SMSG text too long (> 236 bytes):**
```
DMSIUC024E Invalid parameter - SMSG text exceeds 236 bytes
```

**Connection refused by target:**
```
DMSIUC040E Connection refused - SECURE refused connection from UNTRUST
```
