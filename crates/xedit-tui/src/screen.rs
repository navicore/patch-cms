use std::collections::HashMap;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use xedit_core::command::CurLinePosition;
use xedit_core::editor::Editor;

use crate::app::CursorFocus;

const PREFIX_WIDTH: usize = 6; // 5 chars + 1 space
const TOF_MARKER: &str = "* * * Top of File * * *";
const EOF_MARKER: &str = "* * * End of File * * *";

// 3270-inspired color scheme
const ID_LINE_BG: Color = Color::Blue;
const ID_LINE_FG: Color = Color::White;
const CURRENT_LINE_BG: Color = Color::Yellow;
const CURRENT_LINE_FG: Color = Color::Black;
const PREFIX_FG: Color = Color::Cyan;
const PREFIX_EDIT_FG: Color = Color::White;
const DATA_FG: Color = Color::Green;
const MARKER_FG: Color = Color::Blue;
const CMD_PROMPT_FG: Color = Color::Cyan;
const MSG_FG: Color = Color::Yellow;
const INPUT_MODE_FG: Color = Color::Red;

/// Render the complete XEDIT screen
#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    editor: &Editor,
    command_text: &str,
    command_cursor: usize,
    focus: &CursorFocus,
    file_line: usize,
    file_col: usize,
    prefix_inputs: &HashMap<usize, String>,
    in_input_mode: bool,
    input_text: &str,
    insert_mode: bool,
) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(1), // ID line
        Constraint::Min(3),    // file area
        Constraint::Length(1), // message line
        Constraint::Length(1), // command line
    ])
    .split(area);

    render_id_line(frame, chunks[0], editor, insert_mode);

    let file_area_rect = chunks[1];
    let visible = render_file_area(frame, file_area_rect, editor, prefix_inputs);

    render_message_line(frame, chunks[2], editor, in_input_mode);
    render_command_line(frame, chunks[3], command_text, in_input_mode, input_text);

    // Position the cursor based on focus
    position_cursor(
        frame,
        focus,
        file_line,
        file_col,
        command_text,
        command_cursor,
        in_input_mode,
        input_text,
        &visible,
        file_area_rect,
        chunks[3],
    );
}

/// Info about what's visible in the file area, for cursor positioning
struct VisibleRange {
    first_display_idx: usize,
    // Maps display_idx to screen row (relative to file area)
}

fn render_id_line(frame: &mut Frame, area: Rect, editor: &Editor, insert_mode: bool) {
    let filename = if editor.filename().is_empty() {
        "UNNAMED"
    } else {
        editor.filename()
    };
    let filetype = if editor.filetype().is_empty() {
        "UNNAMED"
    } else {
        editor.filetype()
    };

    let mode = if insert_mode { "Ins" } else { "Ovr" };

    let id_text = format!(
        " {:<8} {:<8} {}  Trunc={} Size={} Line={} Col={} Alt={} [{}]",
        filename,
        filetype,
        editor.filemode(),
        editor.trunc(),
        editor.buffer().len(),
        editor.current_line(),
        editor.current_col(),
        editor.alt_count(),
        mode,
    );

    let style = Style::default().fg(ID_LINE_FG).bg(ID_LINE_BG);
    let line = Line::from(vec![Span::styled(
        format!("{:<width$}", id_text, width = area.width as usize),
        style,
    )]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_file_area(
    frame: &mut Frame,
    area: Rect,
    editor: &Editor,
    prefix_inputs: &HashMap<usize, String>,
) -> VisibleRange {
    let height = area.height as usize;
    let buf_len = editor.buffer().len();
    let current = editor.current_line();

    let curline_row = match editor.curline_position() {
        CurLinePosition::Middle => height / 2,
        CurLinePosition::Row(r) => (*r).min(height.saturating_sub(1)),
    };

    // display_idx: 0 = TOF, 1..=buf_len = file lines, buf_len+1 = EOF
    let current_display = if current == 0 { 0 } else { current };
    let first_visible = current_display.saturating_sub(curline_row);

    let mut lines: Vec<Line> = Vec::with_capacity(height);

    for row in 0..height {
        let display_idx = first_visible + row;
        let is_current = display_idx == current_display;

        let line = if display_idx == 0 {
            make_marker_line(TOF_MARKER, is_current, area.width as usize)
        } else if display_idx <= buf_len {
            let line_num = display_idx;
            let prefix_text = prefix_inputs.get(&line_num);
            if let Some(text) = editor.buffer().line_text(line_num) {
                make_data_line(
                    line_num,
                    text,
                    is_current,
                    editor.show_number(),
                    area.width as usize,
                    prefix_text,
                )
            } else {
                make_empty_row(area.width as usize)
            }
        } else if display_idx == buf_len + 1 {
            make_marker_line(EOF_MARKER, false, area.width as usize)
        } else {
            make_empty_row(area.width as usize)
        };

        lines.push(line);
    }

    frame.render_widget(Paragraph::new(lines), area);

    VisibleRange {
        first_display_idx: first_visible,
    }
}

fn make_marker_line(marker: &str, is_current: bool, width: usize) -> Line<'static> {
    let prefix = if is_current { "====>" } else { "=====" };
    let text = format!("{} {}", prefix, marker);
    let padded = format!("{:<width$}", text, width = width);

    if is_current {
        Line::from(Span::styled(
            padded,
            Style::default()
                .fg(CURRENT_LINE_FG)
                .bg(CURRENT_LINE_BG)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(Span::styled(padded, Style::default().fg(MARKER_FG)))
    }
}

fn make_data_line(
    line_num: usize,
    text: &str,
    is_current: bool,
    show_number: bool,
    width: usize,
    prefix_input: Option<&String>,
) -> Line<'static> {
    let data_width = width.saturating_sub(PREFIX_WIDTH);
    let display_text = if text.len() > data_width {
        &text[..data_width]
    } else {
        text
    };

    // Build prefix string
    let prefix_str = if let Some(input) = prefix_input {
        // Show the user's prefix input (padded/truncated to 5 chars)
        format!("{:<5} ", &input[..input.len().min(5)])
    } else if is_current {
        if show_number {
            format!("{:>04}> ", line_num)
        } else {
            "====> ".to_string()
        }
    } else if show_number {
        format!("{:>05} ", line_num)
    } else {
        "      ".to_string()
    };

    let padded_data = format!("{:<dw$}", display_text, dw = data_width);

    if is_current && prefix_input.is_none() {
        // Current line: full highlight
        let full = format!("{}{}", prefix_str, padded_data);
        Line::from(Span::styled(
            full,
            Style::default()
                .fg(CURRENT_LINE_FG)
                .bg(CURRENT_LINE_BG)
                .add_modifier(Modifier::BOLD),
        ))
    } else if prefix_input.is_some() {
        // Line with pending prefix command: highlight the prefix
        Line::from(vec![
            Span::styled(
                prefix_str,
                Style::default()
                    .fg(PREFIX_EDIT_FG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                padded_data,
                if is_current {
                    Style::default()
                        .fg(CURRENT_LINE_FG)
                        .bg(CURRENT_LINE_BG)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(DATA_FG)
                },
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(prefix_str, Style::default().fg(PREFIX_FG)),
            Span::styled(padded_data, Style::default().fg(DATA_FG)),
        ])
    }
}

fn make_empty_row(width: usize) -> Line<'static> {
    Line::from(Span::raw(format!("{:<width$}", "", width = width)))
}

fn render_message_line(frame: &mut Frame, area: Rect, editor: &Editor, in_input_mode: bool) {
    let text = if in_input_mode {
        "INPUT MODE — type text, Enter on empty line to exit, Esc to cancel"
    } else {
        editor
            .message()
            .unwrap_or("Tab=toggle focus | Arrows=navigate | Enter=execute | Esc=cancel")
    };

    let style = if in_input_mode {
        Style::default()
            .fg(INPUT_MODE_FG)
            .add_modifier(Modifier::BOLD)
    } else if editor.message().is_some() {
        Style::default().fg(MSG_FG)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let line = Line::from(Span::styled(
        format!("{:<width$}", text, width = area.width as usize),
        style,
    ));
    frame.render_widget(Paragraph::new(line), area);
}

fn render_command_line(
    frame: &mut Frame,
    area: Rect,
    command_text: &str,
    in_input_mode: bool,
    input_text: &str,
) {
    let (prompt, text) = if in_input_mode {
        ("input>", input_text)
    } else {
        ("====>", command_text)
    };

    let remaining = (area.width as usize).saturating_sub(prompt.len() + 1);
    let display_text = if text.len() > remaining {
        &text[text.len() - remaining..]
    } else {
        text
    };

    let line = Line::from(vec![
        Span::styled(
            format!("{} ", prompt),
            Style::default()
                .fg(CMD_PROMPT_FG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(display_text.to_string(), Style::default().fg(DATA_FG)),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

#[allow(clippy::too_many_arguments)]
fn position_cursor(
    frame: &mut Frame,
    focus: &CursorFocus,
    file_line: usize,
    file_col: usize,
    _command_text: &str,
    command_cursor: usize,
    in_input_mode: bool,
    input_text: &str,
    visible: &VisibleRange,
    file_area: Rect,
    cmd_area: Rect,
) {
    if in_input_mode {
        // Cursor at end of input text in command line area
        let prompt_len = 7; // "input> "
        let x = cmd_area.x + prompt_len + input_text.len() as u16;
        frame.set_cursor_position((x.min(cmd_area.x + cmd_area.width - 1), cmd_area.y));
        return;
    }

    match focus {
        CursorFocus::CommandLine => {
            let prompt_len = 6u16; // "=====> "
            let x = cmd_area.x + prompt_len + command_cursor as u16;
            frame.set_cursor_position((x.min(cmd_area.x + cmd_area.width - 1), cmd_area.y));
        }
        CursorFocus::FileArea => {
            // Find which screen row the file_line maps to
            let display_idx = if file_line == 0 { 0 } else { file_line };
            if display_idx >= visible.first_display_idx {
                let row = display_idx - visible.first_display_idx;
                if row < file_area.height as usize {
                    let screen_y = file_area.y + row as u16;
                    // file_col is 1-based; screen column is 0-based from area.x
                    let screen_x = file_area.x + (file_col as u16).saturating_sub(1);
                    frame.set_cursor_position((
                        screen_x.min(file_area.x + file_area.width - 1),
                        screen_y,
                    ));
                }
            }
        }
    }
}
