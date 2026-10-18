//! Server configuration.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use smali_analysis::Severity;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub android_jar: Option<PathBuf>,
    pub api_level: Option<u32>,
    pub extra_roots: Vec<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub diagnostics: HashMap<String, String>,
}

/// Configured severity override for a code. `None` inside means "off".
pub enum Override {
    Default,
    Off,
    Severity(Severity),
}

impl Config {
    /// Accepts `{ "smali": {...} }` or the settings object itself.
    pub fn from_value(v: &serde_json::Value) -> Option<Config> {
        let inner = v.get("smali").unwrap_or(v);
        if inner.is_null() {
            return None;
        }
        serde_json::from_value(inner.clone()).ok()
    }

    pub fn severity(&self, code: &str) -> Override {
        match self.diagnostics.get(code).map(String::as_str) {
            Some("off") => Override::Off,
            Some("hint") => Override::Severity(Severity::Hint),
            Some("info") => Override::Severity(Severity::Info),
            Some("warning") => Override::Severity(Severity::Warning),
            Some("error") => Override::Severity(Severity::Error),
            _ => Override::Default,
        }
    }

    pub fn index_inputs(&self) -> (Option<PathBuf>, Option<u32>, Vec<PathBuf>) {
        (
            self.android_jar.clone(),
            self.api_level,
            self.extra_roots.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_and_flat() {
        let v = serde_json::json!({"smali": {"apiLevel": 34, "extraRoots": ["/fw"], "diagnostics": {"S301": "off"}}});
        let c = Config::from_value(&v).unwrap_or_default();
        assert_eq!(c.api_level, Some(34));
        assert!(matches!(c.severity("S301"), Override::Off));
        let flat = serde_json::json!({"androidJar": "/a.jar"});
        assert_eq!(
            Config::from_value(&flat).and_then(|c| c.android_jar),
            Some(PathBuf::from("/a.jar"))
        );
    }
}
