use crate::error::{PipelineError, Result};
use crate::parser::{parse_pipeline, PipelineSpec};
use crate::stage::Stream;
use crate::stages::create_stage;

/// Result of executing a pipeline.
#[derive(Debug)]
pub struct PipelineResult {
    pub rc: i32,
    pub messages: Vec<String>,
}

/// Execute a parsed pipeline specification.
pub fn execute_pipeline(spec: &PipelineSpec) -> Result<PipelineResult> {
    if spec.stages.is_empty() {
        return Err(PipelineError::EmptyPipeline);
    }

    // Build all stages
    let mut stages: Vec<_> = spec
        .stages
        .iter()
        .map(create_stage)
        .collect::<Result<Vec<_>>>()?;

    // Initialize all stages first, collecting any records they emit.
    // This ensures every stage's initialize() runs before any process() calls.
    let stage_count = stages.len();
    let mut pending: Vec<Vec<String>> = Vec::with_capacity(stage_count);
    for stage in stages.iter_mut() {
        let records = stage.initialize()?;
        let primary = records
            .into_iter()
            .filter(|r| r.stream == Stream::Primary)
            .map(|r| r.data)
            .collect();
        pending.push(primary);
    }

    // Now feed initialized records through downstream stages' process().
    for (i, primary) in pending.into_iter().enumerate() {
        feed_records_through(&mut stages, i + 1, primary)?;
    }

    // Finish: call finish() on each stage in order.
    // Emitted records propagate through remaining stages' process() methods.
    for i in 0..stage_count {
        let records = stages[i].finish()?;
        let primary: Vec<String> = records
            .into_iter()
            .filter(|r| r.stream == Stream::Primary)
            .map(|r| r.data)
            .collect();

        feed_records_through(&mut stages, i + 1, primary)?;
    }

    // Collect output from the terminal stage only
    let messages = stages
        .last()
        .map(|s| s.collected_output().to_vec())
        .unwrap_or_default();

    Ok(PipelineResult { rc: 0, messages })
}

/// Feed records through stages starting at `start_index`.
fn feed_records_through(
    stages: &mut [Box<dyn crate::stage::Stage>],
    start_index: usize,
    mut current: Vec<String>,
) -> Result<()> {
    for stage in stages.iter_mut().skip(start_index) {
        let mut next = Vec::new();
        for record in &current {
            let out = stage.process(record)?;
            for r in out {
                if r.stream == Stream::Primary {
                    next.push(r.data);
                }
            }
        }
        current = next;
    }
    Ok(())
}

/// Convenience: parse and execute a pipeline in one call.
pub fn run_pipe(input: &str) -> Result<PipelineResult> {
    let spec = parse_pipeline(input)?;
    execute_pipeline(&spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::StageSpec;

    #[test]
    fn literal_to_console() {
        let result = run_pipe("literal hello | console").unwrap();
        assert_eq!(result.rc, 0);
        assert_eq!(result.messages, vec!["hello"]);
    }

    #[test]
    fn literal_solo() {
        let result = run_pipe("literal hello").unwrap();
        assert_eq!(result.rc, 0);
        assert!(result.messages.is_empty());
    }

    #[test]
    fn console_solo() {
        let result = run_pipe("console").unwrap();
        assert_eq!(result.rc, 0);
        assert!(result.messages.is_empty());
    }

    #[test]
    fn multi_word_literal() {
        let result = run_pipe("literal hello world | console").unwrap();
        assert_eq!(result.rc, 0);
        assert_eq!(result.messages, vec!["hello world"]);
    }

    #[test]
    fn unknown_stage_error() {
        let err = run_pipe("nosuchstage | console").unwrap_err();
        assert_eq!(err.rc(), 28);
    }

    #[test]
    fn empty_pipeline_error() {
        let err = run_pipe("").unwrap_err();
        assert_eq!(err.rc(), 24);
    }

    #[test]
    fn run_pipe_with_pipe_prefix() {
        let result = run_pipe("PIPE literal test | console").unwrap();
        assert_eq!(result.rc, 0);
        assert_eq!(result.messages, vec!["test"]);
    }

    #[test]
    fn pipeline_result_rc_zero() {
        let result = run_pipe("literal x | console").unwrap();
        assert_eq!(result.rc, 0);
    }

    #[test]
    fn literal_empty_string_to_console() {
        let result = run_pipe("literal | console").unwrap();
        assert_eq!(result.rc, 0);
        assert_eq!(result.messages, vec![""]);
    }

    #[test]
    fn three_stage_pipeline() {
        // literal -> console (sink, absorbs) -> console (gets nothing)
        let result = run_pipe("literal hello | console | console").unwrap();
        assert_eq!(result.rc, 0);
        // Only terminal stage's collected_output is returned
        assert!(result.messages.is_empty());
    }

    #[test]
    fn empty_spec_bypasses_parser() {
        // Directly construct an empty PipelineSpec to test executor guard
        let spec = PipelineSpec { stages: vec![] };
        let err = execute_pipeline(&spec).unwrap_err();
        assert!(matches!(err, PipelineError::EmptyPipeline));
        assert_eq!(err.rc(), 24);
    }

    #[test]
    fn unknown_stage_in_execute_pipeline() {
        let spec = PipelineSpec {
            stages: vec![StageSpec {
                name: "bogus".to_string(),
                raw_name: "BOGUS".to_string(),
                args: String::new(),
            }],
        };
        let err = execute_pipeline(&spec).unwrap_err();
        assert_eq!(err.rc(), 28);
    }

    #[test]
    fn passthrough_stage_forwards_records() {
        use crate::stage::Stage;
        use crate::stages::create_stage;

        // Register an inline Echo stage via a custom PipelineSpec + manual execution
        #[derive(Debug)]
        struct Echo;
        impl Stage for Echo {
            fn name(&self) -> &str {
                "echo"
            }
            // Relies on default pass-through process() and no-op initialize()
        }

        // Build stages manually: literal -> echo -> console
        let specs = vec![
            StageSpec {
                name: "literal".to_string(),
                raw_name: "literal".to_string(),
                args: "hello".to_string(),
            },
            StageSpec {
                name: "console".to_string(),
                raw_name: "console".to_string(),
                args: String::new(),
            },
        ];

        // Build with echo injected in the middle
        let mut stages: Vec<Box<dyn Stage>> = Vec::new();
        stages.push(create_stage(&specs[0]).unwrap()); // literal
        stages.push(Box::new(Echo)); // echo pass-through
        stages.push(create_stage(&specs[1]).unwrap()); // console

        // Run the two-pass initialize
        let stage_count = stages.len();
        let mut pending: Vec<Vec<String>> = Vec::with_capacity(stage_count);
        for stage in stages.iter_mut() {
            let records = stage.initialize().unwrap();
            let primary = records
                .into_iter()
                .filter(|r| r.stream == Stream::Primary)
                .map(|r| r.data)
                .collect();
            pending.push(primary);
        }
        for (i, primary) in pending.into_iter().enumerate() {
            feed_records_through(&mut stages, i + 1, primary).unwrap();
        }

        // Finish
        for i in 0..stage_count {
            let records = stages[i].finish().unwrap();
            let primary: Vec<String> = records
                .into_iter()
                .filter(|r| r.stream == Stream::Primary)
                .map(|r| r.data)
                .collect();
            feed_records_through(&mut stages, i + 1, primary).unwrap();
        }

        // Terminal console should have the record
        let messages = stages.last().unwrap().collected_output();
        assert_eq!(messages, &["hello"]);
    }
}
