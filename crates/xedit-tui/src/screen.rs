use std::collections::HashMap;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use xedit_core::command::CurLinePosition;
use xedit_core::editor::Editor;

use crate::app::CursorFocus;

/// Resolve a color from the editor's color overrides, falling back to default.
fn resolve_color(editor: &Editor, area: &str, default: Color) -> Color {
    match editor.color_override(area) {
        Some(name) => parse_color_name(name).unwrap_or(default),
        None => default,
    }
}

fn parse_color_name(name: &str) -> Option<Color> {
    match name.to_uppercase().as_str() {
        "BLACK" => Some(Color::Black),
        "RED" => Some(Color::Red),
        "GREEN" => Some(Color::Green),
        "YELLOW" => Some(Color::Yellow),
        "BLUE" => Some(Color::Blue),
        "MAGENTA" => Some(Color::Magenta),
        "CYAN" => Some(Color::Cyan),
        "WHITE" => Some(Color::White),
        "DARKGRAY" | "DARK_GRAY" => Some(Color::DarkGray),
        _ => None,
    }
}

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
const SHADOW_FG: Color = Color::DarkGray;
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

    render_id_line(
        frame,
        chunks[0],
        editor,
        insert_mode,
        resolve_color(editor, "IdLine", ID_LINE_FG),
        resolve_color(editor, "IdLine", ID_LINE_BG),
    );

    let file_area_rect = chunks[1];
    let visible = render_file_area(frame, file_area_rect, editor, prefix_inputs);

    render_message_line(
        frame,
        chunks[2],
        editor,
        in_input_mode,
        resolve_color(editor, "MsgLine", MSG_FG),
    );
    render_command_line(
        frame,
        chunks[3],
        command_text,
        in_input_mode,
        input_text,
        resolve_color(editor, "CmdLine", CMD_PROMPT_FG),
    );

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

fn render_id_line(
    frame: &mut Frame,
    area: Rect,
    editor: &Editor,
    insert_mode: bool,
    fg: Color,
    bg: Color,
) {
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

    let style = Style::default().fg(fg).bg(bg);
    let line = Line::from(vec![Span::styled(
        format!("{:<width$}", id_text, width = area.width as usize),
        style,
    )]);
    frame.render_widget(Paragraph::new(line), area);
}

/// An item in the display list
enum DisplayItem {
    Tof,
    FileLine(usize), // 1-based line number
    Shadow(usize),   // count of hidden lines
    Eof,
}

/// Build the display list, collapsing hidden lines into shadow entries
fn build_display_list(editor: &Editor) -> Vec<DisplayItem> {
    let buf_len = editor.buffer().len();
    let mut items = Vec::with_capacity(buf_len + 2);
    items.push(DisplayItem::Tof);

    if editor.all_filter_active() {
        let mut i = 1;
        while i <= buf_len {
            if editor.is_line_visible(i) {
                items.push(DisplayItem::FileLine(i));
                i += 1;
            } else {
                let shadow = editor.shadow_count_after(i - 1);
                if shadow > 0 && editor.show_shadow() {
                    items.push(DisplayItem::Shadow(shadow));
                }
                i += shadow;
            }
        }
    } else {
        for i in 1..=buf_len {
            items.push(DisplayItem::FileLine(i));
        }
    }

    items.push(DisplayItem::Eof);
    items
}

fn render_file_area(
    frame: &mut Frame,
    area: Rect,
    editor: &Editor,
    prefix_inputs: &HashMap<usize, String>,
) -> VisibleRange {
    let height = area.height as usize;
    let current = editor.current_line();
    let width = area.width as usize;

    let display_list = build_display_list(editor);

    let curline_row = match editor.curline_position() {
        CurLinePosition::Middle => height / 2,
        CurLinePosition::Row(r) => (*r).min(height.saturating_sub(1)),
    };

    // Find which display item corresponds to the current line
    let current_item_idx = display_list
        .iter()
        .position(|item| match item {
            DisplayItem::Tof => current == 0,
            DisplayItem::FileLine(n) => *n == current,
            _ => false,
        })
        .unwrap_or(0);

    let first_item = current_item_idx.saturating_sub(curline_row);

    // The first_display_idx for cursor positioning: we need to map back to line numbers
    let first_display_idx = match display_list.get(first_item) {
        Some(DisplayItem::Tof) => 0,
        Some(DisplayItem::FileLine(n)) => *n,
        _ => 0,
    };

    let _data_fg = resolve_color(editor, "FileArea", DATA_FG);
    let _prefix_fg = resolve_color(editor, "Prefix", PREFIX_FG);
    let _curline_fg = resolve_color(editor, "CurLine", CURRENT_LINE_FG);
    let _curline_bg = resolve_color(editor, "CurLine", CURRENT_LINE_BG);
    let shadow_fg = resolve_color(editor, "Shadow", SHADOW_FG);

    let mut lines: Vec<Line> = Vec::with_capacity(height);

    for row in 0..height {
        // Check for reserved lines (1-based row in file area)
        if let Some(reserved_text) = editor.reserved_line(row + 1) {
            let padded = format!("{:<width$}", reserved_text, width = width);
            lines.push(Line::from(Span::styled(
                padded,
                Style::default().fg(Color::White).bg(Color::Blue),
            )));
            continue;
        }

        let item_idx = first_item + row;
        let item = display_list.get(item_idx);

        let line = match item {
            Some(DisplayItem::Tof) => {
                let is_current = current == 0;
                make_marker_line(TOF_MARKER, is_current, width)
            }
            Some(DisplayItem::FileLine(line_num)) => {
                let is_current = *line_num == current;
                let prefix_text = prefix_inputs.get(line_num);
                if let Some(text) = editor.buffer().line_text(*line_num) {
                    make_data_line(
                        *line_num,
                        text,
                        is_current,
                        editor.show_number(),
                        width,
                        prefix_text,
                    )
                } else {
                    make_empty_row(width)
                }
            }
            Some(DisplayItem::Shadow(count)) => {
                let text = format!("      --- {} line(s) not displayed ---", count);
                let padded = format!("{:<width$}", text, width = width);
                Line::from(Span::styled(padded, Style::default().fg(shadow_fg)))
            }
            Some(DisplayItem::Eof) => make_marker_line(EOF_MARKER, false, width),
            None => make_empty_row(width),
        };

        lines.push(line);
    }

    frame.render_widget(Paragraph::new(lines), area);

    VisibleRange { first_display_idx }
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

fn render_message_line(
    frame: &mut Frame,
    area: Rect,
    editor: &Editor,
    in_input_mode: bool,
    msg_fg: Color,
) {
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
        Style::default().fg(msg_fg)
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
    cmd_fg: Color,
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
            Style::default().fg(cmd_fg).add_modifier(Modifier::BOLD),
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
