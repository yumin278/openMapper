//! Hand-authored signal head positions for a map's crossings, kept beside the map in
//! `openMapper/crossing_signals.json`.
//!
//! Where [`crate::signal_sets`] names the head and pole of a signal, this file says where
//! each one stands: a crossing's `.sco` (its path normalised and lower-cased) maps to a list
//! of points, each a light number of the crossing's own program and a place in the crossing's
//! frame. Nothing here is worked out for the user - every point is one they put down in the
//! crossing editor - so a crossing openMapper did not make can have its signals placed too.
//!
//! The file is written the same way it is read: read, the one key changed, everything else
//! (other crossings, entries this editor does not understand) kept exactly as it was.

use std::collections::BTreeMap;
use std::path::Path;

use glam::DVec3;
use serde::{Deserialize, Serialize};

pub(crate) const FILE_NAME: &str = "crossing_signals.json";

/// The key a crossing's `.sco` path is kept under: backslashes, lower-case.
pub(crate) fn key(rel: &str) -> String {
    rel.replace('/', "\\").to_lowercase()
}

/// One signal the user put down: the light of the crossing's own program it shows, the
/// signal set (empty: the map's default), and where its head stands in the crossing's frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct SignalPoint {
    pub light: usize,
    #[serde(default)]
    pub set: String,
    pub x: f32,
    pub up: f32,
    pub fwd: f32,
    pub heading: f32,
}

impl SignalPoint {
    /// The place in the crossing's frame, as openMapper uses (x, y forward, z up).
    pub fn local(&self) -> DVec3 {
        DVec3::new(self.x as f64, self.fwd as f64, self.up as f64)
    }

    fn validated(self) -> anyhow::Result<Self> {
        for (name, v) in [("x", self.x), ("up", self.up), ("fwd", self.fwd), ("heading", self.heading)] {
            anyhow::ensure!(v.is_finite(), "{name} is not a finite number");
        }
        Ok(self)
    }
}

/// What the file holds, however it was put there.
#[derive(Deserialize)]
struct Raw {
    version: u32,
    #[serde(default)]
    crossings: BTreeMap<String, Vec<serde_json::Value>>,
}

/// The map's crossing signal positions, loaded from `openMapper/crossing_signals.json`.
#[derive(Default)]
pub(crate) struct CrossingSignals {
    crossings: BTreeMap<String, Vec<SignalPoint>>,
}

impl CrossingSignals {
    /// Replace the positions with this map's, including when the file disappears.
    pub(crate) fn reload(&mut self, map_dir: &Path) {
        *self = Self::load(map_dir);
    }

    fn load(map_dir: &Path) -> Self {
        let path = map_dir.join("openMapper").join(FILE_NAME);
        let mut result = Self::default();
        let parsed = std::fs::read(&path)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| serde_json::from_slice::<Raw>(&bytes).map_err(anyhow::Error::from));
        match parsed {
            Err(e) => log::warn!("crossing signals {}: {e}; none loaded", path.display()),
            Ok(raw) if raw.version != 1 => log::warn!(
                "crossing signals {}: unsupported version {} (expected 1); none loaded",
                path.display(),
                raw.version
            ),
            Ok(raw) => {
                for (rel, entries) in raw.crossings {
                    for (index, entry) in entries.into_iter().enumerate() {
                        match serde_json::from_value::<SignalPoint>(entry)
                            .map_err(anyhow::Error::from)
                            .and_then(|p| p.validated())
                        {
                            Ok(point) => result.crossings.entry(key(&rel)).or_default().push(point),
                            Err(e) => log::warn!(
                                "crossing signals {}: skipping {rel:?} items[{index}]: {e}",
                                path.display()
                            ),
                        }
                    }
                }
            }
        }
        log::info!(
            "crossing signals {}: {} crossing(s)",
            path.display(),
            result.crossings.len()
        );
        result
    }

    /// The positions of crossing `rel`, or none when it is not in the file.
    pub(crate) fn points(&self, rel: &str) -> Option<&[SignalPoint]> {
        self.crossings.get(&key(rel)).map(|v| v.as_slice())
    }

    /// Put `rel`'s positions in the cache (after they were written to disk).
    pub(crate) fn set(&mut self, rel: &str, points: Vec<SignalPoint>) {
        self.crossings.insert(key(rel), points);
    }
}

/// Rewrite `openMapper/crossing_signals.json`, replacing only crossing `rel`'s entry with
/// `points`. Every other key - and anything this editor does not read - is kept as it was.
/// A file that cannot be read, or whose version is not 1, is left alone.
pub(crate) fn write_points(map_dir: &Path, rel: &str, points: &[SignalPoint]) -> anyhow::Result<()> {
    let path = map_dir.join("openMapper").join(FILE_NAME);
    let mut root: serde_json::Map<String, serde_json::Value> = if path.exists() {
        let bytes = std::fs::read(&path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        value
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("{}: not a JSON object", path.display()))?
    } else {
        serde_json::Map::new()
    };
    if let Some(v) = root.get("version").and_then(|v| v.as_u64()) {
        anyhow::ensure!(
            v == 1,
            "{}: unsupported version {v} (expected 1); not written",
            path.display()
        );
    }
    let before = serde_json::to_string_pretty(&serde_json::Value::Object(root.clone()))
        .unwrap_or_else(|_| "{unreadable}".into());
    root.insert("version".into(), serde_json::json!(1));
    if !root.contains_key("crossings") {
        root.insert("crossings".into(), serde_json::json!({}));
    }
    let crossings = root
        .get_mut("crossings")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("{}: \"crossings\" is not an object", path.display()))?;
    crossings.insert(key(rel), serde_json::to_value(points)?);
    let after = serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .unwrap_or_else(|_| "{unreadable}".into());
    log::info!("crossing signals {}: before write:\n{before}", path.display());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("{}: {e}", parent.display()))?;
    }
    crate::doc::safe::write_atomic(&path, after.as_bytes())
        .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    log::info!("crossing signals {}: after write:\n{after}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests;
