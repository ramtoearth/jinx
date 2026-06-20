//! Terminal color detection and `resolve_style` (Requisitos 16.1–16.5).
//!
//! The module is I/O-free except for `detect_color_mode`, which reads
//! environment variables and runs `tput colors`.  All mapping functions are
//! pure so they can be property-tested (Properties 22, 23).

use ratatui::style::{Color, Modifier, Style};
use jinx_core::HexColor;

// ---------------------------------------------------------------------------
// Color mode
// ---------------------------------------------------------------------------

/// The color capability of the current terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// 24-bit true color.
    TrueColor,
    /// 256-color xterm palette.
    Xterm256,
    /// No color support.
    Monochrome,
}

/// Detect the color capability of the current terminal.
///
/// Priority:
/// 1. `COLORTERM=truecolor` or `COLORTERM=24bit` → TrueColor
/// 2. `COLORTERM=256color` or `TERM` ends in `256color` → Xterm256
/// 3. `tput colors` returns ≥256 → Xterm256, ≥8 → depends on TERM, else Monochrome
pub fn detect_color_mode() -> ColorMode {
    let colorterm = std::env::var("COLORTERM").unwrap_or_default().to_lowercase();
    if colorterm == "truecolor" || colorterm == "24bit" {
        return ColorMode::TrueColor;
    }

    let term = std::env::var("TERM").unwrap_or_default().to_lowercase();
    if colorterm.contains("256") || term.contains("256color") {
        return ColorMode::Xterm256;
    }

    // Ask tput as a fallback
    if let Ok(out) = std::process::Command::new("tput").arg("colors").output() {
        if let Ok(s) = std::str::from_utf8(&out.stdout) {
            if let Ok(n) = s.trim().parse::<i32>() {
                if n >= 256 {
                    return ColorMode::Xterm256;
                } else if n < 0 || term == "dumb" {
                    return ColorMode::Monochrome;
                }
            }
        }
    }

    if term == "dumb" || term.is_empty() {
        return ColorMode::Monochrome;
    }

    // Default to TrueColor for modern terminals
    ColorMode::TrueColor
}

// ---------------------------------------------------------------------------
// Neutral colour constant
// ---------------------------------------------------------------------------

/// Neutral hex colour used for entries that have no Group assigned.
/// Chosen to be visually distinct from any user-defined Group colour.
pub const NEUTRAL_HEX: &str = "#808080";

fn parse_hex(hex: &str) -> (u8, u8, u8) {
    let s = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(128);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(128);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(128);
    (r, g, b)
}

// ---------------------------------------------------------------------------
// xterm-256 palette mapping
// ---------------------------------------------------------------------------

/// Map an RGB triplet to the nearest xterm-256 colour index by Euclidean
/// distance in RGB space.  On ties the **lower** index wins (deterministic).
pub fn nearest_xterm256(r: u8, g: u8, b: u8) -> u8 {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let mut best_idx = 0u8;
    let mut best_dist = i32::MAX;
    for (i, &(pr, pg, pb)) in XTERM256_PALETTE.iter().enumerate() {
        let dr = r - pr as i32;
        let dg = g - pg as i32;
        let db = b - pb as i32;
        let dist = dr * dr + dg * dg + db * db;
        if dist < best_dist {
            best_dist = dist;
            best_idx = i as u8;
        }
    }
    best_idx
}

// ---------------------------------------------------------------------------
// Styled text representation
// ---------------------------------------------------------------------------

/// Result of `resolve_style` — a Ratatui `Style` plus an optional text prefix
/// that carries the group name in monochrome mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledText {
    pub style: Style,
    /// `Some("[group_name]")` in Monochrome mode, `None` otherwise.
    pub prefix: Option<String>,
}

// ---------------------------------------------------------------------------
// resolve_style — pure mapping (Requisitos 16.1–16.5)
// ---------------------------------------------------------------------------

/// Compute the Ratatui style and optional text prefix for a given Group colour
/// and terminal colour mode.
///
/// - **TrueColor**: use the exact hex colour.
/// - **Xterm256**: map to the nearest palette colour, returning a fallback style
///   that callers can use to emit a warning.
/// - **Monochrome**: return `Style::default()` with `prefix = Some("[name]")`.
pub fn resolve_style(
    group_color: Option<&HexColor>,
    group_name: Option<&str>,
    mode: ColorMode,
) -> StyledText {
    match group_color {
        None => StyledText {
            style: Style::default().fg(Color::DarkGray),
            prefix: None,
        },
        Some(hex) => {
            let (r, g, b) = parse_hex(hex.as_str());
            match mode {
                ColorMode::TrueColor => StyledText {
                    style: Style::default().fg(Color::Rgb(r, g, b)),
                    prefix: None,
                },
                ColorMode::Xterm256 => {
                    let idx = nearest_xterm256(r, g, b);
                    StyledText {
                        style: Style::default().fg(Color::Indexed(idx)),
                        prefix: None,
                    }
                }
                ColorMode::Monochrome => StyledText {
                    style: Style::default().add_modifier(Modifier::BOLD),
                    prefix: group_name.map(|n| format!("[{n}]")),
                },
            }
        }
    }
}


// ---------------------------------------------------------------------------
// xterm-256 colour palette (216 colour cube + 24 grayscale + 16 system)
// ---------------------------------------------------------------------------

pub const XTERM256_PALETTE: [(u8, u8, u8); 256] = {
    // Computed at compile time using the standard xterm-256 mapping.
    // Indices 0-15: system colours (terminal-dependent; we use common defaults)
    // Indices 16-231: 6×6×6 colour cube
    // Indices 232-255: grayscale ramp
    let mut p = [(0u8, 0u8, 0u8); 256];

    // System colours (approximate ANSI defaults)
    p[0] = (0, 0, 0);         // black
    p[1] = (128, 0, 0);       // maroon
    p[2] = (0, 128, 0);       // green
    p[3] = (128, 128, 0);     // olive
    p[4] = (0, 0, 128);       // navy
    p[5] = (128, 0, 128);     // purple
    p[6] = (0, 128, 128);     // teal
    p[7] = (192, 192, 192);   // silver
    p[8] = (128, 128, 128);   // grey
    p[9] = (255, 0, 0);       // red
    p[10] = (0, 255, 0);      // lime
    p[11] = (255, 255, 0);    // yellow
    p[12] = (0, 0, 255);      // blue
    p[13] = (255, 0, 255);    // fuchsia
    p[14] = (0, 255, 255);    // aqua
    p[15] = (255, 255, 255);  // white

    // 6×6×6 colour cube: index = 16 + 36r + 6g + b  (r,g,b ∈ 0..=5)
    let mut i = 16usize;
    let mut r = 0usize;
    while r < 6 {
        let mut g = 0usize;
        while g < 6 {
            let mut b = 0usize;
            while b < 6 {
                let rv = if r == 0 { 0u8 } else { (55 + r * 40) as u8 };
                let gv = if g == 0 { 0u8 } else { (55 + g * 40) as u8 };
                let bv = if b == 0 { 0u8 } else { (55 + b * 40) as u8 };
                p[i] = (rv, gv, bv);
                i += 1;
                b += 1;
            }
            g += 1;
        }
        r += 1;
    }

    // Grayscale ramp: indices 232-255
    let mut j = 0usize;
    while j < 24 {
        let v = (8 + j * 10) as u8;
        p[232 + j] = (v, v, v);
        j += 1;
    }

    p
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use jinx_core::HexColor;

    #[test]
    fn truecolor_uses_exact_rgb() {
        let hex = HexColor::new("#3465A4").unwrap();
        let result = resolve_style(Some(&hex), Some("trabajo"), ColorMode::TrueColor);
        assert_eq!(result.style.fg, Some(Color::Rgb(0x34, 0x65, 0xA4)));
        assert!(result.prefix.is_none());
    }

    #[test]
    fn monochrome_adds_prefix() {
        let hex = HexColor::new("#3465A4").unwrap();
        let result = resolve_style(Some(&hex), Some("trabajo"), ColorMode::Monochrome);
        assert_eq!(result.prefix, Some("[trabajo]".to_string()));
    }

    #[test]
    fn xterm256_nearest_black() {
        let idx = nearest_xterm256(0, 0, 0);
        assert_eq!(idx, 0, "nearest to black should be index 0");
    }

    #[test]
    fn xterm256_nearest_white() {
        let idx = nearest_xterm256(255, 255, 255);
        assert_eq!(idx, 15, "nearest to white should be index 15");
    }
}
