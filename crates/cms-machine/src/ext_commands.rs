use cms_core::ExtCommandHandler;
use cms_pipelines::run_pipe;
use cms_spool::command::{execute_spool_command, parse_spool_command};
use cms_spool::{InMemoryBackend, SpoolManager};

/// Composite extension command handler that dispatches to spool and pipeline
/// subsystems. Commands not recognized by either are passed through (returns None).
pub struct CmsExtCommandHandler {
    spool: SpoolManager<InMemoryBackend>,
}

impl CmsExtCommandHandler {
    pub fn new(userid: &str) -> Self {
        CmsExtCommandHandler {
            spool: SpoolManager::new(InMemoryBackend::new(), userid),
        }
    }
}

impl ExtCommandHandler for CmsExtCommandHandler {
    fn try_execute(&mut self, input: &str) -> Option<(i32, Vec<String>)> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        // Try PIPE command first (starts with "PIPE" or abbreviated)
        let first_word = trimmed
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if first_word == "PIPE" {
            // Strip the "PIPE" prefix and run the pipeline
            let rest = trimmed
                .split_once(char::is_whitespace)
                .map(|(_, r)| r.trim())
                .unwrap_or("");
            if rest.is_empty() {
                return Some((
                    24,
                    vec!["DMSPIP024E Empty pipeline specification".to_string()],
                ));
            }
            return match run_pipe(rest) {
                Ok(result) => Some((result.rc, result.messages)),
                Err(e) => Some((e.rc(), vec![e.to_string()])),
            };
        }

        // Try spool commands (SPOOL, QUERY, PURGE, SENDFILE, RECEIVE)
        if let Some(cmd) = parse_spool_command(trimmed) {
            let result = execute_spool_command(&cmd, &mut self.spool);
            return Some((result.rc, result.messages));
        }

        None // not a spool or pipeline command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spool_command_handled() {
        let mut handler = CmsExtCommandHandler::new("TESTUSER");
        let result = handler.try_execute("SP PRT CLASS B");
        assert!(result.is_some());
        let (rc, msgs) = result.unwrap();
        assert_eq!(rc, 0);
        assert!(msgs[0].contains("PRINTER"));
    }

    #[test]
    fn query_reader_handled() {
        let mut handler = CmsExtCommandHandler::new("TESTUSER");
        let result = handler.try_execute("Q R");
        assert!(result.is_some());
        let (rc, msgs) = result.unwrap();
        assert_eq!(rc, 0);
        assert!(msgs[0].contains("No files"));
    }

    #[test]
    fn pipe_command_handled() {
        let mut handler = CmsExtCommandHandler::new("TESTUSER");
        let result = handler.try_execute("PIPE literal hello | console");
        assert!(result.is_some());
        let (rc, msgs) = result.unwrap();
        assert_eq!(rc, 0);
        assert_eq!(msgs, vec!["hello"]);
    }

    #[test]
    fn pipe_empty_returns_error() {
        let mut handler = CmsExtCommandHandler::new("TESTUSER");
        let result = handler.try_execute("PIPE");
        assert!(result.is_some());
        let (rc, _) = result.unwrap();
        assert_eq!(rc, 24);
    }

    #[test]
    fn unknown_command_returns_none() {
        let mut handler = CmsExtCommandHandler::new("TESTUSER");
        let result = handler.try_execute("BOGUSCMD stuff");
        assert!(result.is_none());
    }

    #[test]
    fn purge_command_handled() {
        let mut handler = CmsExtCommandHandler::new("TESTUSER");
        let result = handler.try_execute("PUR PRT ALL");
        assert!(result.is_some());
        let (rc, _) = result.unwrap();
        assert_eq!(rc, 0);
    }
}
