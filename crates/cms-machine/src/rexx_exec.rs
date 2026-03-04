use std::cell::RefCell;
use std::rc::Rc;

use cms_core::command::ExecHandler;
use cms_core::{CmsFileSystem, CommandProcessor, GlobalVars, NoExecHandler, NoSmsgSender};

use patch_rexx::env::Environment;
use patch_rexx::eval::Evaluator;
use patch_rexx::lexer::Lexer;
use patch_rexx::parser::Parser;
use patch_rexx::value::RexxValue;

/// REXX EXEC handler that supports ADDRESS CMS for executing CMS commands
/// from within REXX programs.
///
/// This is the simple variant — each ADDRESS CMS command runs against a
/// temporary CommandProcessor with no persistent filesystem or globalv state.
/// SAY output goes directly to stdout (suitable for interactive sessions).
pub struct CmsRexxExecHandler;

impl ExecHandler for CmsRexxExecHandler {
    fn execute_exec(&mut self, source: &str, args: &str) -> (i32, Vec<String>) {
        match run_rexx_exec(source, args, None, None) {
            Ok((rc, messages, _, _)) => (rc, messages),
            Err(msg) => (28, vec![msg]),
        }
    }
}

/// REXX EXEC handler with access to swappable filesystem and globalv state.
///
/// The handler receives mutable references to the parent processor's components
/// by temporarily taking them during execution and restoring them after.
#[derive(Default)]
pub struct CmsRexxExecHandlerWithSwap {
    /// Shared filesystem — swapped out of the CommandProcessor before REXX
    /// execution and restored after.
    pub filesystem: Option<CmsFileSystem>,
    /// Shared globalv — swapped similarly.
    pub globalv: Option<GlobalVars>,
}

impl CmsRexxExecHandlerWithSwap {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ExecHandler for CmsRexxExecHandlerWithSwap {
    fn execute_exec(&mut self, source: &str, args: &str) -> (i32, Vec<String>) {
        let fs = self.filesystem.take();
        let gv = self.globalv.take();
        match run_rexx_exec(source, args, fs, gv) {
            Ok((rc, messages, fs_back, gv_back)) => {
                self.filesystem = fs_back;
                self.globalv = gv_back;
                (rc, messages)
            }
            Err(msg) => (28, vec![msg]),
        }
    }
}

type RexxExecResult = Result<(i32, Vec<String>, Option<CmsFileSystem>, Option<GlobalVars>), String>;

fn run_rexx_exec(
    source: &str,
    args: &str,
    filesystem: Option<CmsFileSystem>,
    globalv: Option<GlobalVars>,
) -> RexxExecResult {
    // Parse the REXX source
    let mut lexer = Lexer::new(source);
    let tokens = lexer
        .tokenize()
        .map_err(|e| format!("REXX syntax error: {}", e))?;

    let mut parser = Parser::new(tokens);
    let program = parser
        .parse()
        .map_err(|e| format!("REXX parse error: {}", e))?;

    // Set up the REXX environment with ADDRESS CMS
    let mut rexx_env = Environment::new();
    rexx_env.set_address("CMS");

    let mut evaluator = Evaluator::new(&mut rexx_env, &program);

    if !args.is_empty() {
        evaluator.set_main_args(vec![RexxValue::new(args)]);
    }

    // Wrap filesystem and globalv in Rc<RefCell<>> for the command handler closure
    let shared_fs = Rc::new(RefCell::new(filesystem.unwrap_or_default()));
    let shared_gv = Rc::new(RefCell::new(globalv.unwrap_or_default()));
    let output = Rc::new(RefCell::new(Vec::<String>::new()));

    let fs_handle = Rc::clone(&shared_fs);
    let gv_handle = Rc::clone(&shared_gv);
    let output_handle = Rc::clone(&output);

    let handler = move |addr_env: &str,
                        command: &str,
                        _vars: &mut patch_rexx::env::EnvVars<'_>|
          -> Option<i32> {
        let addr_upper = addr_env.to_uppercase();
        if addr_upper != "CMS" && addr_upper != "COMMAND" {
            return None; // fall through to shell
        }

        let cmd_text = command.trim();
        if cmd_text.is_empty() {
            return Some(0);
        }

        // Swap out the shared filesystem and globalv for a temporary processor
        let mut fs_taken = CmsFileSystem::new();
        std::mem::swap(&mut *fs_handle.borrow_mut(), &mut fs_taken);

        let mut gv_taken = GlobalVars::new();
        std::mem::swap(&mut *gv_handle.borrow_mut(), &mut gv_taken);

        // Create temp processor (no EXEC handler — nested EXEC returns RC=28,
        // matching real CMS behavior for deep nesting)
        let mut temp_proc = CommandProcessor::with_smsg_sender(
            fs_taken,
            Box::new(NoExecHandler),
            Box::new(NoSmsgSender),
        );
        *temp_proc.globalv_mut() = gv_taken;

        let result = temp_proc.execute(cmd_text);
        for msg in &result.messages {
            output_handle.borrow_mut().push(msg.clone());
        }

        // Swap state back
        std::mem::swap(&mut *fs_handle.borrow_mut(), temp_proc.filesystem_mut());
        std::mem::swap(&mut *gv_handle.borrow_mut(), temp_proc.globalv_mut());

        Some(result.rc)
    };

    evaluator.set_command_handler_with_env(Box::new(handler));

    // Execute the REXX program
    let exec_result = evaluator
        .exec()
        .map_err(|e| format!("REXX runtime error: {}", e));

    drop(evaluator);

    let messages = Rc::try_unwrap(output)
        .map_err(|_| "REXX: internal state error (multiple references)".to_string())?
        .into_inner();

    let fs_back = Rc::try_unwrap(shared_fs).ok().map(|c| c.into_inner());
    let gv_back = Rc::try_unwrap(shared_gv).ok().map(|c| c.into_inner());

    let rc = match exec_result {
        Ok(_) => 0,
        Err(msg) => {
            let mut msgs = messages;
            msgs.push(msg);
            return Ok((28, msgs, fs_back, gv_back));
        }
    };

    Ok((rc, messages, fs_back, gv_back))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_say_exec() {
        let mut handler = CmsRexxExecHandler;
        let (rc, _) = handler.execute_exec("/* REXX */ say 'hello'", "");
        assert_eq!(rc, 0);
    }

    #[test]
    fn address_cms_globalv() {
        let mut handler = CmsRexxExecHandler;
        let source = r#"/* REXX */
'GLOBALV SET COLOR blue'
"#;
        let (rc, _) = handler.execute_exec(source, "");
        assert_eq!(rc, 0);
    }

    #[test]
    fn syntax_error_returns_rc28() {
        let mut handler = CmsRexxExecHandler;
        let (rc, msgs) = handler.execute_exec("if then else what bad", "");
        assert_eq!(rc, 28);
        assert!(!msgs.is_empty());
    }

    #[test]
    fn args_passed_to_rexx() {
        let mut handler = CmsRexxExecHandler;
        let source = r#"/* REXX */
parse arg target
say target
"#;
        let (rc, _) = handler.execute_exec(source, "OPER");
        assert_eq!(rc, 0);
    }

    #[test]
    fn with_swap_globalv_persists() {
        let mut handler = CmsRexxExecHandlerWithSwap::new();
        handler.filesystem = Some(CmsFileSystem::new());
        handler.globalv = Some(GlobalVars::new());

        let source = r#"/* REXX */
'GLOBALV SET COLOR blue'
"#;
        let (rc, _) = handler.execute_exec(source, "");
        assert_eq!(rc, 0);

        // Verify the globalv was updated through the shared state
        let gv = handler.globalv.as_ref().unwrap();
        assert_eq!(gv.get("COLOR"), Some("blue"));
    }

    #[test]
    fn with_swap_globalv_get() {
        let mut handler = CmsRexxExecHandlerWithSwap::new();
        handler.filesystem = Some(CmsFileSystem::new());
        let mut gv = GlobalVars::new();
        gv.set("FRUIT", "apple");
        handler.globalv = Some(gv);

        let source = r#"/* REXX */
'GLOBALV GET FRUIT'
"#;
        let (rc, msgs) = handler.execute_exec(source, "");
        assert_eq!(rc, 0);
        assert_eq!(msgs, vec!["apple"]);
    }
}
