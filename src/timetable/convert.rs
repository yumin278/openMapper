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

/// Header marker outside keyword blocks; old maps without it remain unchanged.
pub(super) const TYPE2_DRAFT: &str = "Type 2 stop preparation in openMapper";

pub(super) fn load_drafts(trips: &[Trip]) -> BTreeSet<String> {
    trips.iter().filter(|t| {
        std::fs::read(&t.path).ok().is_some_and(|bytes| {
            bytes.split(|&b| b == b'\n').any(|line| line.strip_suffix(b"\r").unwrap_or(line) == TYPE2_DRAFT.as_bytes())
        })
    }).map(|t| key(&t.name)).collect()
}

fn legacy_ids(trip: &Trip) -> Result<Vec<i64>, (usize, &str)> {
    trip.stations_legacy.iter().enumerate().map(|(i, station)| {
        let id = station.first().map(|s| s.trim()).unwrap_or_default();
        id.parse::<i64>().ok().filter(|&id| id > 0).ok_or((i + 1, id))
    }).collect()
}

impl Timetable {
    pub(crate) fn is_type2_draft(&self, trip: &Trip) -> bool {
        self.type2_drafts.contains(&key(&trip.name))
    }

    pub(crate) fn is_type1(&self, trip: &Trip) -> bool {
        is_type1(trip) || self.is_type2_draft(trip)
    }

    /// Start editing the future station list without changing the original track.
    /// Existing profile indices still refer to the same rows.
    pub(crate) fn prepare_type2_stops(&mut self, name: &str) -> bool {
        let Some(trip) = self.trip(name) else { return false };
        if self.is_type2_draft(trip) {
            return true;
        }
        if !is_type1(trip) || !trip.stations.is_empty() || trip.train_reverse {
            return false;
        }
        let Ok(ids) = legacy_ids(trip) else { return false };
        let trip = self.trip_mut(name).unwrap();
        if trip.display_name.trim().is_empty() {
            trip.display_name = trip.name.clone();
        }
        trip.stations = ids;
        trip.stations_legacy.clear();
        self.type2_drafts.insert(key(name));
        self.reroute.remove(&key(name));
        self.touch_trip(name);
        true
    }

    pub(crate) fn excludes_type2(&self, trip: &Trip) -> bool {
        self.type2_exclusions.get(&key(&trip.name)).copied().unwrap_or_else(|| if self.is_type2_draft(trip) { trip.stations.len() < 2 } else { trip.stations_legacy.len() < 2 })
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
        let legacy = || self.trips.iter().filter(|t| self.is_type1(t));
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
    /// Clear the original reference and remove only old tracks no other trip can use.
    /// Station links are still recalculated separately.
    pub fn convert_all_to_type2(&mut self) -> (usize, usize) {
        let mut converted = Vec::new();
        let mut old_tracks = BTreeSet::new();
        let mut skipped = 0;
        for i in 0..self.trips.len() {
            let trip = &self.trips[i];
            if !self.is_type1(trip) {
                continue;
            }
            if self.excludes_type2(trip) {
                skipped += 1;
                continue;
            }
            let draft = self.is_type2_draft(trip);
            let trip = &mut self.trips[i];
            if (!draft && !trip.stations.is_empty()) || trip.train_reverse {
                log::warn!("timetable: {} skipped in Type 2 conversion (mixed station formats or train reversal)", trip.name);
                skipped += 1;
                continue;
            }
            let ids = if draft {
                if trip.stations.iter().any(|&id| id <= 0) {
                    skipped += 1;
                    continue;
                }
                trip.stations.clone()
            } else {
                match legacy_ids(trip) {
                    Ok(ids) => ids,
                    Err((row, id)) => {
                        log::warn!("timetable: {} skipped in Type 2 conversion (stop {row} has invalid object ID {id:?})", trip.name);
                        skipped += 1;
                        continue;
                    }
                }
            };
            let track = if trip.display_name.trim().is_empty() { &trip.name } else { trip.display_name.trim() };
            old_tracks.insert(key(track));
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
            self.type2_drafts.remove(&name);
            self.type2_converted.insert(name);
        }
        for track in old_tracks {
            // The explicit reference wins; a different trip may also use its own named track.
            let used = self.trips.iter().any(|t| {
                t.display_name.trim().eq_ignore_ascii_case(&track)
                    || (!self.type2_converted.contains(&key(&t.name)) && t.name.eq_ignore_ascii_case(&track))
            });
            if !used && self.tracks.iter().any(|t| key(&super::track_name(t)) == track) {
                self.tracks.retain(|t| key(&super::track_name(t)) != track);
                let file = format!("{track}.ttr");
                self.changed.remove(&file);
                self.removed.insert(file);
                self.hand.remove(&track);
            }
        }
        (converted.len(), skipped)
    }
}
