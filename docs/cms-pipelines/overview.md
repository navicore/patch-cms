# Overview

The `cms-pipelines` crate implements Hartmann pipelines -- the VM/CMS mechanism
for composing data transformations as a series of stages connected by pipes.

## Pipeline Syntax

Pipelines are invoked with the `PIPE` command. Stages are separated by `|`:

```
PIPE literal Hello world | console
PIPE literal abc | locate /b/ | console
PIPE literal secret | nlocate /secret/ | console
```

Each stage reads records from its input, processes them, and writes records to
its output. The first stage in a pipeline is a source (no input); the last is
typically a sink.

## Built-in Stages

| Stage     | Purpose                                         |
|-----------|-------------------------------------------------|
| `literal` | Emits its argument as a single output record    |
| `console` | Writes each input record to console output      |
| `locate`  | Passes records containing a given string        |
| `nlocate` | Passes records NOT containing a given string    |

The `Stage` trait can be implemented to add custom stages.

## Two-Pass Executor

The pipeline executor runs in two passes:

1. **Initialize** -- Each stage is created and connected to its neighbors via
   input/output channels.
2. **Process + Finish** -- Records flow through the pipeline. Each stage
   processes input records and produces output. When the source is exhausted,
   stages receive a finish signal to flush any buffered state.

## Multi-Stream Support

Stages can produce output on primary and secondary streams. The return code
from a stage determines which output stream receives the record:

| RC | Meaning |
|----|---------|
| 0  | Record written to primary output |
| 4  | Record written to secondary output (e.g., non-matching in `locate`) |
| 12 | End of stage processing |

This allows filter stages like `locate` to split matching and non-matching
records into separate streams.

## Return Codes

Pipeline execution follows CMS conventions:

| RC | Meaning |
|----|---------|
| 0  | Pipeline completed successfully |
| 24 | Syntax error or empty pipeline specification |
| 28 | Stage not found |
| 32 | Pipeline execution error |

## Integration with CMS

The pipeline subsystem is wired into `cms-core`'s `CommandProcessor` through
the `ExtCommandHandler` trait. When a command starts with `PIPE`, the extension
handler strips the prefix and passes the rest to `run_pipe()`. Results (return
code and output messages) are returned through the standard command result
path.
