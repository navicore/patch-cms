pub mod console;
pub mod literal;
pub mod locate;
mod locate_common;
pub mod nlocate;

use crate::error::{PipelineError, Result};
use crate::parser::StageSpec;
use crate::stage::Stage;

/// Create a stage instance from a parsed stage specification.
pub fn create_stage(spec: &StageSpec) -> Result<Box<dyn Stage>> {
    match spec.name.as_str() {
        "literal" => Ok(Box::new(literal::Literal::new(spec.args.clone()))),
        "console" => Ok(Box::new(console::Console::new())),
        "locate" => Ok(Box::new(locate::Locate::new(&spec.args)?)),
        "nlocate" => Ok(Box::new(nlocate::Nlocate::new(&spec.args)?)),
        _ => Err(PipelineError::UnknownStage(spec.raw_name.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_creates_literal() {
        let spec = StageSpec {
            name: "literal".to_string(),
            raw_name: "literal".to_string(),
            args: "hello".to_string(),
        };
        let stage = create_stage(&spec).unwrap();
        assert_eq!(stage.name(), "literal");
    }

    #[test]
    fn factory_creates_console() {
        let spec = StageSpec {
            name: "console".to_string(),
            raw_name: "console".to_string(),
            args: String::new(),
        };
        let stage = create_stage(&spec).unwrap();
        assert_eq!(stage.name(), "console");
    }

    #[test]
    fn factory_creates_locate() {
        let spec = StageSpec {
            name: "locate".to_string(),
            raw_name: "locate".to_string(),
            args: "/test/".to_string(),
        };
        let stage = create_stage(&spec).unwrap();
        assert_eq!(stage.name(), "locate");
    }

    #[test]
    fn factory_creates_nlocate() {
        let spec = StageSpec {
            name: "nlocate".to_string(),
            raw_name: "nlocate".to_string(),
            args: "/test/".to_string(),
        };
        let stage = create_stage(&spec).unwrap();
        assert_eq!(stage.name(), "nlocate");
    }

    #[test]
    fn factory_unknown_stage_error() {
        let spec = StageSpec {
            name: "nosuchstage".to_string(),
            raw_name: "NOSUCHSTAGE".to_string(),
            args: String::new(),
        };
        let err = create_stage(&spec).unwrap_err();
        assert_eq!(err.rc(), 28);
        // Error message uses original case
        assert!(err.to_string().contains("NOSUCHSTAGE"));
    }
}
