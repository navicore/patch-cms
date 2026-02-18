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
const SCALE_FG: Color = Color::Cyan;
const HEX_FG: Color = Color::DarkGray;

// ---------------------------------------------------------------------------
// Layout types
// ---------------------------------------------------------------------------

/// A single screen row in the pre-computed layout.
#[derive(Debug, Clone)]
enum RenderRow {
    Tof {
        is_current: bool,
    },
    DataLine {
        line_num: usize,
        is_current: bool,
    },
    HexHigh {
        line_num: usize,
    },
    HexLow {
        line_num: usize,
    },
    WrapCont {
        line_num: usize,
        is_current: bool,
        chunk_idx: usize,
    },
    Shadow {
        count: usize,
    },
    Eof,
    Scale,
    Reserved {
        text: String,
    },
    Empty,
}

/// Complete screen layout: one `RenderRow` per screen row plus cursor mapping.
struct ScreenLayout {
    rows: Vec<RenderRow>,
    line_to_first_row: HashMap<usize, usize>,
}

// ---------------------------------------------------------------------------
// Pure helper functions
// ---------------------------------------------------------------------------

/// Convert a 4-bit nibble (0–15) to its hex character ('0'–'F').
fn to_hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'A' + nibble - 10) as char,
        _ => '.',
    }
}

/// Build a hex-nibble line (blank prefix + column-aligned nibbles).
///
/// `high` selects the high nibble (true) or low nibble (false).
/// Uses the first byte of each character's UTF-8 encoding for the nibble
/// value, keeping 1:1 column alignment with the text row.
fn make_hex_nibble_line(text: &str, data_width: usize, width: usize, high: bool) -> Line<'static> {
    let prefix = " ".repeat(PREFIX_WIDTH);
    let mut nibbles = String::with_capacity(data_width);
    let mut buf = [0u8; 4];
    for ch in text.chars().take(data_width) {
        ch.encode_utf8(&mut buf);
        let b = buf[0]; // first byte of UTF-8 encoding
        if high {
            nibbles.push(to_hex_char((b >> 4) & 0xF));
        } else {
            nibbles.push(to_hex_char(b & 0xF));
        }
    }
    let padded = format!(
        "{}{:<dw$}",
        prefix,
        nibbles,
        dw = width.saturating_sub(PREFIX_WIDTH)
    );
    Line::from(Span::styled(padded, Style::default().fg(HEX_FG)))
}

/// Split `text` into char-based chunks of at most `chunk_size` characters.
fn char_chunks(text: &str, chunk_size: usize) -> Vec<String> {
    if chunk_size == 0 {
        return vec![text.to_string()];
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars
        .chunks(chunk_size)
        .map(|c| c.iter().collect())
        .collect()
}

/// Build an IBM XEDIT–style column ruler (scale line).
fn make_scale_line(width: usize) -> Line<'static> {
    let data_width = width.saturating_sub(PREFIX_WIDTH);
    let prefix = " ".repeat(PREFIX_WIDTH);
    let mut ruler = String::with_capacity(data_width);

    for col in 1..=data_width {
        if col == 1 {
            ruler.push('|');
        } else if col % 10 == 0 {
            let digit = ((col / 10) % 10) as u32;
            ruler.push(char::from_digit(digit, 10).unwrap_or('.'));
        } else if col % 5 == 0 {
            ruler.push('+');
        } else {
            ruler.push('.');
        }
    }

    let text = format!("{}{}", prefix, ruler);
    Line::from(Span::styled(text, Style::default().fg(SCALE_FG)))
}

/// Render the complete XEDIT screen
#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    editor: &Editor,
    ring_pos: usize,
    ring_total: usize,
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
        ring_pos,
        ring_total,
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

/// Info about what's visible in the file area, for cursor positioning.
struct VisibleRange {
    /// Maps line_num → first screen row (relative to file area).
    line_to_first_row: HashMap<usize, usize>,
}

#[allow(clippy::too_many_arguments)]
fn render_id_line(
    frame: &mut Frame,
    area: Rect,
    editor: &Editor,
    insert_mode: bool,
    ring_pos: usize,
    ring_total: usize,
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

    let ring_info = if ring_total > 1 {
        format!(" Ring {}/{}", ring_pos, ring_total)
    } else {
        String::new()
    };

    let id_text = format!(
        " {:<8} {:<8} {}  Trunc={} Size={} Line={} Col={} Alt={} [{}]{}",
        filename,
        filetype,
        editor.filemode(),
        editor.trunc(),
        editor.buffer().len(),
        editor.current_line(),
        editor.current_col(),
        editor.alt_count(),
        mode,
        ring_info,
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

// ---------------------------------------------------------------------------
// Screen layout builder
// ---------------------------------------------------------------------------

fn build_screen_layout(editor: &Editor, height: usize, width: usize) -> ScreenLayout {
    let current = editor.current_line();
    let data_width = width.saturating_sub(PREFIX_WIDTH);
    let hex_mode = editor.hex();
    // HEX takes priority over WRAP (IBM XEDIT behavior)
    let wrap_mode = editor.wrap() && !hex_mode;
    let scale_mode = editor.show_scale();

    let display_list = build_display_list(editor);

    // Count reserved rows (1-based row within file area)
    let reserved_count = (1..=height)
        .filter(|r| editor.reserved_line(*r).is_some())
        .count();
    let available = height.saturating_sub(reserved_count);

    if available == 0 {
        // All rows reserved — nothing to lay out
        let mut rows = Vec::with_capacity(height);
        for screen_row in 0..height {
            if let Some(text) = editor.reserved_line(screen_row + 1) {
                rows.push(RenderRow::Reserved {
                    text: text.to_string(),
                });
            } else {
                rows.push(RenderRow::Empty);
            }
        }
        return ScreenLayout {
            rows,
            line_to_first_row: HashMap::new(),
        };
    }

    // Compute how many screen rows each display item occupies
    let row_counts: Vec<usize> = display_list
        .iter()
        .map(|item| match item {
            DisplayItem::FileLine(n) => {
                if hex_mode {
                    3
                } else if wrap_mode && data_width > 0 {
                    let text_len = editor
                        .buffer()
                        .line_text(*n)
                        .map(|t| t.chars().count())
                        .unwrap_or(0);
                    if text_len <= data_width {
                        1
                    } else {
                        text_len.div_ceil(data_width)
                    }
                } else {
                    1
                }
            }
            _ => 1,
        })
        .collect();

    // Find which display item corresponds to the current line
    let current_idx = display_list
        .iter()
        .position(|item| match item {
            DisplayItem::Tof => current == 0,
            DisplayItem::FileLine(n) => *n == current,
            _ => false,
        })
        .unwrap_or(0);

    // Desired screen row for the current line (within available rows)
    let curline_row = match editor.curline_position() {
        CurLinePosition::Middle => available / 2,
        CurLinePosition::Row(r) => (*r).min(available.saturating_sub(1)),
    };

    // Scale steals one row above the current line
    let scale_rows = if scale_mode && current > 0 { 1 } else { 0 };
    let target_before = curline_row.saturating_sub(scale_rows);

    // Walk backward from the current item to find the first visible item
    let mut rows_above = 0;
    let mut first_idx = current_idx;
    for i in (0..current_idx).rev() {
        let rc = row_counts[i];
        if rows_above + rc > target_before {
            break;
        }
        rows_above += rc;
        first_idx = i;
    }

    // Expand display items into flat RenderRow entries (one per screen row,
    // excluding reserved lines which are interleaved later).
    let mut content_rows: Vec<RenderRow> = Vec::new();
    let mut idx = first_idx;

    while content_rows.len() < available && idx < display_list.len() {
        // Insert scale line immediately before the current-line item
        if scale_mode && current > 0 && idx == current_idx {
            content_rows.push(RenderRow::Scale);
            if content_rows.len() >= available {
                break;
            }
        }

        match &display_list[idx] {
            DisplayItem::Tof => {
                content_rows.push(RenderRow::Tof {
                    is_current: current == 0,
                });
            }
            DisplayItem::FileLine(n) => {
                let is_current = *n == current;
                let rc = row_counts[idx];

                if hex_mode {
                    content_rows.push(RenderRow::DataLine {
                        line_num: *n,
                        is_current,
                    });
                    if content_rows.len() < available {
                        content_rows.push(RenderRow::HexHigh { line_num: *n });
                    }
                    if content_rows.len() < available {
                        content_rows.push(RenderRow::HexLow { line_num: *n });
                    }
                } else if wrap_mode && rc > 1 {
                    content_rows.push(RenderRow::DataLine {
                        line_num: *n,
                        is_current,
                    });
                    for chunk in 1..rc {
                        if content_rows.len() >= available {
                            break;
                        }
                        content_rows.push(RenderRow::WrapCont {
                            line_num: *n,
                            is_current,
                            chunk_idx: chunk,
                        });
                    }
                } else {
                    content_rows.push(RenderRow::DataLine {
                        line_num: *n,
                        is_current,
                    });
                }
            }
            DisplayItem::Shadow(count) => {
                content_rows.push(RenderRow::Shadow { count: *count });
            }
            DisplayItem::Eof => {
                content_rows.push(RenderRow::Eof);
            }
        }

        idx += 1;
    }

    // Pad to available height
    while content_rows.len() < available {
        content_rows.push(RenderRow::Empty);
    }

    // Interleave reserved lines at their fixed screen-row positions
    let mut final_rows: Vec<RenderRow> = Vec::with_capacity(height);
    let mut content_iter = content_rows.into_iter();

    for screen_row in 0..height {
        if let Some(text) = editor.reserved_line(screen_row + 1) {
            final_rows.push(RenderRow::Reserved {
                text: text.to_string(),
            });
        } else if let Some(row) = content_iter.next() {
            final_rows.push(row);
        } else {
            final_rows.push(RenderRow::Empty);
        }
    }

    // Build line_to_first_row: maps line_num → first screen row for that line
    let mut line_to_first_row = HashMap::new();
    for (screen_row, row) in final_rows.iter().enumerate() {
        match row {
            RenderRow::Tof { .. } => {
                line_to_first_row.entry(0).or_insert(screen_row);
            }
            RenderRow::DataLine { line_num, .. } => {
                line_to_first_row.entry(*line_num).or_insert(screen_row);
            }
            _ => {}
        }
    }

    ScreenLayout {
        rows: final_rows,
        line_to_first_row,
    }
}

// ---------------------------------------------------------------------------
// Paint: render file area using pre-computed layout
// ---------------------------------------------------------------------------

fn render_file_area(
    frame: &mut Frame,
    area: Rect,
    editor: &Editor,
    prefix_inputs: &HashMap<usize, String>,
) -> VisibleRange {
    let height = area.height as usize;
    let width = area.width as usize;
    let layout = build_screen_layout(editor, height, width);

    let shadow_fg = resolve_color(editor, "Shadow", SHADOW_FG);
    let data_width = width.saturating_sub(PREFIX_WIDTH);
    let hex_mode = editor.hex();
    let wrap_mode = editor.wrap() && !hex_mode;

    let mut lines: Vec<Line> = Vec::with_capacity(height);

    for render_row in &layout.rows {
        match render_row {
            RenderRow::Tof { is_current } => {
                lines.push(make_marker_line(TOF_MARKER, *is_current, width));
            }
            RenderRow::DataLine {
                line_num,
                is_current,
            } => {
                let prefix_text = prefix_inputs.get(line_num);
                if let Some(text) = editor.buffer().line_text(*line_num) {
                    // In wrap mode, only show the first chunk of chars
                    let display =
                        if wrap_mode && data_width > 0 && text.chars().count() > data_width {
                            let first_chunk: String = text.chars().take(data_width).collect();
                            make_data_line(
                                *line_num,
                                &first_chunk,
                                *is_current,
                                editor.show_number(),
                                width,
                                prefix_text,
                            )
                        } else {
                            make_data_line(
                                *line_num,
                                text,
                                *is_current,
                                editor.show_number(),
                                width,
                                prefix_text,
                            )
                        };
                    lines.push(display);
                } else {
                    lines.push(make_empty_row(width));
                }
            }
            RenderRow::HexHigh { line_num } => {
                let text = editor.buffer().line_text(*line_num).unwrap_or("");
                lines.push(make_hex_nibble_line(text, data_width, width, true));
            }
            RenderRow::HexLow { line_num } => {
                let text = editor.buffer().line_text(*line_num).unwrap_or("");
                lines.push(make_hex_nibble_line(text, data_width, width, false));
            }
            RenderRow::WrapCont {
                line_num,
                is_current,
                chunk_idx,
            } => {
                debug_assert!(
                    data_width > 0,
                    "WrapCont should not exist when data_width == 0"
                );
                let text = editor.buffer().line_text(*line_num).unwrap_or("");
                let chunks = char_chunks(text, data_width);
                let chunk_text = chunks.get(*chunk_idx).map(|s| s.as_str()).unwrap_or("");
                let blank_prefix = " ".repeat(PREFIX_WIDTH);
                let padded_data = format!("{:<dw$}", chunk_text, dw = data_width);

                if *is_current {
                    let full = format!("{}{}", blank_prefix, padded_data);
                    lines.push(Line::from(Span::styled(
                        full,
                        Style::default()
                            .fg(CURRENT_LINE_FG)
                            .bg(CURRENT_LINE_BG)
                            .add_modifier(Modifier::BOLD),
                    )));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled(blank_prefix, Style::default().fg(PREFIX_FG)),
                        Span::styled(padded_data, Style::default().fg(DATA_FG)),
                    ]));
                }
            }
            RenderRow::Shadow { count } => {
                let text = format!("      --- {} line(s) not displayed ---", count);
                let padded = format!("{:<width$}", text, width = width);
                lines.push(Line::from(Span::styled(
                    padded,
                    Style::default().fg(shadow_fg),
                )));
            }
            RenderRow::Eof => {
                lines.push(make_marker_line(EOF_MARKER, false, width));
            }
            RenderRow::Scale => {
                lines.push(make_scale_line(width));
            }
            RenderRow::Reserved { text } => {
                let padded = format!("{:<width$}", text, width = width);
                lines.push(Line::from(Span::styled(
                    padded,
                    Style::default().fg(Color::White).bg(Color::Blue),
                )));
            }
            RenderRow::Empty => {
                lines.push(make_empty_row(width));
            }
        }
    }

    frame.render_widget(Paragraph::new(lines), area);

    VisibleRange {
        line_to_first_row: layout.line_to_first_row,
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
    // Truncate by char count, not byte count, to avoid panicking on multi-byte UTF-8
    let display_text: String = if text.chars().count() > data_width {
        text.chars().take(data_width).collect()
    } else {
        text.to_string()
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
            // Look up the screen row for the current line from the layout map
            if let Some(&row) = visible.line_to_first_row.get(&file_line) {
                if row < file_area.height as usize {
                    let screen_y = file_area.y + row as u16;
                    // file_col is 1-based within the data area; add PREFIX_WIDTH
                    let screen_x =
                        file_area.x + PREFIX_WIDTH as u16 + (file_col as u16).saturating_sub(1);
                    frame.set_cursor_position((
                        screen_x.min(file_area.x + file_area.width - 1),
                        screen_y,
                    ));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use xedit_core::command::{Command, SetCommand};

    // --- to_hex_char ---

    #[test]
    fn hex_char_digits() {
        for n in 0..=9u8 {
            assert_eq!(to_hex_char(n), (b'0' + n) as char);
        }
    }

    #[test]
    fn hex_char_letters() {
        assert_eq!(to_hex_char(10), 'A');
        assert_eq!(to_hex_char(11), 'B');
        assert_eq!(to_hex_char(12), 'C');
        assert_eq!(to_hex_char(13), 'D');
        assert_eq!(to_hex_char(14), 'E');
        assert_eq!(to_hex_char(15), 'F');
    }

    #[test]
    fn hex_char_out_of_range() {
        assert_eq!(to_hex_char(16), '.');
        assert_eq!(to_hex_char(255), '.');
    }

    // --- char_chunks ---

    #[test]
    fn chunks_basic() {
        assert_eq!(char_chunks("hello", 3), vec!["hel", "lo"]);
    }

    #[test]
    fn chunks_empty() {
        assert_eq!(char_chunks("", 5), vec![""]);
    }

    #[test]
    fn chunks_no_wrap() {
        assert_eq!(char_chunks("hi", 10), vec!["hi"]);
    }

    #[test]
    fn chunks_exact_fit() {
        assert_eq!(char_chunks("abcdef", 3), vec!["abc", "def"]);
    }

    #[test]
    fn chunks_zero_size() {
        assert_eq!(char_chunks("abc", 0), vec!["abc"]);
    }

    #[test]
    fn chunks_multibyte() {
        // Each char is one chunk unit regardless of byte width
        assert_eq!(char_chunks("aöü", 2), vec!["aö", "ü"]);
    }

    // --- make_scale_line ---

    #[test]
    fn scale_line_80() {
        let line = make_scale_line(80);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        // PREFIX_WIDTH = 6 blank chars, then ruler starts
        let ruler = &text[PREFIX_WIDTH..];
        assert_eq!(ruler.chars().nth(0), Some('|'), "col 1 = |");
        assert_eq!(ruler.chars().nth(4), Some('+'), "col 5 = +");
        assert_eq!(ruler.chars().nth(9), Some('1'), "col 10 = 1");
        assert_eq!(ruler.chars().nth(14), Some('+'), "col 15 = +");
        assert_eq!(ruler.chars().nth(19), Some('2'), "col 20 = 2");
    }

    #[test]
    fn scale_line_20() {
        let line = make_scale_line(20);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let ruler = &text[PREFIX_WIDTH..];
        // data_width = 14, so ruler has 14 columns
        assert_eq!(ruler.chars().count(), 14);
        assert_eq!(ruler.chars().nth(0), Some('|'));
        assert_eq!(ruler.chars().nth(9), Some('1'));
    }

    #[test]
    fn scale_line_narrow() {
        // Width <= PREFIX_WIDTH means data_width = 0, ruler is empty
        let line = make_scale_line(6);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text.chars().count(), 6);
    }

    // --- hex line rendering ---

    #[test]
    fn hex_nibbles_hello() {
        // 'H' = 0x48, 'e' = 0x65, 'l' = 0x6C, 'l' = 0x6C, 'o' = 0x6F
        // Uses first byte of UTF-8 encoding (same as codepoint for ASCII)
        let expected_high = "46666";
        let expected_low = "85CCF";
        let text = "Hello";
        let mut high = String::new();
        let mut low = String::new();
        let mut buf = [0u8; 4];
        for ch in text.chars() {
            ch.encode_utf8(&mut buf);
            let b = buf[0];
            high.push(to_hex_char((b >> 4) & 0xF));
            low.push(to_hex_char(b & 0xF));
        }
        assert_eq!(high, expected_high);
        assert_eq!(low, expected_low);
    }

    #[test]
    fn hex_nibbles_multibyte() {
        // '€' = U+20AC, UTF-8: E2 82 AC — first byte is 0xE2
        let mut buf = [0u8; 4];
        '€'.encode_utf8(&mut buf);
        let b = buf[0]; // 0xE2
        assert_eq!(to_hex_char((b >> 4) & 0xF), 'E');
        assert_eq!(to_hex_char(b & 0xF), '2');
    }

    // --- Layout tests using Editor ---

    fn make_test_editor(lines: &[&str]) -> Editor {
        let mut ed = Editor::new();
        for line in lines {
            ed.input_line(line);
        }
        // Reset to line 1
        ed.execute(&Command::Top).unwrap();
        ed.execute(&Command::Down(1)).unwrap();
        ed
    }

    #[test]
    fn layout_normal_line_to_row_mapping() {
        let ed = make_test_editor(&["a", "b", "c"]);
        let layout = build_screen_layout(&ed, 10, 80);
        // In normal mode, each line takes 1 row
        let row1 = layout
            .line_to_first_row
            .get(&1)
            .copied()
            .expect("line 1 should be visible in layout");
        let row2 = layout
            .line_to_first_row
            .get(&2)
            .copied()
            .expect("line 2 should be visible in layout");
        // Rows should be sequential (offset by centering)
        assert_eq!(row2, row1 + 1);
    }

    #[test]
    fn layout_hex_mode_3x_spacing() {
        let mut ed = make_test_editor(&["a", "b"]);
        ed.execute(&Command::Set(SetCommand::Hex(true))).unwrap();
        let layout = build_screen_layout(&ed, 20, 80);
        // In hex mode, each line takes 3 rows
        let r1 = layout
            .line_to_first_row
            .get(&1)
            .copied()
            .expect("line 1 should be visible");
        let r2 = layout
            .line_to_first_row
            .get(&2)
            .copied()
            .expect("line 2 should be visible");
        assert_eq!(r2 - r1, 3, "hex lines should be 3 rows apart");
    }

    #[test]
    fn layout_wrap_mode_variable_spacing() {
        let mut ed = make_test_editor(&[
            "short",
            &"x".repeat(200), // long line that wraps
            "end",
        ]);
        ed.execute(&Command::Set(SetCommand::Wrap(true))).unwrap();

        let layout = build_screen_layout(&ed, 40, 80);

        // data_width = 80 - 6 = 74
        // "short" fits in 1 row, "x"*200 needs ceil(200/74) = 3 rows
        let r1 = layout
            .line_to_first_row
            .get(&1)
            .copied()
            .expect("line 1 visible");
        let r2 = layout
            .line_to_first_row
            .get(&2)
            .copied()
            .expect("line 2 visible");
        let r3 = layout
            .line_to_first_row
            .get(&3)
            .copied()
            .expect("line 3 visible");
        // line 1 ("short") takes 1 row
        assert_eq!(r2 - r1, 1);
        // line 2 ("x"*200) takes 3 rows (200/74 = 2.7 → 3)
        assert_eq!(r3 - r2, 3);
    }

    #[test]
    fn layout_scale_adds_row() {
        let mut ed = make_test_editor(&["a", "b", "c"]);
        let layout_no_scale = build_screen_layout(&ed, 10, 80);

        ed.execute(&Command::Set(SetCommand::Scale(true))).unwrap();
        let layout_with_scale = build_screen_layout(&ed, 10, 80);

        // With scale on, current line should be shifted down by 1
        let row_no_scale = layout_no_scale
            .line_to_first_row
            .get(&1)
            .copied()
            .expect("line 1 should be visible in layout");
        let row_with_scale = layout_with_scale
            .line_to_first_row
            .get(&1)
            .copied()
            .expect("line 1 should be visible with scale");
        assert_eq!(
            row_with_scale,
            row_no_scale + 1,
            "scale line shifts current line down by 1"
        );
    }

    #[test]
    fn layout_scale_has_scale_row_before_current() {
        let mut ed = make_test_editor(&["a", "b", "c"]);
        ed.execute(&Command::Set(SetCommand::Scale(true))).unwrap();
        let layout = build_screen_layout(&ed, 10, 80);

        // The row before the current line should be a Scale
        let cur_row = layout
            .line_to_first_row
            .get(&1)
            .copied()
            .expect("line 1 should be visible in layout");
        assert!(
            cur_row > 0,
            "current line should not be at row 0 with scale"
        );
        assert!(
            matches!(layout.rows[cur_row - 1], RenderRow::Scale),
            "row before current line should be Scale"
        );
    }

    #[test]
    fn layout_scale_not_at_tof() {
        let mut ed = make_test_editor(&["a"]);
        // Move to TOF
        ed.execute(&Command::Top).unwrap();
        ed.execute(&Command::Set(SetCommand::Scale(true))).unwrap();
        let layout = build_screen_layout(&ed, 10, 80);

        // No Scale row should appear when current line is TOF (0)
        assert!(
            !layout.rows.iter().any(|r| matches!(r, RenderRow::Scale)),
            "no scale when at TOF"
        );
    }

    #[test]
    fn layout_scale_with_curline_row_zero() {
        let mut ed = make_test_editor(&["a", "b", "c"]);
        ed.execute(&Command::Set(SetCommand::Scale(true))).unwrap();
        ed.execute(&Command::Set(SetCommand::CurLine(
            xedit_core::command::CurLinePosition::Row(0),
        )))
        .unwrap();
        let layout = build_screen_layout(&ed, 10, 80);

        // Scale should appear at row 0, pushing current line to row 1
        assert!(
            matches!(layout.rows[0], RenderRow::Scale),
            "scale at row 0 when curline is row 0"
        );
        let cur_row = layout
            .line_to_first_row
            .get(&1)
            .copied()
            .expect("line 1 should be visible");
        assert_eq!(cur_row, 1, "current line at row 1 (after scale)");
    }

    #[test]
    fn layout_hex_wins_over_wrap() {
        let mut ed = make_test_editor(&[&"x".repeat(200)]);
        // Enable both — hex should win
        ed.execute(&Command::Set(SetCommand::Hex(true))).unwrap();
        ed.execute(&Command::Set(SetCommand::Wrap(true))).unwrap();

        let layout = build_screen_layout(&ed, 20, 80);

        // Should have HexHigh/HexLow rows, not WrapCont
        assert!(
            layout
                .rows
                .iter()
                .any(|r| matches!(r, RenderRow::HexHigh { .. })),
            "hex rows present"
        );
        assert!(
            !layout
                .rows
                .iter()
                .any(|r| matches!(r, RenderRow::WrapCont { .. })),
            "no wrap rows when hex is on"
        );
    }

    #[test]
    fn layout_wrap_short_text_no_continuation() {
        let mut ed = make_test_editor(&["short"]);
        ed.execute(&Command::Set(SetCommand::Wrap(true))).unwrap();
        let layout = build_screen_layout(&ed, 10, 80);

        // "short" fits in data_width, so no WrapCont rows
        assert!(
            !layout
                .rows
                .iter()
                .any(|r| matches!(r, RenderRow::WrapCont { .. })),
            "no wrap continuation for short text"
        );
    }

    #[test]
    fn layout_wrap_exact_fit_no_continuation() {
        // data_width = 80 - 6 = 74, text exactly 74 chars
        let mut ed = make_test_editor(&[&"a".repeat(74)]);
        ed.execute(&Command::Set(SetCommand::Wrap(true))).unwrap();
        let layout = build_screen_layout(&ed, 10, 80);

        assert!(
            !layout
                .rows
                .iter()
                .any(|r| matches!(r, RenderRow::WrapCont { .. })),
            "no wrap continuation when text exactly fits"
        );
    }

    #[test]
    fn layout_empty_buffer() {
        let ed = Editor::new();
        let layout = build_screen_layout(&ed, 10, 80);
        // Should have TOF and EOF, no crashes
        assert!(layout
            .rows
            .iter()
            .any(|r| matches!(r, RenderRow::Tof { .. })));
        assert!(layout.rows.iter().any(|r| matches!(r, RenderRow::Eof)));
    }

    #[test]
    fn layout_scale_plus_hex() {
        let mut ed = make_test_editor(&["Hello"]);
        ed.execute(&Command::Set(SetCommand::Hex(true))).unwrap();
        ed.execute(&Command::Set(SetCommand::Scale(true))).unwrap();
        let layout = build_screen_layout(&ed, 20, 80);

        // Should have Scale before current line, then DataLine + HexHigh + HexLow
        let cur_row = layout
            .line_to_first_row
            .get(&1)
            .copied()
            .expect("line 1 should be visible in layout");
        assert!(cur_row > 0);
        assert!(matches!(layout.rows[cur_row - 1], RenderRow::Scale));
        assert!(matches!(
            layout.rows[cur_row],
            RenderRow::DataLine { line_num: 1, .. }
        ));
        assert!(matches!(
            layout.rows[cur_row + 1],
            RenderRow::HexHigh { line_num: 1 }
        ));
        assert!(matches!(
            layout.rows[cur_row + 2],
            RenderRow::HexLow { line_num: 1 }
        ));
    }
}
