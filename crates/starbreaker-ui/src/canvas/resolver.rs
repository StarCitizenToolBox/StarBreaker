//! Recursive canvas tree resolver.

use std::collections::{HashMap, HashSet};

use log::warn;

use crate::error::UiError;

use super::parser::CanvasParser;
use super::types::{CanvasRecord, ViewComponent};

/// Recursively expands sub-canvas references into a tree of [`CanvasRecord`]s.
pub struct CanvasWidgetTreeResolver {
    max_depth: usize,
}

impl Default for CanvasWidgetTreeResolver {
    fn default() -> Self {
        Self { max_depth: 16 }
    }
}

impl CanvasWidgetTreeResolver {
    /// Create a resolver with default max depth (16).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a resolver with a custom maximum expansion depth.
    pub fn with_max_depth(max_depth: usize) -> Self {
        Self { max_depth }
    }

    /// Resolve a canvas tree starting from `root_guid`.
    pub fn resolve<F, E>(&self, root_guid: &str, fetch: F) -> Result<ResolvedCanvas, UiError>
    where
        F: Fn(&str) -> Result<serde_json::Value, E>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let mut visited = HashSet::new();
        let mut resolved_map: HashMap<String, CanvasRecord> = HashMap::new();

        let root = self.resolve_one(root_guid, &fetch, &mut visited, &mut resolved_map, 0)?;

        Ok(ResolvedCanvas {
            root,
            children: resolved_map,
        })
    }

    fn resolve_one<F, E>(
        &self,
        guid: &str,
        fetch: &F,
        visited: &mut HashSet<String>,
        resolved_map: &mut HashMap<String, CanvasRecord>,
        depth: usize,
    ) -> Result<CanvasRecord, UiError>
    where
        F: Fn(&str) -> Result<serde_json::Value, E>,
        E: std::error::Error + Send + Sync + 'static,
    {
        if depth > self.max_depth {
            return Err(UiError::MaxDepthExceeded {
                guid: guid.to_string(),
                max_depth: self.max_depth,
            });
        }

        if !visited.insert(guid.to_string()) {
            return Err(UiError::CycleDetected(guid.to_string()));
        }

        let json = fetch(guid).map_err(|e| UiError::FetchFailed {
            guid: guid.to_string(),
            source: Box::new(e),
        })?;

        let name = json
            .get("__name")
            .or_else(|| json.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or(guid)
            .to_string();

        let mut record = CanvasParser::parse(guid, &name, &json)?;

        for item in &record.scene {
            if let Some(sub_guid) = &item.guid
                && !sub_guid.is_empty()
                && sub_guid != "null"
                && !resolved_map.contains_key(sub_guid)
            {
                match self.resolve_one(sub_guid, fetch, visited, resolved_map, depth + 1) {
                    Ok(child) => {
                        resolved_map.insert(sub_guid.clone(), child);
                    }
                    Err(UiError::CycleDetected(_)) => {
                        return Err(UiError::CycleDetected(sub_guid.clone()));
                    }
                    Err(UiError::MaxDepthExceeded { .. }) => {
                        return Err(UiError::MaxDepthExceeded {
                            guid: sub_guid.clone(),
                            max_depth: self.max_depth,
                        });
                    }
                    Err(e) => {
                        warn!(
                            "starbreaker-ui: failed to resolve sub-canvas {}: {}",
                            sub_guid, e
                        );
                    }
                }
            }
        }

        for view in &record.views {
            for component in &view.components {
                if let ViewComponent::WidgetCanvas {
                    sub_guid: Some(sg),
                    ..
                } = component
                    && !resolved_map.contains_key(sg)
                {
                    match self.resolve_one(sg, fetch, visited, resolved_map, depth + 1) {
                        Ok(child) => {
                            resolved_map.insert(sg.clone(), child);
                        }
                        Err(UiError::CycleDetected(_)) => {
                            return Err(UiError::CycleDetected(sg.clone()));
                        }
                        Err(UiError::MaxDepthExceeded { .. }) => {
                            return Err(UiError::MaxDepthExceeded {
                                guid: sg.clone(),
                                max_depth: self.max_depth,
                            });
                        }
                        Err(e) => {
                            warn!(
                                "starbreaker-ui: failed to resolve view sub-canvas {}: {}",
                                sg, e
                            );
                        }
                    }
                }
            }
        }

        visited.remove(guid);
        record.name = name;
        Ok(record)
    }
}

/// The output of a successful resolve call.
#[derive(Debug, Clone)]
pub struct ResolvedCanvas {
    pub root: CanvasRecord,
    pub children: HashMap<String, CanvasRecord>,
}
