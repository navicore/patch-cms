pub mod error;
pub mod executor;
pub mod parser;
pub mod stage;
pub mod stages;

pub use error::PipelineError;
pub use executor::{run_pipe, PipelineResult};
