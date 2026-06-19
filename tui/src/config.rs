//! User configuration loaded from `~/.config/terminal-day-organizer/config.toml`.
//!
//! The file is created automatically on first run with defaults and inline
//! comments so the user knows what to edit without reading documentation.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteBackend {
    Bedrock,
    Openai,
    Anthropic,
    Gemini,
    Llamaapi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    pub model: String,
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    #[serde(default = "default_backend")]
    pub backend: RemoteBackend,
    #[serde(default, alias = "model_id")]
    pub bedrock_model: String,
    #[serde(default)]
    pub openai_model: String,
    #[serde(default)]
    pub anthropic_model: String,
    #[serde(default)]
    pub gemini_model: String,
    #[serde(default)]
    pub llamaapi_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_language")]
    pub language: String,
    pub provider: Provider,
    pub local: LocalConfig,
    pub remote: RemoteConfig,
    #[serde(default)]
    pub last_export_dir: Option<String>,
    #[serde(default = "default_visible_panels")]
    pub visible_panels: [bool; 5],
}

fn default_visible_panels() -> [bool; 5] {
    [true; 5]
}

fn default_language() -> String {
    "en".to_string()
}

fn default_backend() -> RemoteBackend {
    RemoteBackend::Bedrock
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: "en".to_string(),
            provider: Provider::Local,
            local: LocalConfig {
                model: "llama3.2:3b".to_string(),
                host: "http://localhost:11434".to_string(),
            },
            remote: RemoteConfig {
                backend: RemoteBackend::Bedrock,
                bedrock_model: String::new(),
                openai_model: "gpt-4o".to_string(),
                anthropic_model: "claude-sonnet-4-6".to_string(),
                gemini_model: "gemini-2.5-flash-lite".to_string(),
                llamaapi_model: "Llama-4-Maverick-17B-128E-Instruct-FP8".to_string(),
            },
            last_export_dir: None,
            visible_panels: [true; 5],
        }
    }
}

// ---------------------------------------------------------------------------
// File location
// ---------------------------------------------------------------------------

pub fn config_path() -> PathBuf {
    directories::ProjectDirs::from("", "", "jinx")
        .map(|dirs| dirs.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from(".jinx/config.toml"))
}

// ---------------------------------------------------------------------------
// Load / create
// ---------------------------------------------------------------------------

/// Template written on first run. Contains inline comments so users understand
/// each field without consulting external docs.
const DEFAULT_TOML: &str = r#"# jinx — AI model configuration
# Edit this file to change provider or model.
# Location: ~/Library/Application Support/jinx/config.toml (macOS)
#            ~/.config/jinx/config.toml (Linux)

# UI language: "en" (English) or "es" (Spanish)
language = "en"

# Active provider: "local" (Ollama) or "remote" (cloud)
provider = "local"

[local]
# Ollama models with tool calling support: llama3.1, llama3.2, qwen3
model = "llama3.2:3b"
# Ollama server URL
host = "http://localhost:11434"

[remote]
# Active remote backend: bedrock, openai, anthropic, gemini, llamaapi
backend = "bedrock"
# Per-backend model IDs (each backend remembers its own model)
bedrock_model = ""
openai_model = "gpt-4o"
anthropic_model = "claude-sonnet-4-6"
gemini_model = "gemini-2.5-flash-lite"
llamaapi_model = "Llama-4-Maverick-17B-128E-Instruct-FP8"
# API keys are read from environment variables:
#   OPENAI_API_KEY, ANTHROPIC_API_KEY, GOOGLE_API_KEY, LLAMA_API_KEY
#   Amazon Bedrock uses AWS credentials (aws configure)
"#;

/// Persist `cfg` back to `config_path()`, overwriting the file.
pub fn save(cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml_str = toml::to_string_pretty(cfg)?;
    std::fs::write(&path, toml_str)?;
    Ok(())
}

/// Load the config file, creating it with defaults if it does not exist.
/// Parse errors fall back to defaults and are logged to stderr — the app
/// should never crash just because the config file has a typo.
pub fn load() -> Config {
    let path = config_path();

    if !path.exists() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, DEFAULT_TOML) {
            eprintln!("[config] could not create {}: {e}", path.display());
        }
        return Config::default();
    }

    match std::fs::read_to_string(&path) {
        Err(e) => {
            eprintln!("[config] could not read {}: {e} — using defaults", path.display());
            Config::default()
        }
        Ok(contents) => match toml::from_str::<Config>(&contents) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("[config] parse error in {}: {e} — using defaults", path.display());
                Config::default()
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toml_parses() {
        let cfg: Config = toml::from_str(DEFAULT_TOML).expect("DEFAULT_TOML must parse");
        assert_eq!(cfg.provider, Provider::Local);
        assert_eq!(cfg.remote.backend, RemoteBackend::Bedrock);
    }

    #[test]
    fn legacy_config_with_model_id_parses() {
        let legacy = r#"
language = "en"
provider = "remote"

[local]
model = "llama3.2:3b"
host = "http://localhost:11434"

[remote]
model_id = "anthropic.claude-3-5-sonnet-20241022-v2:0"
"#;
        let cfg: Config = toml::from_str(legacy).expect("legacy config must parse");
        assert_eq!(cfg.remote.backend, RemoteBackend::Bedrock);
        assert_eq!(cfg.remote.bedrock_model, "anthropic.claude-3-5-sonnet-20241022-v2:0");
        assert!(cfg.remote.openai_model.is_empty());
    }
}
