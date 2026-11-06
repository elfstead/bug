use crate::Viewport;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Controller-owned data, independent of the page title and runtime view ID.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewMetadata {
    pub name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}
impl ViewMetadata {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self
            .name
            .as_ref()
            .is_some_and(|name| name.trim().is_empty() || name.contains('\0'))
        {
            return Err("view name must be nonblank and contain no NUL".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedView {
    pub url: String,
    pub viewport: Viewport,
    pub zoom: f64,
    #[serde(default)]
    pub metadata: ViewMetadata,
    #[serde(default)]
    pub load_script: String,
}

/// Reopening instructions, not a serialized JS heap or OS window handle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSnapshot {
    pub version: u32,
    pub views: Vec<SavedView>,
}
impl SessionSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported session version (expected 1)".into());
        }
        let mut names = std::collections::HashSet::new();
        for view in &self.views {
            view.viewport.validate()?;
            view.metadata.validate()?;
            if view.url.contains('\0')
                || view.load_script.contains('\0')
                || !view.zoom.is_finite()
                || !(0.25..=5.).contains(&view.zoom)
            {
                return Err("invalid saved URL, script, or zoom".into());
            }
            if let Some(name) = &view.metadata.name
                && !names.insert(name)
            {
                return Err(format!("duplicate view name {name}"));
            }
        }
        Ok(())
    }
}
