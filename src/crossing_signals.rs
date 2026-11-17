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
//! [`prepare_points`] builds that file without touching the disk, so the crossing editor can
//! put it in the same transaction as the object's `.sco`; [`write_points`] is the plain
//! one-file writer tests and other callers use.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    /// Required: a point says which signal set it uses (empty: the map's default). A missing
    /// field is an invalid entry, and the warning names the missing field.
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
    crossings: BTreeMap<String, serde_json::Value>,
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

    /// Read this map's positions without changing `self` (the editor opening a map fresh).
    pub(crate) fn load(map_dir: &Path) -> Self {
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
                // (the raw keys come from a BTreeMap: the first spelling of a crossing in
                // sorted order is the one kept, so the same file always reads the same way)
                let mut claimed: BTreeMap<String, String> = BTreeMap::new();
                for (rel, entries) in raw.crossings {
                    let Some(entries) = entries.as_array() else {
                        log::warn!(
                            "crossing signals {}: skipping {rel:?}: not an array of positions",
                            path.display()
                        );
                        continue;
                    };
                    let normalised = key(&rel);
                    if let Some(kept) = claimed.get(&normalised) {
                        log::warn!(
                            "crossing signals {}: {rel:?} and {kept:?} both normalise to {normalised:?}; keeping {kept:?}, ignoring {} item(s) of {rel:?}",
                            path.display(),
                            entries.len()
                        );
                        continue;
                    }
                    claimed.insert(normalised.clone(), rel.clone());
                    let mut points = Vec::new();
                    for (index, entry) in entries.iter().enumerate() {
                        match serde_json::from_value::<SignalPoint>(entry.clone())
                            .map_err(anyhow::Error::from)
                            .and_then(|p| p.validated())
                        {
                            Ok(point) => points.push(point),
                            Err(e) => log::warn!(
                                "crossing signals {}: skipping {rel:?} items[{index}]: {e}",
                                path.display()
                            ),
                        }
                    }
                    // (a valid array keeps its key even when it is empty or nothing in it was usable)
                    result.crossings.insert(normalised, points);
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

/// The file [`prepare_points`] would write, made without touching the disk: the crossing
/// editor puts [`PreparedPoints::path`] and [`PreparedPoints::bytes`] in the same
/// `doc::safe` transaction as the object's `.sco`, then calls [`PreparedPoints::log_after`]
/// once that transaction has succeeded.
pub(crate) struct PreparedPoints {
    /// The map's `openMapper/crossing_signals.json`.
    pub path: PathBuf,
    /// The whole file as it would stand after the write.
    pub bytes: Vec<u8>,
    /// The file as it stands now (none: there is no file yet).
    before: Option<String>,
    /// The file as it would stand, kept as text for the log.
    after: String,
    /// The normalised key that was written.
    rel: String,
    /// Other spellings of the same crossing that the write replaces in the file.
    merged: Vec<String>,
}

impl PreparedPoints {
    /// The file as it stands, in full (or that there is none).
    pub fn log_before(&self) {
        match &self.before {
            Some(before) => log::info!("crossing signals {}: before write:\n{before}", self.path.display()),
            None => log::info!("crossing signals {}: before write: (no file)", self.path.display()),
        }
    }

    /// The file as it would stand, in full. Call this only once the write (or the
    /// transaction it is part of) has actually succeeded.
    pub fn log_after(&self) {
        if self.merged.is_empty() {
            log::info!(
                "crossing signals {}: after write ({}):\n{}",
                self.path.display(),
                self.rel,
                self.after
            );
        } else {
            log::info!(
                "crossing signals {}: after write ({}; replaced {:?}):\n{}",
                self.path.display(),
                self.rel,
                self.merged,
                self.after
            );
        }
    }
}

/// Build the whole file with only crossing `rel`'s entry replaced by `points`, without
/// writing anything. Every other key - and anything this editor does not read - is kept as
/// it was, as are the JSON values of the other crossings. Other spellings of `rel` (another
/// case, forward slashes) are removed, so the file holds one entry for the crossing.
///
/// An existing file must be a JSON object with an integer `version` of 1 and, when present,
/// a `crossings` object; anything else is refused, not rewritten. A missing file becomes a
/// new version 1 file. Every point must have finite numbers; the points are not changed.
pub(crate) fn prepare_points(map_dir: &Path, rel: &str, points: &[SignalPoint]) -> anyhow::Result<PreparedPoints> {
    let target = key(rel);
    // (nothing is prepared until every point can be written: the file holds other crossings)
    for (index, point) in points.iter().enumerate() {
        point
            .clone()
            .validated()
            .map_err(|e| anyhow::anyhow!("{target} point {index}: {e}"))?;
    }
    let path = map_dir.join("openMapper").join(FILE_NAME);
    let existing = if path.exists() {
        Some(std::fs::read(&path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?)
    } else {
        None
    };
    let (mut root, before) = match &existing {
        Some(bytes) => {
            let before = String::from_utf8_lossy(bytes).into_owned();
            let value: serde_json::Value =
                serde_json::from_slice(bytes).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            let root = value
                .as_object()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("{}: not a JSON object", path.display()))?;
            (root, Some(before))
        }
        None => (serde_json::Map::new(), None),
    };
    if existing.is_some() {
        let version = root.get("version").ok_or_else(|| {
            anyhow::anyhow!(
                "{}: no \"version\" (expected integer 1); not written",
                path.display()
            )
        })?;
        anyhow::ensure!(
            version.as_u64() == Some(1),
            "{}: unsupported version {version} (expected integer 1); not written",
            path.display()
        );
    } else {
        root.insert("version".into(), serde_json::json!(1));
    }
    if !root.contains_key("crossings") {
        root.insert("crossings".into(), serde_json::json!({}));
    }
    let crossings = root
        .get_mut("crossings")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("{}: \"crossings\" is not an object", path.display()))?;
    let mut merged = Vec::new();
    for spelling in crossings.keys() {
        if spelling != &target && key(spelling) == target {
            merged.push(spelling.clone());
        }
    }
    for spelling in &merged {
        crossings.remove(spelling);
    }
    crossings.insert(target.clone(), serde_json::to_value(points)?);
    let after = serde_json::to_string_pretty(&serde_json::Value::Object(root))?;
    Ok(PreparedPoints {
        path,
        bytes: after.as_bytes().to_vec(),
        before,
        after,
        rel: target,
        merged,
    })
}

/// Rewrite `openMapper/crossing_signals.json`, replacing only crossing `rel`'s entry with
/// `points`. Every other key - and anything this editor does not read - is kept as it was.
/// A file that cannot be read, or whose version is not the integer 1, is left alone.
pub(crate) fn write_points(map_dir: &Path, rel: &str, points: &[SignalPoint]) -> anyhow::Result<()> {
    let prepared = prepare_points(map_dir, rel, points)?;
    if let Some(parent) = prepared.path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| anyhow::anyhow!("{}: {e}", parent.display()))?;
    }
    prepared.log_before();
    crate::doc::safe::write_atomic(&prepared.path, &prepared.bytes)
        .map_err(|e| anyhow::anyhow!("{}: {e}", prepared.path.display()))?;
    prepared.log_after();
    Ok(())
}

#[cfg(test)]
mod tests;
