// Feature: jinx
// Property tests for AppState focus machine, viewport guard, and key routing.
// Properties 18–21.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proptest::prelude::*;
use jinx::app::{AppEvent, AppState, Panel, ViewportState, MIN_COLS, MIN_ROWS, reduce};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn arb_tab_count() -> impl Strategy<Value = usize> {
    0usize..=20
}

fn arb_viewport_too_small() -> impl Strategy<Value = (u16, u16)> {
    (0u16..MIN_COLS, 0u16..MIN_ROWS)
}

fn arb_viewport_ok() -> impl Strategy<Value = (u16, u16)> {
    (MIN_COLS..=300u16, MIN_ROWS..=100u16)
}

fn key_tab() -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
}

fn key_shift_tab() -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE))
}

fn key_char(c: char) -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 18: Exactamente un Panel_Enfocado
// ---------------------------------------------------------------------------
// Valida: Requisitos 14.2

proptest! {
    #[test]
    fn p18_exactly_one_focused_panel(
        tabs in arb_tab_count(),
        (cols, rows) in arb_viewport_ok(),
    ) {
        let mut state = AppState::new(cols, rows);
        for _ in 0..tabs {
            state = reduce(state, key_tab());
        }
        // There is always exactly one focused panel — this is structurally
        // guaranteed by the Panel enum (single value), but we verify the
        // invariant holds after mutations.
        let panel = state.focused_panel;
        let valid = matches!(panel, Panel::Chat | Panel::Tareas | Panel::Calendario);
        prop_assert!(valid, "focused_panel is not a valid Panel variant");
    }
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 19: Ciclo determinista de foco con Tab
// ---------------------------------------------------------------------------
// Valida: Requisitos 14.3

proptest! {
    #[test]
    fn p19_tab_cycles_focus_deterministically(
        k in arb_tab_count(),
        (cols, rows) in arb_viewport_ok(),
    ) {
        let order = [Panel::Chat, Panel::Tareas, Panel::Calendario];
        let mut state = AppState::new(cols, rows);
        // Always start from Chat (default)
        prop_assert_eq!(state.focused_panel, Panel::Chat);

        for _ in 0..k {
            state = reduce(state, key_tab());
        }
        let expected = order[k % 3];
        prop_assert_eq!(
            state.focused_panel, expected,
            "after {} Tab presses, expected {:?}", k, expected
        );
    }
}

// Shift-Tab goes backwards
// ---------------------------------------------------------------------------
// Feature: jinx, Property 19: Ciclo determinista de foco con Tab
// ---------------------------------------------------------------------------
// Valida: Requisitos 14.3

proptest! {
    #[test]
    fn p19_shift_tab_reverses(
        k in 1usize..=20,
        (cols, rows) in arb_viewport_ok(),
    ) {
        let mut state = AppState::new(cols, rows);
        // Advance k steps forward
        for _ in 0..k {
            state = reduce(state.clone(), key_tab());
        }
        let after_forward = state.focused_panel;

        // Reverse one step
        state = reduce(state, key_shift_tab());
        let order = [Panel::Chat, Panel::Tareas, Panel::Calendario];
        let expected = order[(k + 2) % 3]; // k-1 mod 3
        prop_assert_eq!(
            state.focused_panel, expected,
            "shift-tab from {:?} should reach {:?}", after_forward, expected
        );
    }
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 20: Ruteo exclusivo al panel enfocado
// ---------------------------------------------------------------------------
// Valida: Requisitos 14.5
//
// We verify that pressing a panel-specific key ('n' = new item) only opens a
// modal when the expected panel is focused, and has no effect on other panels.

proptest! {
    #[test]
    fn p20_new_task_key_only_in_tareas(
        (cols, rows) in arb_viewport_ok(),
    ) {
        // Focus Tareas — 'n' should open NewTask modal
        let state = AppState::new(cols, rows);
        let state = reduce(state, key_tab()); // Chat → Tareas
        prop_assert_eq!(state.focused_panel, Panel::Tareas);
        let state = reduce(state, key_char('n'));
        prop_assert!(state.modal.is_some(), "'n' in Tareas should open modal");

        // Focus Chat — 'n' should do nothing to the modal
        let state = AppState::new(cols, rows); // reset
        prop_assert_eq!(state.focused_panel, Panel::Chat);
        let state = reduce(state, key_char('n'));
        prop_assert!(state.modal.is_none(), "'n' in Chat should not open modal");
    }
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 21: Bloqueo por viewport insuficiente
// ---------------------------------------------------------------------------
// Valida: Requisitos 14.9

proptest! {
    #[test]
    fn p21_small_viewport_blocks_storage_ops(
        (cols, rows) in arb_viewport_too_small(),
    ) {
        let state = AppState::new(cols, rows);
        prop_assert_eq!(state.viewport_state, ViewportState::TooSmall);

        // Pressing 'n' (new task) must NOT open any modal in TooSmall state
        let after = reduce(state, key_char('n'));
        prop_assert!(
            after.modal.is_none(),
            "storage action must be blocked when viewport is TooSmall"
        );
    }
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 21: Bloqueo por viewport insuficiente
// ---------------------------------------------------------------------------
// Valida: Requisitos 14.9

proptest! {
    #[test]
    fn p21_tab_still_works_in_small_viewport(
        (cols, rows) in arb_viewport_too_small(),
        k in arb_tab_count(),
    ) {
        // Tab (global, non-storage key) should still cycle focus in small viewport
        let mut state = AppState::new(cols, rows);
        for _ in 0..k {
            state = reduce(state, key_tab());
        }
        let order = [Panel::Chat, Panel::Tareas, Panel::Calendario];
        let expected = order[k % 3];
        prop_assert_eq!(state.focused_panel, expected);
    }
}
