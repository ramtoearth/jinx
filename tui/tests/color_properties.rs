// Feature: jinx
// Property tests for the colour resolution functions.
// Properties 22 and 23.

use proptest::prelude::*;
use storage::HexColor;
use jinx::color::{ColorMode, nearest_xterm256, resolve_style, XTERM256_PALETTE};

fn arb_hex() -> impl Strategy<Value = HexColor> {
    r"#[0-9a-fA-F]{6}".prop_map(|s| HexColor::new(s).unwrap())
}

fn arb_rgb() -> impl Strategy<Value = (u8, u8, u8)> {
    (any::<u8>(), any::<u8>(), any::<u8>())
}

fn arb_name() -> impl Strategy<Value = String> {
    r"[a-z]{1,20}".prop_map(|s| s)
}

fn parse_hex(hex: &str) -> (u8, u8, u8) {
    let s = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&s[0..2], 16).unwrap();
    let g = u8::from_str_radix(&s[2..4], 16).unwrap();
    let b = u8::from_str_radix(&s[4..6], 16).unwrap();
    (r, g, b)
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 22: Fallback de color y neutro distinguible
// ---------------------------------------------------------------------------
// Valida: Requisitos 16.2, 16.4

proptest! {
    #[test]
    fn p22_xterm256_minimises_euclidean_distance((r, g, b) in arb_rgb()) {
        let chosen_idx = nearest_xterm256(r, g, b);
        let (pr, pg, pb) = XTERM256_PALETTE[chosen_idx as usize];
        let chosen_dist = {
            let dr = r as i32 - pr as i32;
            let dg = g as i32 - pg as i32;
            let db = b as i32 - pb as i32;
            dr * dr + dg * dg + db * db
        };

        // No other palette entry should be closer; on tie the lower index wins
        for (i, &(qr, qg, qb)) in XTERM256_PALETTE.iter().enumerate() {
            let dr = r as i32 - qr as i32;
            let dg = g as i32 - qg as i32;
            let db = b as i32 - qb as i32;
            let dist = dr * dr + dg * dg + db * db;
            if dist < chosen_dist {
                prop_assert!(
                    false,
                    "index {i} (dist {dist}) is closer than chosen index {chosen_idx} (dist {chosen_dist})"
                );
            }
            if dist == chosen_dist {
                prop_assert!(
                    chosen_idx as usize <= i,
                    "tie-breaking by lower index failed: chose {chosen_idx} over {i}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 22: Fallback de color y neutro distinguible
// ---------------------------------------------------------------------------
// Valida: Requisitos 16.2, 16.4

proptest! {
    #[test]
    fn p22_neutral_differs_from_group_colors(
        group_colors in proptest::collection::vec(arb_hex(), 1..10),
    ) {
        let neutral = resolve_style(None, None, ColorMode::TrueColor);
        for hex in &group_colors {
            let group = resolve_style(Some(hex), Some("g"), ColorMode::TrueColor);
            prop_assert_ne!(
                neutral.style.fg, group.style.fg,
                "neutral colour must differ from group colour {}", hex
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 23: Marcador textual en modo monocromo
// ---------------------------------------------------------------------------
// Valida: Requisitos 16.5

proptest! {
    #[test]
    fn p23_monochrome_includes_group_name_prefix(hex in arb_hex(), name in arb_name()) {
        let styled = resolve_style(Some(&hex), Some(&name), ColorMode::Monochrome);
        let expected_prefix = format!("[{name}]");
        prop_assert_eq!(
            styled.prefix.as_deref(),
            Some(expected_prefix.as_str()),
            "monochrome prefix should be [{}]", name
        );
    }
}

proptest! {
    #[test]
    fn p23_no_group_no_prefix_any_mode(
        mode in prop_oneof![
            Just(ColorMode::TrueColor),
            Just(ColorMode::Xterm256),
            Just(ColorMode::Monochrome),
        ],
    ) {
        let styled = resolve_style(None, None, mode);
        prop_assert!(
            styled.prefix.is_none(),
            "entries without a group must never have a text prefix"
        );
    }
}
