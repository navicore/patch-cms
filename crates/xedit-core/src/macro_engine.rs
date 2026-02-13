//! REXX macro engine for XEDIT.
//!
//! Macros are REXX programs that can:
//! - Query editor state via pre-populated EXTRACT variables
//! - Execute XEDIT commands via ADDRESS XEDIT (bare string expressions)
//! - Return a result code
//!
//! # Example macro
//! ```rexx
//! /* CENTER.XEDIT — center text on current line */
//! 'EXTRACT /CURLINE/TRUNC/'
//! text = strip(curline.3)
//! pad = (trunc.1 - length(text)) % 2
//! 'COMMAND REPLACE' copies(' ', pad) || text
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use patch_rexx::env::Environment;
use patch_rexx::eval::Evaluator;
use patch_rexx::lexer::Lexer;
use patch_rexx::parser::Parser;
use patch_rexx::value::RexxValue;

use crate::command::parse_command;
use crate::editor::Editor;
use crate::error::{Result, XeditError};

/// Run a REXX macro against the editor.
///
/// The macro executes with ADDRESS set to XEDIT. Bare string expressions
/// (like `'LOCATE /foo/'`) are dispatched to the editor as XEDIT commands.
/// EXTRACT-style stem variables are pre-populated before execution.
pub fn run_macro(editor: &mut Editor, source: &str, args: &str) -> Result<()> {
    // Parse the REXX source
    let mut lexer = Lexer::new(source);
    let tokens = lexer
        .tokenize()
        .map_err(|e| XeditError::InvalidCommand(format!("REXX syntax error: {}", e)))?;

    let mut parser = Parser::new(tokens);
    let program = parser
        .parse()
        .map_err(|e| XeditError::InvalidCommand(format!("REXX parse error: {}", e)))?;

    // Set up the REXX environment
    let mut rexx_env = Environment::new();
    rexx_env.set_address("XEDIT");

    // Pre-populate EXTRACT variables
    populate_extract_vars(&mut rexx_env, editor);

    // Temporarily move editor state into an Rc<RefCell> so the command handler
    // closure (which must be 'static) can access it. We swap in an empty editor,
    // run the macro, then swap back.
    let mut placeholder = Editor::new();
    std::mem::swap(editor, &mut placeholder);
    let shared_editor = Rc::new(RefCell::new(placeholder));

    // Create the evaluator with a command handler that dispatches to XEDIT
    let mut evaluator = Evaluator::new(&mut rexx_env, &program);

    if !args.is_empty() {
        evaluator.set_main_args(vec![RexxValue::new(args)]);
    }

    // The command handler intercepts ADDRESS XEDIT commands
    let editor_handle = Rc::clone(&shared_editor);
    let handler = move |addr_env: &str, command: &str| -> Option<i32> {
        let addr_upper = addr_env.to_uppercase();
        if addr_upper != "XEDIT" && addr_upper != "COMMAND" {
            return None; // fall through to shell for other environments
        }

        // Strip leading "COMMAND " prefix if present (XEDIT convention)
        let cmd_text = command
            .strip_prefix("COMMAND ")
            .or_else(|| command.strip_prefix("command "))
            .unwrap_or(command)
            .trim();

        if cmd_text.is_empty() {
            return Some(0);
        }

        // Handle EXTRACT specially — pre-populated before execution
        if cmd_text.to_uppercase().starts_with("EXTRACT ") {
            return Some(0);
        }

        // Parse and execute the XEDIT command
        let mut ed = editor_handle.borrow_mut();
        match parse_command(cmd_text) {
            Ok(cmd) => match ed.execute(&cmd) {
                Ok(_result) => Some(0),
                Err(_e) => Some(1),
            },
            Err(_e) => Some(1),
        }
    };

    evaluator.set_command_handler(Box::new(handler));

    // Execute the macro
    let result = evaluator
        .exec()
        .map_err(|e| XeditError::InvalidCommand(format!("REXX runtime error: {}", e)));

    // Swap the editor state back, dropping the Rc wrapper
    drop(evaluator); // release the handler's Rc clone
    let recovered = Rc::try_unwrap(shared_editor)
        .expect("macro engine: editor Rc should have single owner after evaluator drop")
        .into_inner();
    *editor = recovered;

    result?;
    Ok(())
}

/// Pre-populate REXX environment with EXTRACT-style stem variables.
///
/// This follows the IBM XEDIT EXTRACT convention where each item
/// sets stem variables: `item.0` = count, `item.1` = first value, etc.
fn populate_extract_vars(env: &mut Environment, editor: &Editor) {
    // CURLINE: current line number and text
    let curline_num = editor.current_line().to_string();
    let curline_text = editor.current_line_text().to_string();
    env.set_compound("CURLINE", "0", RexxValue::new("3"));
    env.set_compound("CURLINE", "1", RexxValue::new(&curline_num));
    env.set_compound(
        "CURLINE",
        "2",
        RexxValue::new(if editor.at_tof() { "ON" } else { "OFF" }),
    );
    env.set_compound("CURLINE", "3", RexxValue::new(&curline_text));

    // SIZE: number of lines in the file
    let size = editor.buffer().len().to_string();
    env.set_compound("SIZE", "0", RexxValue::new("1"));
    env.set_compound("SIZE", "1", RexxValue::new(&size));

    // LINE: current line number
    env.set_compound("LINE", "0", RexxValue::new("1"));
    env.set_compound("LINE", "1", RexxValue::new(&curline_num));

    // COLUMN: current column
    let col = editor.current_col().to_string();
    env.set_compound("COLUMN", "0", RexxValue::new("1"));
    env.set_compound("COLUMN", "1", RexxValue::new(&col));

    // FNAME: filename
    env.set_compound("FNAME", "0", RexxValue::new("1"));
    env.set_compound("FNAME", "1", RexxValue::new(editor.filename()));

    // FTYPE: filetype
    env.set_compound("FTYPE", "0", RexxValue::new("1"));
    env.set_compound("FTYPE", "1", RexxValue::new(editor.filetype()));

    // FMODE: filemode
    env.set_compound("FMODE", "0", RexxValue::new("1"));
    env.set_compound("FMODE", "1", RexxValue::new(editor.filemode()));

    // TRUNC: truncation column
    let trunc = editor.trunc().to_string();
    env.set_compound("TRUNC", "0", RexxValue::new("1"));
    env.set_compound("TRUNC", "1", RexxValue::new(&trunc));

    // ALT: alteration count
    let alt = editor.alt_count().to_string();
    env.set_compound("ALT", "0", RexxValue::new("1"));
    env.set_compound("ALT", "1", RexxValue::new(&alt));

    // TOF: whether at top of file
    env.set_compound("TOF", "0", RexxValue::new("1"));
    env.set_compound(
        "TOF",
        "1",
        RexxValue::new(if editor.at_tof() { "ON" } else { "OFF" }),
    );

    // EOF: whether at end of file
    env.set_compound("EOF", "0", RexxValue::new("1"));
    env.set_compound(
        "EOF",
        "1",
        RexxValue::new(if editor.at_eof() { "ON" } else { "OFF" }),
    );

    // MODIFIED: whether file has been changed
    env.set_compound("MODIFIED", "0", RexxValue::new("1"));
    env.set_compound(
        "MODIFIED",
        "1",
        RexxValue::new(if editor.is_modified() { "ON" } else { "OFF" }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with_lines(lines: &[&str]) -> Editor {
        let mut ed = Editor::new();
        // Use input_line to add lines since we can't access buffer directly
        for line in lines {
            ed.input_line(line);
        }
        // Reset to line 1
        ed.set_current_line(1);
        ed
    }

    #[test]
    fn macro_navigate_down() {
        let mut ed = editor_with_lines(&["alpha", "beta", "gamma"]);
        assert_eq!(ed.current_line(), 1);

        run_macro(&mut ed, "'DOWN 2'", "").unwrap();
        assert_eq!(ed.current_line(), 3);
    }

    #[test]
    fn macro_locate() {
        let mut ed = editor_with_lines(&["alpha", "beta", "gamma"]);

        run_macro(&mut ed, "'LOCATE /gamma/'", "").unwrap();
        assert_eq!(ed.current_line(), 3);
    }

    #[test]
    fn macro_change() {
        let mut ed = editor_with_lines(&["hello world", "hello there"]);

        run_macro(&mut ed, "'CHANGE /hello/hi/ * 2'", "").unwrap();
        assert_eq!(ed.buffer().line_text(1), Some("hi world"));
        assert_eq!(ed.buffer().line_text(2), Some("hi there"));
    }

    #[test]
    fn macro_with_rexx_logic() {
        let mut ed = editor_with_lines(&["aaa", "bbb", "ccc"]);

        let source = r#"
            do i = 1 to 2
                'DOWN 1'
            end
        "#;

        run_macro(&mut ed, source, "").unwrap();
        assert_eq!(ed.current_line(), 3);
    }

    #[test]
    fn macro_reads_extract_vars() {
        let mut ed = editor_with_lines(&["first", "second", "third"]);
        ed.set_current_line(2);

        // Macro that uses pre-populated EXTRACT variables
        let source = r#"
            if curline.1 = 2 then
                'DOWN 1'
        "#;

        run_macro(&mut ed, source, "").unwrap();
        assert_eq!(ed.current_line(), 3);
    }

    #[test]
    fn macro_with_args() {
        let mut ed = editor_with_lines(&["alpha", "beta", "gamma"]);

        let source = r#"
            parse arg target
            'LOCATE /' || target || '/'
        "#;

        run_macro(&mut ed, source, "gamma").unwrap();
        assert_eq!(ed.current_line(), 3);
    }

    #[test]
    fn macro_syntax_error() {
        let mut ed = editor_with_lines(&["test"]);
        let result = run_macro(&mut ed, "if then else what", "");
        assert!(result.is_err());
    }

    #[test]
    fn macro_command_prefix_stripped() {
        let mut ed = editor_with_lines(&["alpha", "beta", "gamma"]);

        run_macro(&mut ed, "'COMMAND DOWN 2'", "").unwrap();
        assert_eq!(ed.current_line(), 3);
    }

    #[test]
    fn macro_multiple_commands() {
        let mut ed = editor_with_lines(&["aaa", "bbb", "ccc", "ddd", "eee"]);

        let source = r#"
            'TOP'
            'DOWN 1'
            'LOCATE /ddd/'
            'DOWN 1'
        "#;

        run_macro(&mut ed, source, "").unwrap();
        assert_eq!(ed.current_line(), 5);
    }
}
