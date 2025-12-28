use std::borrow::Cow;
use std::fmt::Formatter;
use std::hash::Hash;
use std::sync::Arc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use winit::monitor::MonitorHandle;
use winit::window::WindowId;
use crate::config::Config;
use crate::ndi::NdiSource;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
pub struct MonitorId {
    name: Cow<'static, str>,
    #[serde(skip)]
    window_id: Option<WindowId>,
}

impl std::fmt::Display for MonitorId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl Hash for MonitorId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

impl PartialEq for MonitorId {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for MonitorId {}

impl From<String> for MonitorId {
    fn from(name: String) -> Self {
        Self {
            name: Cow::from(name),
            window_id: None,
        }
    }
}

impl From<MonitorId> for String {
    fn from(id: MonitorId) -> Self {
        id.name.to_string()
    }
}

impl From<&MonitorHandle> for MonitorId {
    fn from(monitor: &MonitorHandle) -> Self {
        Self {
            name: Cow::from(monitor.name().unwrap_or_default()),
            window_id: None,
        }
    }
}

impl AsRef<str> for MonitorId {
    fn as_ref(&self) -> &str {
        &self.name
    }
}

impl AsRef<WindowId> for MonitorId {
    fn as_ref(&self) -> &WindowId {
        self.window_id.as_ref().expect("MonitorId does not have a window handle")
    }
}

impl PartialEq<MonitorHandle> for MonitorId {
    fn eq(&self, monitor: &MonitorHandle) -> bool {
        self.name == monitor.name().unwrap_or_default()
    }
}

impl MonitorId {
    pub fn has_window_handle(&self) -> bool {
        self.window_id.is_some()
    }

    pub fn with_handle(self, window_id: WindowId) -> Self {
        Self { window_id: Some(window_id), ..self }
    }
}

#[derive(Clone, Default)]
pub struct SourceSelector {
    monitor_sources: Arc<DashMap<MonitorId, String>>,
    sources: Arc<DashMap<String, ndi::Source>>,
}

impl SourceSelector {
    pub fn is_configured(&self, monitor_id: &MonitorId) -> bool {
        self.monitor_sources.contains_key(monitor_id)
    }

    pub fn new(config: Option<Config>) -> Self {
        if let Some(config) = config {
            let monitor_sources = Arc::new(config.screens.into_iter().map(|s| (s.monitor, s.source)).collect());

            Self { monitor_sources, sources: Default::default() }
        }else {
            Self::default()
        }
    }

    pub fn targets(&self) -> Vec<(MonitorId, ndi::Source)> {
        self.monitor_sources
            .iter()
            .filter_map(|entry| self.sources.get(entry.value()).map(|source| (entry.key().clone(), source.clone())))
            .collect()
    }

    pub fn update_monitor(&self, monitor_id: MonitorId) {
        if let Some((_, source)) = self.monitor_sources.remove(&monitor_id) {
            self.monitor_sources.insert(monitor_id, source);
        }
    }

    pub fn update_available_sources(&self, sources: Vec<NdiSource>) {
        self.sources.clear();
        for s in sources {
            self.sources.insert(s.name, s.source);
        }
    }
}
