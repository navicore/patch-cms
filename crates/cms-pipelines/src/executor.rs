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

    // Initialize: call initialize() on each stage in order.
    // Primary records from stage N flow into stage N+1's process().
    let stage_count = stages.len();
    for i in 0..stage_count {
        let records = stages[i].initialize()?;
        let primary: Vec<String> = records
            .into_iter()
            .filter(|r| r.stream == Stream::Primary)
            .map(|r| r.data)
            .collect();

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
                args: String::new(),
            }],
        };
        let err = execute_pipeline(&spec).unwrap_err();
        assert_eq!(err.rc(), 28);
    }
}
