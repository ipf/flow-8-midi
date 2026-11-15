// Manual page: renders docs/MANUAL.md (embedded at compile time) with light
// markdown formatting — heading sizes, bullets, separators, table rows as
// text. Inline markers (links, bold) are stripped; keep the source standard
// markdown (see AGENTS.md).
use crate::model::{flow8::FLOW8Controller, message::InterfaceMessage};
use iced::{
    widget::{button, column, container, scrollable, text, Space},
    Color, Element, Fill,
};

const MANUAL_MD: &str = include_str!("../../docs/MANUAL.md");

const HEADING_COLOR: Color = Color::WHITE;
const QUOTE_COLOR: Color = Color {
    r: 0.65,
    g: 0.65,
    b: 0.65,
    a: 1.0,
};

pub fn view_manual(_controller: &FLOW8Controller) -> Element<'static, InterfaceMessage> {
    let mut col = column![].spacing(5).width(Fill);

    for line in MANUAL_MD.lines() {
        let line = strip_inline_markers(line);
        let trimmed = line.trim();

        if trimmed.is_empty() {
            col = col.push(Space::new().height(4));
        } else if trimmed == "***" || trimmed == "---" {
            col = col.push(Space::new().height(6));
        } else if let Some(h) = trimmed.strip_prefix("# ") {
            col = col.push(text(h.to_string()).size(20).color(HEADING_COLOR));
        } else if let Some(h) = trimmed.strip_prefix("## ") {
            col = col.push(text(h.to_string()).size(16).color(HEADING_COLOR));
        } else if let Some(h) = trimmed.strip_prefix("### ") {
            col = col.push(text(h.to_string()).size(13).color(HEADING_COLOR));
        } else if let Some(b) = trimmed.strip_prefix("> ") {
            col = col.push(
                container(text(b.to_string()).size(11).color(QUOTE_COLOR))
                    .padding([0, 12]),
            );
        } else if trimmed.starts_with('|') {
            // table: skip separator rows, render cells joined
            if trimmed
                .replace("|", "")
                .trim()
                .chars()
                .all(|c| c == '-' || c == ':' || c == ' ')
            {
                continue;
            }
            let cells: Vec<&str> = trimmed
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();
            col = col.push(text(cells.join("   ·   ")).size(11));
        } else if let Some(b) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            col = col.push(container(text(format!("•  {}", b)).size(12)).padding([0, 12]));
        } else {
            col = col.push(text(line.clone()).size(12));
        }
    }

    let back = button(text("← Back").size(12))
        .on_press(InterfaceMessage::NavigateTo(crate::model::page::Page::About))
        .padding([4, 10])
        .style(button::secondary);

    container(
        scrollable(column![back, Space::new().height(10), col].width(Fill)).height(Fill),
    )
    .padding([0, 10])
    .width(Fill)
    .height(Fill)
    .into()
}

/// Strip `[text](url)` → text and `**bold**` → bold, so the markdown reads
/// as plain text in the viewer.
fn strip_inline_markers(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next(); // drop the second '*'
            }
            '[' => {
                // copy until the closing '](url)'
                let mut label = String::new();
                let mut matched = false;
                for lc in chars.by_ref() {
                    if lc == ']' {
                        matched = true;
                        break;
                    }
                    label.push(lc);
                }
                if matched {
                    // expect '(url)'
                    if chars.peek() == Some(&'(') {
                        chars.next();
                        for uc in chars.by_ref() {
                            if uc == ')' {
                                break;
                            }
                        }
                        out.push_str(&label);
                        continue;
                    }
                    out.push('[');
                    out.push_str(&label);
                    out.push(']');
                } else {
                    out.push('[');
                    out.push_str(&label);
                }
            }
            _ => out.push(c),
        }
    }
    out
}
