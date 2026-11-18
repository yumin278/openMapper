//! A crossing object's own settings, kept beside its `.sco` as `X.crossing.json`: whether it
//! is a copy the crossing editor made, the signal heads the user put down by hand, and the
//! traffic rules a map object gets when its template is applied.
//!
//! ```json
//! { "version": 1,
//!   "copy": { "from": "Sceneryobjects\\...\\Kreuzung.sco" },
//!   "signals": [ { "light": 0, "set": "", "x": 0, "up": 0, "fwd": 0, "heading": 0 } ],
//!   "rule_template": [ { "path_index": 0, "kind": "bus", "value": 0, "extra": 0 } ] }
//! ```
//!
//! Only `version` is always written: `copy`, `signals` and `rule_template` are left out when
//! they say nothing. Nothing here is worked out from the crossing's roads or lanes - a signal
//! stands where a user put it, a template rule is one a user set - so an object openMapper did
//! not make can carry them too.
//!
//! The file is read and written as JSON values: keys this version does not know are kept as
//! they were, and a file that cannot be read, or whose `version` is not the integer 1, is left
//! alone rather than overwritten. Old copies made before the signals and templates moved here
//! hold only `{"from": "..."}`; that marker is still read (as `copy`), and the next save writes
//! the current form.
//!
//! [`prepare`] builds the file without touching the disk, so the crossing editor can put it in
//! the same `doc::safe` transaction as the object's `.sco`; [`write`] is the plain one-file
//! writer the junction makers use.

use std::path::{Path, PathBuf};

use glam::DVec3;
use serde::{Deserialize, Serialize};

/// The file beside `abs`'s `.sco` that holds the crossing's settings.
pub fn path_for(abs: &Path) -> PathBuf {
    abs.with_extension("crossing.json")
}

/// One signal the user put down: the light of the crossing's own program it shows, the signal
/// set (empty: the map's default), and where its head stands in the crossing's frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SignalPoint {
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

/// One rule of a crossing's template: a `[rule]` a map object's paths get when the template is
/// applied, in the object's own path numbers. A `[kill_rule]` is never part of a template.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemplateRule {
    pub path_index: i32,
    pub kind: String,
    #[serde(default)]
    pub value: f64,
    #[serde(default)]
    pub extra: f64,
}

impl TemplateRule {
    pub fn to_map_rule(&self) -> omsi_map::MapRule {
        omsi_map::MapRule { path_index: self.path_index, kind: self.kind.clone(), value: self.value, extra: self.extra, kill: false }
    }

    pub fn from_map_rule(r: &omsi_map::MapRule) -> Self {
        TemplateRule { path_index: r.path_index, kind: r.kind.clone(), value: r.value, extra: r.extra }
    }

    /// The rule in one line, for the log.
    pub fn describe(&self) -> String {
        format!("path {}, {} {}, extra {}", self.path_index, self.kind, self.value, self.extra)
    }

    fn validated(self, paths: usize) -> anyhow::Result<Self> {
        anyhow::ensure!(self.path_index >= 0, "path_index {} is negative", self.path_index);
        anyhow::ensure!((self.path_index as usize) < paths, "path_index {} is beyond the {paths} path(s) of the object", self.path_index);
        anyhow::ensure!(self.value.is_finite() && self.extra.is_finite(), "a rule's number is not finite");
        anyhow::ensure!(!self.kind.trim().is_empty(), "a rule names no kind");
        anyhow::ensure!(!self.kind.contains(char::is_whitespace), "a rule's kind holds a space: {:?}", self.kind);
        Ok(self)
    }
}

/// The template a bus bay's own lanes get: buses only, and no cars, as OMSI 2 writes them.
pub fn bus_template_rules(indices: &[usize]) -> Vec<TemplateRule> {
    indices
        .iter()
        .flat_map(|&index| {
            ["bus", "no_cars"].map(|kind| TemplateRule { path_index: index as i32, kind: kind.into(), value: 0.0, extra: 0.0 })
        })
        .collect()
}

/// A crossing object's settings, however the file was put there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CrossingConfig {
    /// The object this one was copied from, when the crossing editor made it.
    pub copy_from: Option<String>,
    pub signals: Vec<SignalPoint>,
    pub rules: Vec<TemplateRule>,
    /// Top-level keys this version does not read, kept as they were.
    extra: serde_json::Map<String, serde_json::Value>,
}

impl CrossingConfig {
    /// Settings set from what they are, with nothing carried over from a file.
    pub fn of(copy_from: Option<String>, signals: Vec<SignalPoint>, rules: Vec<TemplateRule>) -> Self {
        CrossingConfig { copy_from, signals, rules, extra: serde_json::Map::new() }
    }

    /// Whether the object is one the crossing editor copied.
    pub fn is_copy(&self) -> bool {
        self.copy_from.is_some()
    }

    /// Whether it says anything at all (nothing but `version` would be an empty file).
    pub fn is_empty(&self) -> bool {
        self.copy_from.is_none() && self.signals.is_empty() && self.rules.is_empty() && self.extra.is_empty()
    }

    /// The settings of the object whose `.sco` is `abs`, if it has a usable file: `paths` is
    /// how many `[path]`s the object has, which a template rule must name one of. None when
    /// there is no file, or it cannot be read; both are logged, and every entry that could not
    /// be used is logged and left out.
    pub fn load(abs: &Path, paths: usize) -> Option<CrossingConfig> {
        let path = path_for(abs);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log::info!("crossing settings {}: (no file)", path.display());
                return None;
            }
            Err(e) => {
                log::warn!("crossing settings {}: {e}; none loaded", path.display());
                return None;
            }
        };
        let value: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("crossing settings {}: {e}; none loaded", path.display());
                return None;
            }
        };
        let Some(root) = value.as_object() else {
            log::warn!("crossing settings {}: not a JSON object; none loaded", path.display());
            return None;
        };
        parse_config(&path, root, paths, true)
    }

    /// Whether the settings at `abs` are exactly these signals and rules. Silent, and cheap:
    /// the junction makers ask this while naming an object file, so two plates of one shape
    /// share a file only when their settings are the same. A file with no settings holds none;
    /// one that cannot be read matches nothing (its own file is left alone).
    pub fn same(abs: &Path, paths: usize, signals: &[SignalPoint], rules: &[TemplateRule]) -> bool {
        let path = path_for(abs);
        let Some(bytes) = std::fs::read(&path).ok() else {
            return signals.is_empty() && rules.is_empty();
        };
        let Some(root) = serde_json::from_slice::<serde_json::Value>(&bytes).ok().and_then(|v| v.as_object().cloned()) else {
            return false;
        };
        let Some(config) = parse_config(&path, &root, paths, false) else { return false };
        config.signals.as_slice() == signals && config.rules.as_slice() == rules
    }

    /// How many signal heads and how many template rules the object's settings hold, for a
    /// panel that asks every frame whether there is anything to offer. Silent, and cached
    /// against the file's size and time: a file edited elsewhere is read again.
    pub fn counts(abs: &Path) -> (usize, usize) {
        let path = path_for(abs);
        let Ok(meta) = std::fs::metadata(&path) else { return (0, 0) };
        let stamp = (meta.len(), meta.modified().ok());
        {
            let cache = counts_cache().lock().unwrap_or_else(|e| e.into_inner());
            if let Some(hit) = cache.get(&path) {
                if hit.0 == stamp {
                    return hit.1;
                }
            }
        }
        let counts = (|| -> Option<(usize, usize)> {
            let bytes = std::fs::read(&path).ok()?;
            let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
            let root = value.as_object()?;
            if root.get("version").and_then(|v| v.as_u64()) != Some(1) {
                return None;
            }
            let cfg = parse_config(&path, root, usize::MAX, false)?;
            Some((cfg.signals.len(), cfg.rules.len()))
        })()
        .unwrap_or((0, 0));
        counts_cache().lock().unwrap_or_else(|e| e.into_inner()).insert(path, (stamp, counts));
        counts
    }

    /// The file this would be written as, in full, as JSON.
    fn to_json(&self) -> serde_json::Value {
        let mut root = serde_json::Map::new();
        root.insert("version".into(), serde_json::json!(1));
        if let Some(from) = &self.copy_from {
            root.insert("copy".into(), serde_json::json!({ "from": from }));
        }
        if !self.signals.is_empty() {
            root.insert("signals".into(), serde_json::to_value(&self.signals).unwrap_or_default());
        }
        if !self.rules.is_empty() {
            root.insert("rule_template".into(), serde_json::to_value(&self.rules).unwrap_or_default());
        }
        for (key, value) in &self.extra {
            root.insert(key.clone(), value.clone());
        }
        serde_json::Value::Object(root)
    }
}

/// The last answer [`CrossingConfig::counts`] gave for each settings file, with the stamp of
/// the file it came from: panels ask every frame, and a file edited elsewhere must be read
/// again when its size or time changes.
fn counts_cache() -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, ((u64, Option<std::time::SystemTime>), (usize, usize))>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<PathBuf, ((u64, Option<std::time::SystemTime>), (usize, usize))>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Read the settings out of a JSON object, keeping what this version does not know. `warn`
/// says whether what was skipped is put in the log.
fn parse_config(path: &Path, root: &serde_json::Map<String, serde_json::Value>, paths: usize, warn: bool) -> Option<CrossingConfig> {
    macro_rules! warn_at {
        ($($a:tt)*) => {
            if warn {
                log::warn!($($a)*);
            }
        };
    }
    macro_rules! info_at {
        ($($a:tt)*) => {
            if warn {
                log::info!($($a)*);
            }
        };
    }
    let mut cfg = CrossingConfig::default();
    match root.get("version") {
        None => {
            // (a copy made before signals and templates lived here: only its source)
            if let Some(from) = root.get("from").and_then(|v| v.as_str()) {
                cfg.copy_from = Some(from.to_string());
                info_at!("crossing settings {}: old copy marker from {from:?}", path.display());
                return Some(cfg);
            }
            warn_at!("crossing settings {}: no \"version\" (expected integer 1); none loaded", path.display());
            return None;
        }
        Some(v) if v.as_u64() == Some(1) => {}
        Some(v) => {
            warn_at!("crossing settings {}: unsupported version {v} (expected integer 1); none loaded", path.display());
            return None;
        }
    }
    if let Some(copy) = root.get("copy").filter(|v| !v.is_null()) {
        match copy.get("from").and_then(|v| v.as_str()) {
            Some(from) => cfg.copy_from = Some(from.to_string()),
            None => warn_at!("crossing settings {}: \"copy\" has no \"from\"; read as not a copy", path.display()),
        }
    }
    if let Some(list) = root.get("signals").filter(|v| !v.is_null()) {
        match list.as_array() {
            Some(items) => {
                for (index, item) in items.iter().enumerate() {
                    match serde_json::from_value::<SignalPoint>(item.clone())
                        .map_err(anyhow::Error::from)
                        .and_then(|p| p.validated())
                    {
                        Ok(point) => cfg.signals.push(point),
                        Err(e) => warn_at!("crossing settings {}: skipping signals[{index}]: {e}", path.display()),
                    }
                }
            }
            None => warn_at!("crossing settings {}: \"signals\" is not an array; none read", path.display()),
        }
    }
    if let Some(list) = root.get("rule_template").filter(|v| !v.is_null()) {
        match list.as_array() {
            Some(items) => {
                for (index, item) in items.iter().enumerate() {
                    match serde_json::from_value::<TemplateRule>(item.clone())
                        .map_err(anyhow::Error::from)
                        .and_then(|r| r.validated(paths))
                    {
                        Ok(rule) => cfg.rules.push(rule),
                        Err(e) => warn_at!("crossing settings {}: skipping rule_template[{index}]: {e}", path.display()),
                    }
                }
            }
            None => warn_at!("crossing settings {}: \"rule_template\" is not an array; none read", path.display()),
        }
    }
    let mut extra = root.clone();
    for key in ["version", "copy", "signals", "rule_template"] {
        extra.remove(key);
    }
    cfg.extra = extra;
    info_at!(
        "crossing settings {}: copy={:?}, {} signal(s), {} template rule(s), {} other key(s)",
        path.display(),
        cfg.copy_from,
        cfg.signals.len(),
        cfg.rules.len(),
        cfg.extra.len()
    );
    Some(cfg)
}

/// The file [`prepare`] would write, made without touching the disk: the crossing editor puts
/// it in the same `doc::safe` transaction as the object's `.sco`, then calls
/// [`PreparedConfig::log_after`] once that transaction has succeeded.
pub struct PreparedConfig {
    /// The object's `X.crossing.json`.
    pub path: PathBuf,
    /// The whole file as it would stand; None when there is nothing left to write.
    pub bytes: Option<Vec<u8>>,
    /// Whether an existing file is to be removed: it held settings, and now holds none.
    pub remove: bool,
    /// The file as it stands now (none: there is no file yet).
    before: Option<String>,
    /// The file as it would stand, kept as text for the log.
    after: Option<String>,
}

impl PreparedConfig {
    /// The operations that install it, for [`crate::doc::safe::commit`].
    pub fn ops(&self) -> Vec<crate::doc::safe::Op> {
        match (&self.bytes, self.remove) {
            (Some(bytes), _) => vec![crate::doc::safe::Op::Write(self.path.clone(), bytes.clone())],
            (None, true) => vec![crate::doc::safe::Op::Remove(self.path.clone())],
            (None, false) => Vec::new(),
        }
    }

    /// Whether it changes anything on the disk.
    pub fn writes(&self) -> bool {
        self.bytes.is_some() || self.remove
    }

    /// The file as it stands, in full (or that there is none).
    pub fn log_before(&self) {
        match &self.before {
            Some(before) => log::info!("crossing settings {}: before write:\n{before}", self.path.display()),
            None => log::info!("crossing settings {}: before write: (no file)", self.path.display()),
        }
    }

    /// The file as it would stand, in full. Call this only once the write (or the transaction
    /// it is part of) has actually succeeded.
    pub fn log_after(&self) {
        match &self.after {
            Some(after) => log::info!("crossing settings {}: after write:\n{after}", self.path.display()),
            None => log::info!("crossing settings {}: after write: (no file)", self.path.display()),
        }
    }
}

/// Build the whole file from `cfg`, without writing anything. Every signal and template rule
/// must be usable: a place or a number a finite one, a rule naming a path the object has.
/// Nothing but `version` is an empty file; one that is there is then removed.
pub fn prepare(abs: &Path, cfg: &CrossingConfig, paths: usize) -> anyhow::Result<PreparedConfig> {
    let path = path_for(abs);
    for (index, point) in cfg.signals.iter().enumerate() {
        point.clone().validated().map_err(|e| anyhow::anyhow!("signal {index}: {e}"))?;
    }
    for (index, rule) in cfg.rules.iter().enumerate() {
        rule.clone().validated(paths).map_err(|e| anyhow::anyhow!("template rule {index}: {e}"))?;
    }
    let existing = std::fs::read(&path).ok();
    // (a file this version cannot read is not written over: it may be a later version's. A
    // bare old copy marker is read, and is written out in the current form)
    if let Some(bytes) = &existing {
        let readable = serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .map(|root| match root.get("version") {
                Some(v) => v.as_u64() == Some(1),
                None => root.get("from").and_then(|v| v.as_str()).is_some(),
            })
            .unwrap_or(false);
        anyhow::ensure!(readable, "{}: not a settings file this version reads; not written", path.display());
    }
    let before = existing.as_deref().map(|b| String::from_utf8_lossy(b).into_owned());
    let json = cfg.to_json();
    let empty = cfg.is_empty();
    let (bytes, remove, after) = if empty {
        (None, existing.is_some(), None)
    } else {
        let text = format!("{}\n", serde_json::to_string_pretty(&json)?);
        (Some(text.as_bytes().to_vec()), false, Some(text))
    };
    Ok(PreparedConfig { path, bytes, remove, before, after })
}

/// Write an object's settings beside its `.sco`: atomically, and only once everything in them
/// can be used. `paths` is how many `[path]`s the object has.
pub fn write(abs: &Path, cfg: &CrossingConfig, paths: usize) -> anyhow::Result<()> {
    let prepared = prepare(abs, cfg, paths)?;
    if !prepared.writes() {
        return Ok(());
    }
    if let Some(parent) = prepared.path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| anyhow::anyhow!("{}: {e}", parent.display()))?;
    }
    prepared.log_before();
    match (&prepared.bytes, prepared.remove) {
        (Some(bytes), _) => crate::doc::safe::write_atomic(&prepared.path, bytes)
            .map_err(|e| anyhow::anyhow!("{}: {e}", prepared.path.display()))?,
        (None, true) => std::fs::remove_file(&prepared.path).map_err(|e| anyhow::anyhow!("{}: {e}", prepared.path.display()))?,
        (None, false) => return Ok(()),
    }
    prepared.log_after();
    Ok(())
}

#[cfg(test)]
mod tests;
