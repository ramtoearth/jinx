use serde::Deserialize;

const EN_TOML: &str = include_str!("../../locales/en.toml");
const ES_TOML: &str = include_str!("../../locales/es.toml");

pub fn load(lang: &str) -> Locale {
    let raw = match lang {
        "es" => ES_TOML,
        _ => EN_TOML,
    };
    toml::from_str::<Locale>(raw).unwrap_or_else(|e| {
        eprintln!("[locale] parse error for '{lang}': {e} — falling back to en");
        toml::from_str::<Locale>(EN_TOML).expect("embedded en.toml must parse")
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct Locale {
    pub panels: Panels,
    pub modals: Modals,
    pub form_labels: FormLabels,
    pub status: StatusMessages,
    pub errors: ErrorMessages,
    pub hints: Hints,
    pub filters: Filters,
    pub chat: Chat,
    pub misc: Misc,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Panels {
    pub chat: String,
    pub tasks: String,
    pub calendar: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Modals {
    pub new_task: String,
    pub edit_task: String,
    pub new_event: String,
    pub edit_event: String,
    pub new_group: String,
    pub edit_group: String,
    pub filter_tasks: String,
    pub settings: String,
    pub confirm_delete: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FormLabels {
    pub title: String,
    pub priority: String,
    pub deadline: String,
    pub group: String,
    pub status: String,
    pub duration_min: String,
    pub name: String,
    pub color: String,
    pub language: String,
    pub provider: String,
    pub ollama_host: String,
    pub ollama_model: String,
    pub bedrock_model: String,
    pub backend: String,
    pub model: String,
    pub google_calendar: String,
    pub date: String,
    pub datetime: String,
    pub from_date: String,
    pub to_date: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusMessages {
    pub task_saved: String,
    pub event_saved: String,
    pub group_saved: String,
    pub task_deleted: String,
    pub event_deleted: String,
    pub group_deleted: String,
    pub task_completed: String,
    pub task_pending: String,
    pub config_saved: String,
    pub ready: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorMessages {
    pub title_empty: String,
    pub name_empty: String,
    pub start_date_required: String,
    pub duration_integer: String,
    pub color_invalid: String,
    pub empty_message: String,
    pub config_save: String,
    pub agent_send: String,
    pub terminal_too_small: String,
    pub timeout: String,
    pub agent_start: String,
    pub model_required: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Hints {
    pub global: String,
    pub filter_form: String,
    pub task_form: String,
    pub event_form: String,
    pub group_form: String,
    pub settings_form: String,
    pub tasks_nav: String,
    pub tasks_search: String,
    pub tasks_switch_to_groups: String,
    pub groups_nav: String,
    pub groups_switch_to_tasks: String,
    pub calendar_nav: String,
    pub date_input_active: String,
    pub date_input_inactive: String,
    pub no_date: String,
    pub color_hint: String,
    pub delete_prompt: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Filters {
    pub pending: String,
    pub completed: String,
    pub cancelled: String,
    pub all: String,
    pub high: String,
    pub medium: String,
    pub low: String,
    pub today: String,
    pub this_week: String,
    pub this_month: String,
    pub custom: String,
    pub no_group: String,
    pub all_groups: String,
    pub none: String,
    pub filters_prefix: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Chat {
    pub agent: String,
    pub system: String,
    pub you: String,
    pub thinking: String,
    pub thinking_status: String,
    pub older_messages: String,
    pub input_title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Misc {
    pub groups_separator: String,
    pub no_tasks: String,
    pub no_groups: String,
    pub no_calendar_entries: String,
    pub delete_confirm: String,
    pub empty_duration: String,
    pub task_kind: String,
    pub event_kind: String,
    pub group_kind: String,
    pub item_kind: String,
    pub today_marker: String,
    pub custom_color: String,
    pub preset_color: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn en_locale_parses() {
        let locale = toml::from_str::<Locale>(EN_TOML);
        assert!(locale.is_ok(), "en.toml parse error: {:?}", locale.err());
    }

    #[test]
    fn es_locale_parses() {
        let locale = toml::from_str::<Locale>(ES_TOML);
        assert!(locale.is_ok(), "es.toml parse error: {:?}", locale.err());
    }

    #[test]
    fn load_unknown_falls_back_to_en() {
        let locale = load("zz");
        assert_eq!(locale.panels.tasks, "Tasks");
    }
}
