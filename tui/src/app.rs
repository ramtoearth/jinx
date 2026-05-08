//! Pure `AppState` and the `reduce(state, event)` function for the TUI.
//!
//! This module is intentionally free of I/O: all logic is pure so the
//! focus machine, viewport guard, and key routing can be tested with
//! property-based tests (Properties 18–21).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use storage::StorageError;

// ---------------------------------------------------------------------------
// Panel identity
// ---------------------------------------------------------------------------

/// The four panels of the TUI layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Chat,
    Tareas,
    Calendario,
    Proximos,
}

/// Fixed focus cycle: Chat → Tareas → Calendario → Proximos → Chat
static CYCLE: [Panel; 4] = [Panel::Chat, Panel::Tareas, Panel::Calendario, Panel::Proximos];

impl Panel {
    fn index(self) -> usize {
        match self {
            Self::Chat => 0,
            Self::Tareas => 1,
            Self::Calendario => 2,
            Self::Proximos => 3,
        }
    }

    fn from_index(i: usize) -> Self {
        CYCLE[i % 4]
    }

    /// Advance focus forward (Tab).
    pub fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    /// Advance focus backward (Shift+Tab).
    pub fn prev(self) -> Self {
        Self::from_index(self.index() + 3) // +3 mod 4 = -1 mod 4
    }
}

// ---------------------------------------------------------------------------
// Modal overlay
// ---------------------------------------------------------------------------

/// Active modal dialogue, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    NewTask,
    EditTask { id: i64 },
    DeleteTask { id: i64 },
    NewEvent,
    EditEvent { id: i64 },
    DeleteEvent { id: i64 },
    NewGroup,
    EditGroup { id: i64 },
    DeleteGroup { id: i64 },
    Export,
    Error { message: String },
    Settings,
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Minimum terminal dimensions required to render all panels (Requisito 14.9).
pub const MIN_COLS: u16 = 60;
pub const MIN_ROWS: u16 = 20;

/// Whether the terminal meets the minimum size requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportState {
    Ok,
    TooSmall,
}

/// Immutable snapshot of the entire UI state. `reduce` returns a new copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    pub focused_panel: Panel,
    pub viewport: (u16, u16),
    pub viewport_state: ViewportState,
    pub status_bar: String,
    pub modal: Option<Modal>,
    /// True when the Agent is running normally.
    pub agent_alive: bool,
    /// Version counter of the Almacén — incremented on every COMMIT.
    pub storage_version: u64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            focused_panel: Panel::Chat,
            viewport: (0, 0),
            viewport_state: ViewportState::TooSmall,
            status_bar: String::new(),
            modal: None,
            agent_alive: false,
            storage_version: 0,
        }
    }
}

impl AppState {
    pub fn new(cols: u16, rows: u16) -> Self {
        let viewport_state = if cols >= MIN_COLS && rows >= MIN_ROWS {
            ViewportState::Ok
        } else {
            ViewportState::TooSmall
        };
        Self {
            viewport: (cols, rows),
            viewport_state,
            ..Default::default()
        }
    }

    /// Returns `true` if the TUI is too small to render all panels.
    pub fn is_too_small(&self) -> bool {
        self.viewport_state == ViewportState::TooSmall
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Events that the main loop feeds into `reduce`.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Terminal resize.
    Resize(u16, u16),
    /// A key press.
    Key(KeyEvent),
    /// The Almacén version counter was bumped.
    StorageChanged(u64),
    /// The Agent process became alive (after spawn or restart).
    AgentStarted,
    /// The Agent process died unexpectedly.
    AgentDied { reason: String },
    /// A storage operation failed.
    StorageError(StorageError),
    /// A status bar message to show.
    StatusMessage(String),
    /// Close the active modal without saving.
    CloseModal,
    /// Tick — used to trigger Panel_Proximos refresh.
    Tick,
}

// ---------------------------------------------------------------------------
// Reduce — pure state transition
// ---------------------------------------------------------------------------

/// Apply an `AppEvent` to the current `AppState` and return the next state.
/// This function contains no I/O; callers handle side-effects.
pub fn reduce(mut state: AppState, event: AppEvent) -> AppState {
    match event {
        // ------------------------------------------------------------------
        // Viewport resize
        // ------------------------------------------------------------------
        AppEvent::Resize(cols, rows) => {
            state.viewport = (cols, rows);
            state.viewport_state = if cols >= MIN_COLS && rows >= MIN_ROWS {
                ViewportState::Ok
            } else {
                ViewportState::TooSmall
            };
        }

        // ------------------------------------------------------------------
        // Key events
        // ------------------------------------------------------------------
        AppEvent::Key(key) => {
            // Close any active modal on Escape
            if key.code == KeyCode::Esc && state.modal.is_some() {
                state.modal = None;
                return state;
            }

            // Global keys always handled (even in TooSmall viewport)
            match key.code {
                KeyCode::Tab if key.modifiers == KeyModifiers::NONE => {
                    if state.modal.is_none() {
                        state.focused_panel = state.focused_panel.next();
                    }
                }
                KeyCode::BackTab => {
                    if state.modal.is_none() {
                        state.focused_panel = state.focused_panel.prev();
                    }
                }
                KeyCode::Char(',') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if state.modal.is_none() {
                        state.modal = Some(Modal::Settings);
                    }
                }
                _ => {
                    // Delegate panel-specific keys — but only if viewport is Ok
                    // and no modal is active (modals consume keys themselves).
                    if state.viewport_state == ViewportState::Ok && state.modal.is_none() {
                        dispatch_panel_key(&mut state, key);
                    }
                }
            }
        }

        // ------------------------------------------------------------------
        // Storage / agent state
        // ------------------------------------------------------------------
        AppEvent::StorageChanged(v) => {
            state.storage_version = v;
        }
        AppEvent::AgentStarted => {
            state.agent_alive = true;
            state.status_bar = "Agente listo.".to_string();
        }
        AppEvent::AgentDied { reason } => {
            state.agent_alive = false;
            state.status_bar = format!("Agente no disponible: {reason}");
        }
        AppEvent::StorageError(e) => {
            state.status_bar = format!("Error: {}", e.message());
        }
        AppEvent::StatusMessage(msg) => {
            state.status_bar = msg;
        }
        AppEvent::CloseModal => {
            state.modal = None;
        }
        AppEvent::Tick => {
            // Proximos panel refresh is driven by storage_version in the renderer
        }
    }
    state
}

// ---------------------------------------------------------------------------
// Per-panel key dispatch
// ---------------------------------------------------------------------------

fn dispatch_panel_key(state: &mut AppState, key: KeyEvent) {
    match state.focused_panel {
        Panel::Chat => dispatch_chat_key(state, key),
        Panel::Tareas => dispatch_tareas_key(state, key),
        Panel::Calendario => dispatch_calendario_key(state, key),
        Panel::Proximos => dispatch_proximos_key(state, key),
    }
}

fn dispatch_chat_key(_state: &mut AppState, _key: KeyEvent) {
    // Chat input is handled directly by the Panel_Chat widget; the app state
    // has nothing extra to track here.
}

fn dispatch_tareas_key(state: &mut AppState, key: KeyEvent) {
    // Opening a blank modal is a pure state transition; editing/deleting needs
    // the cursor position held in RuntimeState and is handled in main.rs.
    if let KeyCode::Char('n') = key.code {
        state.modal = Some(Modal::NewTask);
    }
}

fn dispatch_calendario_key(_state: &mut AppState, _key: KeyEvent) {
    // Handled directly in main.rs handle_calendario_key
}

fn dispatch_proximos_key(_state: &mut AppState, _key: KeyEvent) {
    // Handled directly in main.rs handle_proximos_key
}
