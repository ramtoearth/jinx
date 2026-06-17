mod chat;
mod tareas;
mod calendario;
mod notas;

pub(crate) use chat::*;
pub(crate) use tareas::*;
pub(crate) use calendario::*;
pub(crate) use notas::*;

use ratatui::{
    style::{Color, Style},
    widgets::{Block, Borders},
};
use std::time::Instant;
use uuid::Uuid;

use crate::state::SPINNER_FRAMES;

// ---------------------------------------------------------------------------
// Shared panel utilities
// ---------------------------------------------------------------------------

pub(crate) fn panel_block(title: &str) -> Block<'_> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
}

pub(crate) fn strip_md(s: &str) -> String {
    s.replace("**", "").replace("__", "")
}

pub(crate) fn word_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![];
    }
    let mut result = Vec::new();
    for raw_line in text.split('\n') {
        let trimmed = raw_line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() <= width {
            result.push(trimmed.to_string());
            continue;
        }
        let mut cur = String::new();
        for word in trimmed.split_whitespace() {
            let word_len = word.chars().count();
            if cur.is_empty() {
                cur = word.to_string();
            } else if cur.chars().count() + 1 + word_len <= width {
                cur.push(' ');
                cur.push_str(word);
            } else {
                result.push(cur);
                cur = word.to_string();
            }
        }
        if !cur.is_empty() {
            result.push(cur);
        }
    }
    result
}

pub(crate) fn spinner_state(pending: &Option<(Uuid, Instant)>) -> Option<(char, u64)> {
    pending.as_ref().map(|(_, started)| {
        let elapsed = started.elapsed();
        let frame_idx = (elapsed.as_millis() / 250) as usize % SPINNER_FRAMES.len();
        (SPINNER_FRAMES[frame_idx], elapsed.as_secs())
    })
}
