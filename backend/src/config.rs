use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::Value;

const DEFAULT_MODELS: &[&str] = &["gpt-6-sol", "gpt-6-astra", "gpt-5.6-sol", "gpt-6-luna"];

#[derive(Clone, Debug)]
pub(crate) struct BpsConfig {
    pub(crate) enabled: bool,
    pub(crate) models: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    models: Vec<String>,
}

const fn default_enabled() -> bool {
    true
}

impl BpsConfig {
    pub(crate) fn from_value(value: &Value) -> Self {
        let parsed = serde_json::from_value::<RawConfig>(value.clone()).unwrap_or(RawConfig {
            enabled: true,
            models: Vec::new(),
        });
        let models = if parsed.models.is_empty() {
            DEFAULT_MODELS
                .iter()
                .map(|model| (*model).to_owned())
                .collect()
        } else {
            parsed
                .models
                .into_iter()
                .filter(|model| !model.trim().is_empty())
                .collect()
        };
        Self {
            enabled: parsed.enabled,
            models,
        }
    }

    pub(crate) fn accepts(&self, model: Option<&str>) -> bool {
        self.enabled && model.is_some_and(|model| self.models.contains(model))
    }
}
