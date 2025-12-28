use serde::Deserialize;
use crate::source_selector::MonitorId;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Config {
    #[serde(rename = "screen")]
    pub screens: Vec<ScreenConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ScreenConfig {
    pub monitor: MonitorId,
    pub source: String,
}

impl Config {
    pub fn read() -> color_eyre::Result<Option<Self>> {
        let path = std::path::Path::new("config.toml");
        if !path.exists() {
            return Ok(None);
        }
        let config = std::fs::read_to_string(path)?;
        let config = toml::from_str(&config)?;

        Ok(Some(config))
    }
}
