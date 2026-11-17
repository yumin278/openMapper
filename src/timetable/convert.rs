//! Convert legacy trip stop records to the OMSI 2 station-link format.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use omsi_timetable::Trip;

use crate::doc::safe::Op;

use super::{key, Timetable};

#[derive(serde::Deserialize, serde::Serialize)]
struct Type2Exclusions {
    /// Effective exclusions at save time, including trips excluded by default.
    #[serde(default)]
    excluded: BTreeSet<String>,
    /// Explicit checkbox choices, reapplied over the defaults when the map is loaded.
    #[serde(default)]
    overrides: BTreeMap<String, bool>,
}

pub(super) fn exclusions_path(map_dir: &Path) -> PathBuf {
    crate::geo::dir_of(map_dir).join("type2_conversion_exclusions.json")
}

pub(super) fn load_exclusions(map_dir: &Path) -> BTreeMap<String, bool> {
    let path = exclusions_path(map_dir);
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Type2Exclusions>(&bytes) {
            Ok(settings) => settings.overrides.into_iter().map(|(name, excluded)| (key(&name), excluded)).collect(),
            Err(err) => {
                log::warn!("timetable: could not read {}: {err}", path.display());
                BTreeMap::new()
            }
        },
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::warn!("timetable: could not read {}: {err}", path.display());
            }
            BTreeMap::new()
        }
    }
}

/// A legacy trip can specify only a track, with no `[station]` records.
pub(crate) fn is_type1(trip: &Trip) -> bool {
    !trip.stations_legacy.is_empty() || (trip.stations.is_empty() && !trip.display_name.trim().is_empty())
}

impl Timetable {
    pub(crate) fn excludes_type2(&self, trip: &Trip) -> bool {
        self.type2_exclusions.get(&key(&trip.name)).copied().unwrap_or(trip.stations_legacy.len() < 2)
    }

    pub(crate) fn set_type2_exclusion(&mut self, trip: &Trip, excluded: bool) {
        if excluded != self.excludes_type2(trip) {
            self.type2_exclusions.insert(key(&trip.name), excluded);
            self.type2_exclusions_changed = true;
            self.edit = crate::doc::next_rev();
        }
    }

    pub(super) fn plan_type2_exclusions(&self, map_dir: &Path) -> Option<Op> {
        let path = exclusions_path(map_dir);
        let legacy = || self.trips.iter().filter(|t| is_type1(t));
        let excluded: BTreeSet<_> = legacy().filter(|t| self.excludes_type2(t)).map(|t| t.name.clone()).collect();
        let overrides: BTreeMap<_, _> = legacy().filter_map(|t| {
            let name = key(&t.name);
            self.type2_exclusions.get(&name).map(|&value| (name, value))
        }).collect();
        let existing = std::fs::read(&path).ok();
        if existing.is_none() && excluded.is_empty() && overrides.is_empty() {
            return None;
        }
        if !self.type2_exclusions_changed && existing.as_ref().is_some_and(|b| serde_json::from_slice::<Type2Exclusions>(b).is_err()) {
            return None;
        }
        let bytes = serde_json::to_vec_pretty(&Type2Exclusions { excluded, overrides }).expect("trip names and booleans can be serialized");
        if existing.as_deref() == Some(bytes.as_slice()) {
            return None;
        }
        Some(Op::Write(path, bytes))
    }

    /// Convert legacy trips selected for conversion. Returns the numbers converted and skipped.
    /// Like the OMSI editor, clear the track reference and leave the track files and station links alone.
    pub fn convert_all_to_type2(&mut self) -> (usize, usize) {
        let mut converted = Vec::new();
        let mut skipped = 0;
        for i in 0..self.trips.len() {
            let trip = &self.trips[i];
            if !is_type1(trip) {
                continue;
            }
            if self.excludes_type2(trip) {
                skipped += 1;
                continue;
            }
            let trip = &mut self.trips[i];
            if !trip.stations.is_empty() || trip.train_reverse {
                log::warn!("timetable: {} skipped in Type 2 conversion (mixed station formats or train reversal)", trip.name);
                skipped += 1;
                continue;
            }
            let ids = trip.stations_legacy.iter().enumerate().map(|(i, station)| {
                let id = station.first().map(|s| s.trim()).unwrap_or_default();
                id.parse::<i64>().ok().filter(|&id| id > 0).ok_or((i + 1, id))
            }).collect::<Result<Vec<_>, _>>();
            let ids = match ids {
                Ok(ids) => ids,
                Err((row, id)) => {
                    log::warn!("timetable: {} skipped in Type 2 conversion (stop {row} has invalid object ID {id:?})", trip.name);
                    skipped += 1;
                    continue;
                }
            };
            trip.stations = ids;
            trip.stations_legacy.clear();
            trip.display_name.clear();
            converted.push(trip.name.clone());
        }
        for name in &converted {
            self.touch_trip(name);
            let name = key(name);
            self.reroute.remove(&name);
            self.type2_exclusions_changed |= self.type2_exclusions.remove(&name).is_some();
            self.type2_converted.insert(name);
        }
        (converted.len(), skipped)
    }
}
