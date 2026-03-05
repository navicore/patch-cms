use std::cell::RefCell;
use std::rc::Rc;

use cms_core::command::ExecHandler;
use cms_core::{CmsFileSystem, CommandProcessor, GlobalVars, NoExecHandler, NoSmsgSender};

use patch_rexx::env::Environment;
use patch_rexx::eval::{Evaluator, ExecSignal};
use patch_rexx::lexer::Lexer;
use patch_rexx::parser::Parser;
use patch_rexx::value::RexxValue;

/// REXX EXEC handler that supports ADDRESS CMS for executing CMS commands
/// from within REXX programs.
///
/// This is the simple variant — each ADDRESS CMS command runs against a
/// temporary CommandProcessor with no persistent filesystem or globalv state.
///
/// **Known limitation:** SAY output goes directly to stdout via the patch-rexx
/// interpreter and is not captured in the returned message vector. This is
/// acceptable for interactive sessions where stdout is the console.
pub struct CmsRexxExecHandler;

impl ExecHandler for CmsRexxExecHandler {
    fn execute_exec(&mut self, source: &str, args: &str) -> (i32, Vec<String>) {
        match run_rexx_exec(source, args, None, None) {
            Ok((rc, messages, _, _)) => (rc, messages),
            Err((msg, _, _)) => (28, vec![msg]),
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
    pub(crate) filesystem: Option<CmsFileSystem>,
    /// Shared globalv — swapped similarly.
    pub(crate) globalv: Option<GlobalVars>,
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
            Err((msg, fs_back, gv_back)) => {
                // Issue #4 fix: restore state even on error path
                self.filesystem = fs_back;
                self.globalv = gv_back;
                (28, vec![msg])
            }
        }
    }
}

/// Error type that preserves filesystem/globalv state for the caller to recover.
type RexxExecResult = Result<
    (i32, Vec<String>, Option<CmsFileSystem>, Option<GlobalVars>),
    (String, Option<CmsFileSystem>, Option<GlobalVars>),
>;

fn run_rexx_exec(
    source: &str,
    args: &str,
    filesystem: Option<CmsFileSystem>,
    globalv: Option<GlobalVars>,
) -> RexxExecResult {
    // Parse the REXX source — errors here happen before state is wrapped,
    // so return the original filesystem/globalv on the error path.
    let mut lexer = Lexer::new(source);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(e) => return Err((format!("REXX syntax error: {}", e), filesystem, globalv)),
    };

    let mut parser = Parser::new(tokens);
    let program = match parser.parse() {
        Ok(p) => p,
        Err(e) => return Err((format!("REXX parse error: {}", e), filesystem, globalv)),
    };

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

    // Issue #2 fix: use vars to write GLOBALV GET results back to the REXX variable pool
    let handler = move |addr_env: &str,
                        command: &str,
                        vars: &mut patch_rexx::env::EnvVars<'_>|
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

        // Create temp processor — nested EXEC is not supported at this depth
        // (NoExecHandler returns RC=28 if a nested EXEC is attempted).
        let mut temp_proc = CommandProcessor::with_smsg_sender(
            fs_taken,
            Box::new(NoExecHandler),
            Box::new(NoSmsgSender),
        );
        *temp_proc.globalv_mut() = gv_taken;

        let result = temp_proc.execute(cmd_text);

        // Write GLOBALV GET results back to the REXX variable pool.
        // GLOBALV GET VAR1 VAR2 ... retrieves values and sets each variable
        // in the caller's environment — this is the CMS convention.
        let tokens: Vec<&str> = cmd_text.split_whitespace().collect();
        let first_upper = tokens
            .first()
            .map(|t| t.to_ascii_uppercase())
            .unwrap_or_default();
        let is_globalv = first_upper == "GLOBALV"
            || (first_upper.len() >= 4 && "GLOBALV".starts_with(&first_upper));
        if is_globalv
            && tokens.len() >= 3
            && tokens[1].eq_ignore_ascii_case("GET")
            && result.rc == 0
        {
            for (var_tok, val) in tokens[2..].iter().zip(result.messages.iter()) {
                vars.set(&var_tok.to_ascii_uppercase(), RexxValue::new(val));
            }
        }

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

    // Unwrap shared state — guaranteed to succeed since evaluator is dropped
    let messages = match Rc::try_unwrap(output) {
        Ok(cell) => cell.into_inner(),
        Err(_rc) => {
            let fs_back = Rc::try_unwrap(shared_fs).ok().map(|c| c.into_inner());
            let gv_back = Rc::try_unwrap(shared_gv).ok().map(|c| c.into_inner());
            return Err((
                "REXX: internal state error (multiple references)".to_string(),
                fs_back,
                gv_back,
            ));
        }
    };

    let fs_back = Rc::try_unwrap(shared_fs).ok().map(|c| c.into_inner());
    let gv_back = Rc::try_unwrap(shared_gv).ok().map(|c| c.into_inner());

    // Issue #1 fix: extract REXX exit code from ExecSignal
    let mut messages = messages;
    let rc = match exec_result {
        Ok(signal) => extract_rc(&signal, &mut messages),
        Err(msg) => {
            messages.push(msg);
            return Ok((28, messages, fs_back, gv_back));
        }
    };

    Ok((rc, messages, fs_back, gv_back))
}

/// Extract a numeric return code from a REXX ExecSignal.
///
/// - `Exit(Some(value))` / `Return(Some(value))`: parse as i32; RC=20 if non-numeric
/// - `Normal` / `Exit(None)` / `Return(None)`: RC=0
fn extract_rc(signal: &ExecSignal, messages: &mut Vec<String>) -> i32 {
    match signal {
        ExecSignal::Exit(Some(val)) | ExecSignal::Return(Some(val)) => {
            let s = val.as_str();
            match s.parse::<i32>() {
                Ok(n) => n,
                Err(_) => {
                    messages.push(format!("DMSERR020E Non-numeric return code: {}", s));
                    20
                }
            }
        }
        _ => 0,
    }
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

    // Issue #1: exit code is propagated
    #[test]
    fn exit_code_propagated() {
        let mut handler = CmsRexxExecHandler;
        let source = "/* REXX */\nexit 4\n";
        let (rc, _) = handler.execute_exec(source, "");
        assert_eq!(rc, 4);
    }

    #[test]
    fn exit_code_24_from_exec() {
        let mut handler = CmsRexxExecHandler;
        let source = "/* REXX */\nexit 24\n";
        let (rc, _) = handler.execute_exec(source, "");
        assert_eq!(rc, 24);
    }

    #[test]
    fn exit_no_value_is_rc0() {
        let mut handler = CmsRexxExecHandler;
        let source = "/* REXX */\nexit\n";
        let (rc, _) = handler.execute_exec(source, "");
        assert_eq!(rc, 0);
    }

    #[test]
    fn exit_non_numeric_is_rc20() {
        let mut handler = CmsRexxExecHandler;
        let source = "/* REXX */\nexit 'FAIL'\n";
        let (rc, msgs) = handler.execute_exec(source, "");
        assert_eq!(rc, 20);
        assert!(
            msgs.iter().any(|m| m.contains("Non-numeric")),
            "Expected non-numeric error, got: {:?}",
            msgs,
        );
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

    // Issue #2: GLOBALV GET writes to REXX variable pool
    #[test]
    fn globalv_get_sets_rexx_variable() {
        let mut handler = CmsRexxExecHandlerWithSwap::new();
        handler.filesystem = Some(CmsFileSystem::new());
        let mut gv = GlobalVars::new();
        gv.set("COLOR", "blue");
        handler.globalv = Some(gv);

        // REXX program does GLOBALV GET COLOR, then uses the variable
        let source = r#"/* REXX */
'GLOBALV GET COLOR'
if color = 'blue' then
    exit 0
else
    exit 99
"#;
        let (rc, _) = handler.execute_exec(source, "");
        assert_eq!(rc, 0, "GLOBALV GET should set the REXX variable 'color'");
    }

    #[test]
    fn globalv_get_multi_var_writeback() {
        let mut handler = CmsRexxExecHandlerWithSwap::new();
        handler.filesystem = Some(CmsFileSystem::new());
        let mut gv = GlobalVars::new();
        gv.set("COLOR", "blue");
        gv.set("FRUIT", "apple");
        handler.globalv = Some(gv);

        let source = r#"/* REXX */
'GLOBALV GET COLOR FRUIT'
if color = 'blue' & fruit = 'apple' then
    exit 0
else
    exit 99
"#;
        let (rc, _) = handler.execute_exec(source, "");
        assert_eq!(rc, 0, "GLOBALV GET should set multiple REXX variables");
    }

    // Issue #4: state preserved on error path
    #[test]
    fn with_swap_state_preserved_on_syntax_error() {
        let mut handler = CmsRexxExecHandlerWithSwap::new();
        let mut gv = GlobalVars::new();
        gv.set("KEY", "preserved");
        handler.filesystem = Some(CmsFileSystem::new());
        handler.globalv = Some(gv);

        let (rc, _) = handler.execute_exec("if then bad syntax", "");
        assert_eq!(rc, 28);

        // State must still be present
        assert!(handler.filesystem.is_some(), "filesystem lost on error");
        let gv = handler.globalv.as_ref().expect("globalv lost on error");
        assert_eq!(gv.get("KEY"), Some("preserved"));
    }
}
