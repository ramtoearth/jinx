use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::state::*;
use crate::agent::restart_agent;
use jinx::config as app_config;
use jinx::text_editor::TextEditor;

const N_BACKENDS: usize = 5;

pub(crate) fn open_settings_modal(state: &mut RuntimeState) {
    let cfg = app_config::load();
    let backend_idx = match cfg.remote.backend {
        app_config::RemoteBackend::Bedrock => 0,
        app_config::RemoteBackend::Openai => 1,
        app_config::RemoteBackend::Anthropic => 2,
        app_config::RemoteBackend::Gemini => 3,
        app_config::RemoteBackend::Llamaapi => 4,
    };
    state.settings_form = SettingsFormState {
        language_idx: if cfg.language == "es" { 1 } else { 0 },
        provider_idx: if cfg.provider == app_config::Provider::Local { 0 } else { 1 },
        backend_idx,
        local_model_input: TextEditor::from_string(&cfg.local.model),
        host_input: TextEditor::from_string(&cfg.local.host),
        bedrock_model_input: TextEditor::from_string(&cfg.remote.bedrock_model),
        openai_model_input: TextEditor::from_string(&cfg.remote.openai_model),
        anthropic_model_input: TextEditor::from_string(&cfg.remote.anthropic_model),
        gemini_model_input: TextEditor::from_string(&cfg.remote.gemini_model),
        llamaapi_model_input: TextEditor::from_string(&cfg.remote.llamaapi_model),
        panel_sel: state.visible_panels,
        panel_cursor: 0,
        field: 0,
    };
    state.app.modal = Some(jinx::app::Modal::Settings);
}

fn active_remote_model(form: &mut SettingsFormState) -> &mut TextEditor {
    match form.backend_idx {
        0 => &mut form.bedrock_model_input,
        1 => &mut form.openai_model_input,
        2 => &mut form.anthropic_model_input,
        3 => &mut form.gemini_model_input,
        _ => &mut form.llamaapi_model_input,
    }
}

fn settings_is_text_field(field: usize, is_local: bool) -> bool {
    match field {
        2 if is_local => true,
        3 => true,
        _ => false,
    }
}

pub(crate) fn settings_active_editor(state: &mut RuntimeState) -> Option<&mut TextEditor> {
    let is_local = state.settings_form.provider_idx == 0;
    match state.settings_form.field {
        2 if is_local => Some(&mut state.settings_form.local_model_input),
        3 if is_local => Some(&mut state.settings_form.host_input),
        3 => Some(active_remote_model(&mut state.settings_form)),
        _ => None,
    }
}

pub(crate) fn handle_settings_form_key(state: &mut RuntimeState, key: KeyEvent) {
    let is_local = state.settings_form.provider_idx == 0;
    let n_fields: usize = 5;

    // Panel visibility field (field 4): Left/Right moves cursor, Space toggles
    if state.settings_form.field == 4 {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                state.settings_form.panel_cursor = (state.settings_form.panel_cursor + 4) % 5;
                return;
            }
            KeyCode::Right | KeyCode::Char('l') => {
                state.settings_form.panel_cursor = (state.settings_form.panel_cursor + 1) % 5;
                return;
            }
            KeyCode::Char(' ') => {
                let c = state.settings_form.panel_cursor;
                let sel = &state.settings_form.panel_sel;
                // Guard: don't uncheck if it's the last visible
                if sel[c] && sel.iter().filter(|&&v| v).count() <= 1 {
                    return;
                }
                state.settings_form.panel_sel[c] = !state.settings_form.panel_sel[c];
                return;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Tab => {
            state.settings_form.field = (state.settings_form.field + 1) % n_fields;
        }
        KeyCode::BackTab => {
            state.settings_form.field = (state.settings_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Down if !settings_is_text_field(state.settings_form.field, is_local) => {
            state.settings_form.field = (state.settings_form.field + 1) % n_fields;
        }
        KeyCode::Up if !settings_is_text_field(state.settings_form.field, is_local) => {
            state.settings_form.field = (state.settings_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Char('j') if !settings_is_text_field(state.settings_form.field, is_local) => {
            state.settings_form.field = (state.settings_form.field + 1) % n_fields;
        }
        KeyCode::Char('k') if !settings_is_text_field(state.settings_form.field, is_local) => {
            state.settings_form.field = (state.settings_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') if state.settings_form.field == 0 => {
            state.settings_form.language_idx = 1 - state.settings_form.language_idx;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') if state.settings_form.field == 1 => {
            state.settings_form.provider_idx = 1 - state.settings_form.provider_idx;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') if state.settings_form.field == 2 && !is_local => {
            let idx = &mut state.settings_form.backend_idx;
            if matches!(key.code, KeyCode::Right | KeyCode::Char('l')) {
                *idx = (*idx + 1) % N_BACKENDS;
            } else {
                *idx = (*idx + N_BACKENDS - 1) % N_BACKENDS;
            }
        }
        KeyCode::Left if settings_is_text_field(state.settings_form.field, is_local) => {
            if let Some(ed) = settings_active_editor(state) { ed.move_left(); }
        }
        KeyCode::Right if settings_is_text_field(state.settings_form.field, is_local) => {
            if let Some(ed) = settings_active_editor(state) { ed.move_right(); }
        }
        KeyCode::Char(c) if settings_is_text_field(state.settings_form.field, is_local) => {
            if let Some(ed) = settings_active_editor(state) { ed.insert_char(c); }
        }
        KeyCode::Backspace if settings_is_text_field(state.settings_form.field, is_local) => {
            if let Some(ed) = settings_active_editor(state) { ed.backspace(); }
        }
        KeyCode::Delete if settings_is_text_field(state.settings_form.field, is_local) => {
            if let Some(ed) = settings_active_editor(state) { ed.delete(); }
        }
        KeyCode::Enter => save_settings(state),
        KeyCode::Esc => state.app.modal = None,
        _ => {}
    }
}

pub(crate) fn save_settings(state: &mut RuntimeState) {
    let is_local = state.settings_form.provider_idx == 0;
    let defaults = app_config::Config::default();
    let lang = if state.settings_form.language_idx == 1 { "es" } else { "en" };
    let form = &state.settings_form;
    let backend = match form.backend_idx {
        0 => app_config::RemoteBackend::Bedrock,
        1 => app_config::RemoteBackend::Openai,
        2 => app_config::RemoteBackend::Anthropic,
        3 => app_config::RemoteBackend::Gemini,
        _ => app_config::RemoteBackend::Llamaapi,
    };

    let existing_cfg = app_config::load();
    let cfg = app_config::Config {
        language: lang.to_string(),
        provider: if is_local {
            app_config::Provider::Local
        } else {
            app_config::Provider::Remote
        },
        local: app_config::LocalConfig {
            model: {
                let m = form.local_model_input.to_string();
                let m = m.trim();
                if m.is_empty() { defaults.local.model } else { m.to_string() }
            },
            host: {
                let h = form.host_input.to_string();
                let h = h.trim();
                if h.is_empty() { defaults.local.host } else { h.to_string() }
            },
        },
        remote: {
            let d = &defaults.remote;
            let or_default = |input: &TextEditor, fallback: &str| {
                let s = input.to_string();
                let trimmed = s.trim().to_string();
                if trimmed.is_empty() { fallback.to_string() } else { trimmed }
            };
            app_config::RemoteConfig {
                backend,
                bedrock_model: form.bedrock_model_input.to_string().trim().to_string(),
                openai_model: or_default(&form.openai_model_input, &d.openai_model),
                anthropic_model: or_default(&form.anthropic_model_input, &d.anthropic_model),
                gemini_model: or_default(&form.gemini_model_input, &d.gemini_model),
                llamaapi_model: or_default(&form.llamaapi_model_input, &d.llamaapi_model),
            }
        },
        last_export_dir: existing_cfg.last_export_dir,
        visible_panels: state.settings_form.panel_sel,
    };
    if let Err(e) = app_config::save(&cfg) {
        state.app.status_bar = state.locale.errors.config_save.replace("{error}", &e.to_string());
        return;
    }

    state.visible_panels = state.settings_form.panel_sel;
    state.locale = jinx::locale::load(lang);
    state.app.modal = None;
    // If current panel is now hidden, jump to first visible
    if !state.visible_panels[state.app.focused_panel.index()] {
        state.app.focused_panel = state.app.focused_panel.next_visible(&state.visible_panels);
    }
    restart_agent(state);
    state.app.status_bar = state.locale.status.config_saved.clone();
}

pub(crate) fn render_settings_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.settings_form;
    let is_local = form.provider_idx == 0;
    let block = Block::default()
        .title(state.locale.modals.settings.as_str())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let language_label = if form.language_idx == 1 { "← Español →" } else { "← English →" };
    let provider_label = if is_local { "← Local →" } else { "← Remote →" };

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(super::form_line(state.locale.form_labels.language.as_str(), language_label.to_string(), form.field == 0));
    lines.push(super::form_line(state.locale.form_labels.provider.as_str(), provider_label.to_string(), form.field == 1));

    if is_local {
        lines.push(super::form_line_editor(state.locale.form_labels.ollama_model.as_str(), &form.local_model_input, form.field == 2));
        if form.field == 3 || !form.host_input.is_empty() {
            lines.push(super::form_line_editor(state.locale.form_labels.ollama_host.as_str(), &form.host_input, form.field == 3));
        } else {
            lines.push(super::form_line(state.locale.form_labels.ollama_host.as_str(), "http://localhost:11434".to_string(), false));
        }
    } else {
        let backend_names = ["Bedrock", "OpenAI", "Anthropic", "Gemini", "LlamaAPI"];
        let backend_label = format!("← {} →", backend_names[form.backend_idx]);
        lines.push(super::form_line(state.locale.form_labels.backend.as_str(), backend_label, form.field == 2));
        let model_editor = match form.backend_idx {
            0 => &form.bedrock_model_input,
            1 => &form.openai_model_input,
            2 => &form.anthropic_model_input,
            3 => &form.gemini_model_input,
            _ => &form.llamaapi_model_input,
        };
        lines.push(super::form_line_editor(state.locale.form_labels.model.as_str(), model_editor, form.field == 3));
    }

    // Panel visibility row
    let panel_names = ["Chat", "Tareas", "Calendario", "Notas", "Finanzas"];
    let field_active = form.field == 4;
    let mut panel_spans: Vec<Span<'static>> = vec![
        Span::styled(
            format!("  {:16}", "Paneles"),
            if field_active { Style::default().fg(Color::Cyan).add_modifier(ratatui::style::Modifier::BOLD) } else { Style::default().fg(Color::DarkGray) },
        ),
    ];
    for (i, name) in panel_names.iter().enumerate() {
        let checked = if form.panel_sel[i] { "[x]" } else { "[ ]" };
        let is_cursor = field_active && form.panel_cursor == i;
        let style = if is_cursor {
            Style::default().fg(Color::White).add_modifier(ratatui::style::Modifier::BOLD)
        } else if form.panel_sel[i] {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        panel_spans.push(Span::styled(format!("{checked}{name} "), style));
    }
    lines.push(Line::from(panel_spans));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        state.locale.hints.settings_form.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}
