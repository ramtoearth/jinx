use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HexColor(String);

static HEX_RE: std::sync::OnceLock<regex_lite::Regex> = std::sync::OnceLock::new();

impl HexColor {
    pub fn new(s: impl Into<String>) -> Result<Self, String> {
        let s = s.into();
        let re = HEX_RE.get_or_init(|| {
            regex_lite::Regex::new(r"^#[0-9A-Fa-f]{6}$").expect("valid hex regex")
        });
        if re.is_match(&s) {
            Ok(Self(s))
        } else {
            Err(format!("invalid hex colour: {s:?} (expected #RRGGBB)"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HexColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
