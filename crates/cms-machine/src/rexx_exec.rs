use std::cell::RefCell;
use std::rc::Rc;

use cms_core::command::ExecHandler;
use cms_core::{
    CmsCommand, CmsFileSystem, CommandProcessor, GlobalVars, GlobalvSubcommand, NoSmsgSender,
    SmsgSender,
};

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
        match run_rexx_exec(source, args, None, None, None) {
            Ok((rc, messages, _, _, _)) => (rc, messages),
            Err((msg, _, _, _)) => (28, vec![msg]),
        }
    }
}

/// REXX EXEC handler with access to swappable filesystem and globalv state.
///
/// The handler receives mutable references to the parent processor's components
/// by temporarily taking them during execution and restoring them after.
/// The CommandProcessor calls `provide_state`/`retrieve_state` around each EXEC.
pub struct CmsRexxExecHandlerWithSwap {
    /// Shared filesystem — swapped out of the CommandProcessor before REXX
    /// execution and restored after.
    pub(crate) filesystem: Option<CmsFileSystem>,
    /// Shared globalv — swapped similarly.
    pub(crate) globalv: Option<GlobalVars>,
    /// Shared SMSG sender — swapped similarly.
    pub(crate) smsg_sender: Option<Box<dyn SmsgSender>>,
}

impl CmsRexxExecHandlerWithSwap {
    pub fn new() -> Self {
        CmsRexxExecHandlerWithSwap {
            filesystem: None,
            globalv: None,
            smsg_sender: None,
        }
    }
}

impl Default for CmsRexxExecHandlerWithSwap {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecHandler for CmsRexxExecHandlerWithSwap {
    fn execute_exec(&mut self, source: &str, args: &str) -> (i32, Vec<String>) {
        let fs = self.filesystem.take();
        let gv = self.globalv.take();
        let smsg = self.smsg_sender.take();
        match run_rexx_exec(source, args, fs, gv, smsg) {
            Ok((rc, messages, fs_back, gv_back, smsg_back)) => {
                self.filesystem = fs_back;
                self.globalv = gv_back;
                self.smsg_sender = smsg_back;
                (rc, messages)
            }
            Err((msg, fs_back, gv_back, smsg_back)) => {
                // Issue #4 fix: restore state even on error path
                self.filesystem = fs_back;
                self.globalv = gv_back;
                self.smsg_sender = smsg_back;
                (28, vec![msg])
            }
        }
    }

    fn provide_state(
        &mut self,
        fs: CmsFileSystem,
        gv: GlobalVars,
    ) -> Option<(CmsFileSystem, GlobalVars)> {
        self.filesystem = Some(fs);
        self.globalv = Some(gv);
        None // accepted
    }

    fn retrieve_state(&mut self) -> Option<(CmsFileSystem, GlobalVars)> {
        match (self.filesystem.take(), self.globalv.take()) {
            (Some(fs), Some(gv)) => Some((fs, gv)),
            _ => None,
        }
    }

    fn provide_smsg_sender(&mut self, sender: Box<dyn SmsgSender>) -> Option<Box<dyn SmsgSender>> {
        self.smsg_sender = Some(sender);
        None // accepted
    }

    fn retrieve_smsg_sender(&mut self) -> Option<Box<dyn SmsgSender>> {
        self.smsg_sender.take()
    }
}

/// Error type that preserves filesystem/globalv/smsg state for the caller to recover.
type RexxExecResult = Result<
    (
        i32,
        Vec<String>,
        Option<CmsFileSystem>,
        Option<GlobalVars>,
        Option<Box<dyn SmsgSender>>,
    ),
    (
        String,
        Option<CmsFileSystem>,
        Option<GlobalVars>,
        Option<Box<dyn SmsgSender>>,
    ),
>;

fn run_rexx_exec(
    source: &str,
    args: &str,
    filesystem: Option<CmsFileSystem>,
    globalv: Option<GlobalVars>,
    smsg_sender: Option<Box<dyn SmsgSender>>,
) -> RexxExecResult {
    // Parse the REXX source — errors here happen before state is wrapped,
    // so return the original filesystem/globalv on the error path.
    let mut lexer = Lexer::new(source);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(e) => {
            return Err((
                format!("REXX syntax error: {}", e),
                filesystem,
                globalv,
                smsg_sender,
            ))
        }
    };

    let mut parser = Parser::new(tokens);
    let program = match parser.parse() {
        Ok(p) => p,
        Err(e) => {
            return Err((
                format!("REXX parse error: {}", e),
                filesystem,
                globalv,
                smsg_sender,
            ))
        }
    };

    // Set up the REXX environment with ADDRESS CMS
    let mut rexx_env = Environment::new();
    rexx_env.set_address("CMS");

    let mut evaluator = Evaluator::new(&mut rexx_env, &program);

    if !args.is_empty() {
        evaluator.set_main_args(vec![RexxValue::new(args)]);
    }

    // Wrap filesystem, globalv, and smsg_sender in Rc<RefCell<>> for the closure
    let shared_fs = Rc::new(RefCell::new(filesystem.unwrap_or_default()));
    let shared_gv = Rc::new(RefCell::new(globalv.unwrap_or_default()));
    let shared_smsg: Rc<RefCell<Option<Box<dyn SmsgSender>>>> = Rc::new(RefCell::new(smsg_sender));
    let output = Rc::new(RefCell::new(Vec::<String>::new()));

    let fs_handle = Rc::clone(&shared_fs);
    let gv_handle = Rc::clone(&shared_gv);
    let smsg_handle = Rc::clone(&shared_smsg);
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

        // Take SMSG sender for the temp processor
        let smsg_taken: Box<dyn SmsgSender> = smsg_handle
            .borrow_mut()
            .take()
            .unwrap_or_else(|| Box::new(NoSmsgSender));

        // Create temp processor with nested EXEC support (one level) and SMSG
        let nested_exec: Box<dyn ExecHandler> = Box::new(CmsRexxExecHandlerWithSwap::new());
        let mut temp_proc = CommandProcessor::with_smsg_sender(fs_taken, nested_exec, smsg_taken);
        *temp_proc.globalv_mut() = gv_taken;

        let result = temp_proc.execute(cmd_text);

        // Write GLOBALV GET results back to the REXX variable pool.
        if result.rc == 0 || result.rc == 4 {
            if let Ok(CmsCommand::Globalv(GlobalvSubcommand::Get(names))) =
                cms_core::command::parse_cms_command(cmd_text)
            {
                for (name, val) in names.iter().zip(result.messages.iter()) {
                    vars.set(&name.to_ascii_uppercase(), RexxValue::new(val));
                }
            }
        }

        for msg in &result.messages {
            output_handle.borrow_mut().push(msg.clone());
        }

        // Swap state back
        std::mem::swap(&mut *fs_handle.borrow_mut(), temp_proc.filesystem_mut());
        std::mem::swap(&mut *gv_handle.borrow_mut(), temp_proc.globalv_mut());

        // Reclaim SMSG sender
        let smsg_back = temp_proc.take_smsg_sender();
        *smsg_handle.borrow_mut() = Some(smsg_back);

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
            let smsg_back = Rc::try_unwrap(shared_smsg)
                .ok()
                .and_then(|c| c.into_inner());
            return Err((
                "REXX: internal state error (multiple references)".to_string(),
                fs_back,
                gv_back,
                smsg_back,
            ));
        }
    };

    // Safe: evaluator is dropped above, so all Rc clones from the command handler
    // closure are gone. If this ever panics, a leaked Rc clone exists.
    let fs_back = Some(
        Rc::try_unwrap(shared_fs)
            .unwrap_or_else(|_| panic!("filesystem Rc still shared after evaluator drop"))
            .into_inner(),
    );
    let gv_back = Some(
        Rc::try_unwrap(shared_gv)
            .unwrap_or_else(|_| panic!("globalv Rc still shared after evaluator drop"))
            .into_inner(),
    );
    let smsg_back = Rc::try_unwrap(shared_smsg)
        .unwrap_or_else(|_| panic!("smsg_sender Rc still shared after evaluator drop"))
        .into_inner();

    // Issue #1 fix: extract REXX exit code from ExecSignal
    let mut messages = messages;
    let rc = match exec_result {
        Ok(signal) => extract_rc(&signal, &mut messages),
        Err(msg) => {
            messages.push(msg);
            return Ok((28, messages, fs_back, gv_back, smsg_back));
        }
    };

    Ok((rc, messages, fs_back, gv_back, smsg_back))
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

    #[test]
    fn globalv_get_partial_miss_sets_found_vars() {
        let mut handler = CmsRexxExecHandlerWithSwap::new();
        handler.filesystem = Some(CmsFileSystem::new());
        let mut gv = GlobalVars::new();
        gv.set("COLOR", "blue");
        // FRUIT is intentionally not set
        handler.globalv = Some(gv);

        // GLOBALV GET COLOR FRUIT — COLOR exists, FRUIT does not.
        // COLOR should still be written back to the REXX variable pool.
        let source = r#"/* REXX */
'GLOBALV GET COLOR FRUIT'
if color = 'blue' then
    exit 0
else
    exit 99
"#;
        let (rc, _) = handler.execute_exec(source, "");
        assert_eq!(
            rc, 0,
            "COLOR should be set in REXX pool despite FRUIT missing"
        );
    }

    #[test]
    fn nested_exec_works() {
        use cms_core::minidisk::AccessMode;

        // Set up a filesystem with an A-disk containing a NESTED EXEC file
        // that sets a GLOBALV variable.
        let dir = tempfile::TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        let spec = cms_core::FileSpec::parse("INNER EXEC A").unwrap();
        fs.write_file(&spec, "/* REXX */\n'GLOBALV SET NESTED yes'\n")
            .unwrap();

        let mut handler = CmsRexxExecHandlerWithSwap::new();
        handler.filesystem = Some(fs);
        handler.globalv = Some(GlobalVars::new());

        // Outer REXX calls EXEC INNER, then checks the GLOBALV it set
        let source = r#"/* REXX */
'EXEC INNER'
'GLOBALV GET NESTED'
if nested = 'yes' then
    exit 0
else
    exit 99
"#;
        let (rc, _) = handler.execute_exec(source, "");
        assert_eq!(rc, 0, "Nested EXEC should work and GLOBALV should persist");

        // Verify globalv persists back to handler
        let gv = handler.globalv.as_ref().unwrap();
        assert_eq!(gv.get("NESTED"), Some("yes"));
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
