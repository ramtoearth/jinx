use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub fn render_markdown(input: &str, width: usize) -> Vec<Line<'static>> {
    let parser = Parser::new(input);
    let mut renderer = MdRenderer::new(width);
    for event in parser {
        renderer.process(event);
    }
    renderer.finish()
}

struct MdRenderer {
    lines: Vec<Line<'static>>,
    current_spans: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    width: usize,
    current_line_len: usize,
    list_depth: usize,
    ordered_index: Option<u64>,
    in_code_block: bool,
    pending_newline: bool,
}

impl MdRenderer {
    fn new(width: usize) -> Self {
        Self {
            lines: Vec::new(),
            current_spans: Vec::new(),
            style_stack: vec![Style::default()],
            width: width.max(10),
            current_line_len: 0,
            list_depth: 0,
            ordered_index: None,
            in_code_block: false,
            pending_newline: false,
        }
    }

    fn current_style(&self) -> Style {
        self.style_stack.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, style: Style) {
        let merged = self.current_style().patch(style);
        self.style_stack.push(merged);
    }

    fn pop_style(&mut self) {
        if self.style_stack.len() > 1 {
            self.style_stack.pop();
        }
    }

    fn flush_line(&mut self) {
        let line = Line::from(std::mem::take(&mut self.current_spans));
        self.lines.push(line);
        self.current_line_len = 0;
    }

    fn append_text(&mut self, text: &str) {
        if self.in_code_block {
            for line in text.split('\n') {
                if self.current_line_len > 0 {
                    self.flush_line();
                }
                let prefixed = format!("  {line}");
                self.current_spans.push(Span::styled(
                    prefixed.clone(),
                    self.current_style(),
                ));
                self.current_line_len = prefixed.len();
            }
            return;
        }

        for word in WordIter::new(text) {
            if word == "\n" {
                self.flush_line();
                continue;
            }

            let word_len = word.chars().count();
            if self.current_line_len > 0
                && self.current_line_len + word_len > self.width
            {
                self.flush_line();
            }

            self.current_spans
                .push(Span::styled(word.to_string(), self.current_style()));
            self.current_line_len += word_len;
        }
    }

    fn process(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.append_text(&text),
            Event::Code(code) => {
                let style = self.current_style().fg(Color::Yellow);
                let formatted = format!("`{code}`");
                let word_len = formatted.chars().count();
                if self.current_line_len > 0
                    && self.current_line_len + word_len > self.width
                {
                    self.flush_line();
                }
                self.current_spans.push(Span::styled(formatted, style));
                self.current_line_len += word_len;
            }
            Event::SoftBreak if self.current_line_len > 0 => {
                self.current_spans
                    .push(Span::raw(" ".to_string()));
                self.current_line_len += 1;
            }
            Event::HardBreak => {
                self.flush_line();
            }
            Event::Rule => {
                self.flush_line();
                let rule = "─".repeat(self.width.min(40));
                self.lines.push(Line::from(Span::styled(
                    rule,
                    Style::default().fg(Color::DarkGray),
                )));
            }
            _ => {}
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                if self.pending_newline || !self.lines.is_empty() {
                    self.flush_line();
                }
                let (color, prefix) = match level {
                    HeadingLevel::H1 => (Color::Cyan, "# "),
                    HeadingLevel::H2 => (Color::Blue, "## "),
                    HeadingLevel::H3 => (Color::Green, "### "),
                    _ => (Color::White, ""),
                };
                let style = Style::default()
                    .fg(color)
                    .add_modifier(Modifier::BOLD);
                self.push_style(style);
                if !prefix.is_empty() {
                    self.current_spans.push(Span::styled(prefix.to_string(), style));
                    self.current_line_len += prefix.len();
                }
            }
            Tag::Emphasis => {
                self.push_style(Style::default().add_modifier(Modifier::ITALIC));
            }
            Tag::Strong => {
                self.push_style(Style::default().add_modifier(Modifier::BOLD));
            }
            Tag::CodeBlock(_) => {
                self.flush_line();
                self.in_code_block = true;
                self.push_style(Style::default().fg(Color::Green));
            }
            Tag::List(start) => {
                if self.current_line_len > 0 {
                    self.flush_line();
                }
                self.ordered_index = start;
                self.list_depth += 1;
            }
            Tag::Item => {
                if self.current_line_len > 0 {
                    self.flush_line();
                }
                let indent = "  ".repeat(self.list_depth.saturating_sub(1));
                let bullet = if let Some(ref mut idx) = self.ordered_index {
                    let b = format!("{indent}{idx}. ");
                    *idx += 1;
                    b
                } else {
                    format!("{indent}• ")
                };
                let blen = bullet.chars().count();
                self.current_spans
                    .push(Span::styled(bullet, self.current_style()));
                self.current_line_len = blen;
            }
            Tag::Paragraph if self.pending_newline => {
                self.flush_line();
            }
            Tag::Link { dest_url, .. } => {
                self.push_style(Style::default().add_modifier(Modifier::UNDERLINED));
                // Store the URL to append after link text
                // We'll handle this in end_tag
                let _ = dest_url;
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.pop_style();
                self.flush_line();
                self.pending_newline = false;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Link => {
                self.pop_style();
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                self.pop_style();
                self.flush_line();
            }
            TagEnd::List(_) => {
                self.list_depth = self.list_depth.saturating_sub(1);
                if self.list_depth == 0 {
                    self.ordered_index = None;
                }
            }
            TagEnd::Item => {
                self.flush_line();
            }
            TagEnd::Paragraph => {
                self.flush_line();
                self.pending_newline = true;
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        if !self.current_spans.is_empty() {
            self.flush_line();
        }
        self.lines
    }
}

struct WordIter<'a> {
    remaining: &'a str,
}

impl<'a> WordIter<'a> {
    fn new(s: &'a str) -> Self {
        Self { remaining: s }
    }
}

impl<'a> Iterator for WordIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }

        if self.remaining.starts_with('\n') {
            self.remaining = &self.remaining[1..];
            return Some("\n");
        }

        // Find next word boundary (space or newline)
        let end = self
            .remaining
            .find('\n')
            .unwrap_or(self.remaining.len());

        let chunk = &self.remaining[..end];
        self.remaining = &self.remaining[end..];

        if chunk.is_empty() {
            return self.next();
        }

        Some(chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_produces_styled_line() {
        let lines = render_markdown("# Hello", 80);
        assert!(!lines.is_empty());
        let first = &lines[0];
        assert!(first.spans.iter().any(|s| s.content.contains("Hello")));
    }

    #[test]
    fn code_block_renders() {
        let input = "```\nfn main() {}\n```";
        let lines = render_markdown(input, 80);
        assert!(lines.iter().any(|l| {
            l.spans.iter().any(|s| s.content.contains("fn main"))
        }));
    }

    #[test]
    fn list_items_have_bullets() {
        let input = "- one\n- two";
        let lines = render_markdown(input, 80);
        assert!(lines.iter().any(|l| {
            l.spans.iter().any(|s| s.content.contains('•'))
        }));
    }
}
