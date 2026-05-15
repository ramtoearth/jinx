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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    /// Ollama model ID. Must support native tool calling (llama3.1, llama3.2, qwen3).
    pub model: String,
    /// URL of the Ollama server.
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    /// Amazon Bedrock model ID, e.g. "anthropic.claude-3-5-sonnet-20241022-v2:0".
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_language")]
    pub language: String,
    pub provider: Provider,
    pub local: LocalConfig,
    pub remote: RemoteConfig,
}

fn default_language() -> String {
    "en".to_string()
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
                model_id: String::new(),
            },
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

# Active provider: "local" (Ollama, no data sent externally) or "remote" (Amazon Bedrock)
provider = "local"

[local]
# Ollama models with tool calling support: llama3.1, llama3.2, qwen3
model = "llama3.2:3b"
# Ollama server URL
host = "http://localhost:11434"

[remote]
# Amazon Bedrock model ID
# Example: "anthropic.claude-3-5-sonnet-20241022-v2:0"
model_id = ""
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
