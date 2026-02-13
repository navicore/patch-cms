use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use xedit_core::command::CurLinePosition;
use xedit_core::editor::Editor;

const PREFIX_WIDTH: usize = 6;
const TOF_MARKER: &str = "* * * Top of File * * *";
const EOF_MARKER: &str = "* * * End of File * * *";

// Classic 3270-inspired color scheme
const ID_LINE_BG: Color = Color::Blue;
const ID_LINE_FG: Color = Color::White;
const CURRENT_LINE_BG: Color = Color::Yellow;
const CURRENT_LINE_FG: Color = Color::Black;
const PREFIX_FG: Color = Color::Cyan;
const DATA_FG: Color = Color::Green;
const MARKER_FG: Color = Color::Blue;
const CMD_PROMPT_FG: Color = Color::Cyan;
const MSG_FG: Color = Color::Yellow;
const INPUT_MODE_FG: Color = Color::Red;

/// Render the complete XEDIT screen
pub fn render(
    frame: &mut Frame,
    editor: &Editor,
    command_text: &str,
    in_input_mode: bool,
    input_text: &str,
) {
    let area = frame.area();

    // Layout: ID line | file area | message line | command line
    let chunks = Layout::vertical([
        Constraint::Length(1),      // ID line
        Constraint::Min(3),         // file area
        Constraint::Length(1),      // message line
        Constraint::Length(1),      // command line
    ])
    .split(area);

    render_id_line(frame, chunks[0], editor);
    render_file_area(frame, chunks[1], editor, in_input_mode);
    render_message_line(frame, chunks[2], editor, in_input_mode);
    render_command_line(frame, chunks[3], command_text, in_input_mode, input_text);
}

fn render_id_line(frame: &mut Frame, area: Rect, editor: &Editor) {
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

    let id_text = format!(
        " {:<8} {:<8} {}  Trunc={} Size={} Line={} Col={} Alt={}",
        filename,
        filetype,
        editor.filemode(),
        editor.trunc(),
        editor.buffer().len(),
        editor.current_line(),
        editor.current_col(),
        editor.alt_count(),
    );

    let style = Style::default().fg(ID_LINE_FG).bg(ID_LINE_BG);
    let line = Line::from(vec![Span::styled(
        format!("{:<width$}", id_text, width = area.width as usize),
        style,
    )]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_file_area(frame: &mut Frame, area: Rect, editor: &Editor, _in_input_mode: bool) {
    let height = area.height as usize;
    let buf_len = editor.buffer().len();
    let current = editor.current_line();

    // Determine which row the current line appears on
    let curline_row = match editor.curline_position() {
        CurLinePosition::Middle => height / 2,
        CurLinePosition::Row(r) => (*r).min(height.saturating_sub(1)),
    };

    // The current line maps to display_line = current + 1 if current > 0,
    // or display_line = 0 if at TOF
    let current_display = if current == 0 { 0 } else { current };

    // First visible display line
    let first_visible = if current_display >= curline_row {
        current_display - curline_row
    } else {
        0
    };

    let mut lines: Vec<Line> = Vec::with_capacity(height);

    for row in 0..height {
        let display_idx = first_visible + row;
        let is_current = display_idx == current_display;

        let line = if display_idx == 0 {
            // Top of File marker
            make_marker_line(TOF_MARKER, is_current, area.width as usize)
        } else if display_idx <= buf_len {
            // Regular file line
            let line_num = display_idx;
            if let Some(text) = editor.buffer().line_text(line_num) {
                make_data_line(
                    line_num,
                    text,
                    is_current,
                    editor.show_number(),
                    area.width as usize,
                )
            } else {
                make_empty_row(area.width as usize)
            }
        } else if display_idx == buf_len + 1 {
            // End of File marker
            make_marker_line(EOF_MARKER, false, area.width as usize)
        } else {
            make_empty_row(area.width as usize)
        };

        lines.push(line);
    }

    frame.render_widget(Paragraph::new(lines), area);
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
        Line::from(Span::styled(
            padded,
            Style::default().fg(MARKER_FG),
        ))
    }
}

fn make_data_line(
    line_num: usize,
    text: &str,
    is_current: bool,
    show_number: bool,
    width: usize,
) -> Line<'static> {
    let prefix = if is_current {
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

    let data_width = width.saturating_sub(PREFIX_WIDTH);
    let display_text = if text.len() > data_width {
        &text[..data_width]
    } else {
        text
    };

    let full = format!(
        "{}{:<dw$}",
        prefix,
        display_text,
        dw = data_width,
    );

    if is_current {
        Line::from(Span::styled(
            full,
            Style::default()
                .fg(CURRENT_LINE_FG)
                .bg(CURRENT_LINE_BG)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(vec![
            Span::styled(prefix, Style::default().fg(PREFIX_FG)),
            Span::styled(
                format!("{:<dw$}", display_text, dw = data_width),
                Style::default().fg(DATA_FG),
            ),
        ])
    }
}

fn make_empty_row(width: usize) -> Line<'static> {
    Line::from(Span::raw(format!("{:<width$}", "", width = width)))
}

fn render_message_line(frame: &mut Frame, area: Rect, editor: &Editor, in_input_mode: bool) {
    let text = if in_input_mode {
        "INPUT MODE — type text, press Enter on empty line to exit"
    } else if let Some(msg) = editor.message() {
        msg
    } else {
        ""
    };

    let style = if in_input_mode {
        Style::default().fg(INPUT_MODE_FG).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MSG_FG)
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
            Style::default().fg(CMD_PROMPT_FG).add_modifier(Modifier::BOLD),
        ),
        Span::styled(display_text.to_string(), Style::default().fg(DATA_FG)),
    ]);

    frame.render_widget(Paragraph::new(line), area);

    // Position cursor at end of text in command/input line
    let cursor_x = area.x + prompt.len() as u16 + 1 + display_text.len() as u16;
    frame.set_cursor_position((cursor_x.min(area.x + area.width - 1), area.y));
}
