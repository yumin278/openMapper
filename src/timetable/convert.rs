//! Convert legacy trip stop records to the OMSI 2 station-link format.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use omsi_timetable::Trip;

use crate::doc::safe::Op;

use super::{key, track_name, tracks, Timetable};

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

/// What a single format-only conversion changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Type2TripConversion {
    pub trip: String,
    pub stops: usize,
    /// The first `[trip]` line after conversion (empty when the trip may fall back to its
    /// own name without matching a track).
    pub display_name: String,
    /// The old track taken out of the timetable and queued for removal on save.
    pub removed_track: Option<String>,
    /// The old track left on disk because another trip still names it.
    pub kept_shared_track: Option<String>,
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The stop object IDs of a legacy `[station]` list, or the first row that is not a positive
/// ID. The records' other lines are dropped with the format.
fn legacy_station_ids(trip: &Trip) -> Result<Vec<i64>, (usize, String)> {
    trip.stations_legacy
        .iter()
        .enumerate()
        .map(|(i, station)| {
            let id = station.first().map(|s| s.trim()).unwrap_or_default();
            id.parse::<i64>().ok().filter(|&id| id > 0).ok_or((i + 1, id.to_string()))
        })
        .collect()
}

/// Replace a Type 1 trip's legacy records with Type 2 stop IDs. Profiles, terminus, line and
/// the route data are left as they are.
fn apply_type2_format(trip: &mut Trip, ids: Vec<i64>, display_name: String) {
    trip.stations = ids;
    trip.stations_legacy.clear();
    trip.display_name = display_name;
}

/// A nonempty `display_name` that names no track and no other trip's track: openOMSI then
/// reads the station links even while a track of the trip's own name must stay on disk for
/// another trip.
fn nonmatching_reference(tt: &Timetable, trip: &str, removed: Option<usize>) -> String {
    let taken = |candidate: &str| {
        tt.tracks.iter().enumerate().any(|(i, track)| Some(i) != removed && same(&track_name(track), candidate))
            || tt.trips.iter().any(|other| !same(&other.name, trip) && same(&tracks::referenced_name(other), candidate))
    };
    let base = format!("{trip} (station links)");
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|candidate| !taken(candidate.as_str()))
        .expect("an unused display name exists")
}

impl Timetable {
    pub(crate) fn excludes_type2(&self, trip: &Trip) -> bool {
        // (only a Type 1 trip is converted in a batch; a converted Type 2 trip has no
        // exclusion to remember even when it has fewer than two stops)
        is_type1(trip) && self.type2_exclusions.get(&key(&trip.name)).copied().unwrap_or(trip.stations_legacy.len() < 2)
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
            let ids = match legacy_station_ids(trip) {
                Ok(ids) => ids,
                Err((row, id)) => {
                    log::warn!("timetable: {} skipped in Type 2 conversion (stop {row} has invalid object ID {id:?})", trip.name);
                    skipped += 1;
                    continue;
                }
            };
            apply_type2_format(trip, ids, String::new());
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

    /// Convert one Type 1 trip's format to Type 2, whether or not the batch would exclude it.
    /// This changes no station link and finds no route: only the trip's stop records, its
    /// track reference and the old track file (when no other trip names it) change.
    pub(crate) fn convert_trip_to_type2(&mut self, name: &str) -> Result<Type2TripConversion, String> {
        let Some(mut trip) = self.trip(name).cloned() else {
            return Err(format!("trip {name:?} is not in the timetable"));
        };
        let exact = trip.name.clone();
        log::info!("timetable: single Type 2 conversion requested for {exact:?}; input trip: {trip:?}");
        if !is_type1(&trip) {
            let why = format!("trip {exact:?} is already Type 2");
            log::warn!("timetable: single Type 2 conversion refused: {why}");
            return Err(why);
        }
        if trip.train_reverse || !trip.stations.is_empty() {
            let what = if trip.train_reverse {
                "it is a train-reverse trip"
            } else {
                "it has both [station] and [station_typ2] records"
            };
            let why = format!("trip {exact:?} cannot be converted: {what}");
            log::warn!("timetable: single Type 2 conversion refused: {why}");
            return Err(why);
        }
        let ids = match legacy_station_ids(&trip) {
            Ok(ids) => ids,
            Err((row, id)) => {
                let why = format!("trip {exact:?}: stop {row} has invalid object ID {id:?}");
                log::warn!("timetable: single Type 2 conversion refused: {why}");
                return Err(why);
            }
        };
        let stops = ids.len();

        // (openOMSI reads the trimmed display_name when it is nonempty, else the trip's name.
        // The old track is the one that name resolves to now.)
        let old_ref = tracks::referenced_name(&trip);
        let other = tracks::other_trip_naming(self, &exact, &old_ref).map(|t| t.name.clone());
        let source_tracks: Vec<String> = self.tracks.iter().filter(|t| same(&track_name(t), &old_ref)).map(track_name).collect();
        let source_index = self.tracks.iter().position(|t| same(&track_name(t), &old_ref));
        let removed_index = source_index.filter(|_| other.is_none());
        // (an empty display_name falls back to the trip's name: a track of that name, whether
        // this trip's old source or an unrelated one, would intercept the station links)
        let fallback_taken = self.tracks.iter().enumerate().any(|(i, t)| Some(i) != removed_index && same(&track_name(t), &exact));
        let display_name = if fallback_taken { nonmatching_reference(self, &exact, removed_index) } else { String::new() };
        // (a trip with no stop IDs is only a Type 2 trip when its first line is empty; a
        // nonmatching nonempty line would keep it a track-only Type 1 trip)
        if ids.is_empty() && !display_name.is_empty() {
            let why = format!(
                "trip {exact:?} has no [station] records and its name still matches a track; it cannot become a Type 2 trip without a track reference"
            );
            log::warn!("timetable: single Type 2 conversion refused: {why}");
            return Err(why);
        }
        log::info!(
            "timetable: single Type 2 conversion of {exact:?}: old reference {old_ref:?}, source tracks {source_tracks:?}, other trip naming it {other:?}, source index {source_index:?}, remove index {removed_index:?}, chosen display_name {display_name:?}, stop IDs {ids:?}"
        );

        apply_type2_format(&mut trip, ids, display_name.clone());
        *self.trip_mut(&exact).expect("the trip was found above") = trip;
        let removed_track = if let Some(i) = removed_index {
            let track = self.tracks.remove(i);
            let stem = track_name(&track);
            let file = key(&format!("{stem}.ttr"));
            self.changed.remove(&file);
            self.removed.insert(file);
            self.hand.remove(&key(&stem));
            self.mark();
            Some(stem)
        } else {
            None
        };
        let kept_shared_track = (!source_tracks.is_empty() && other.is_some()).then_some(old_ref);
        self.hand.remove(&key(&exact));
        self.reroute.remove(&key(&exact));
        self.touch_trip(&exact);
        let exclusion_cleared = self.type2_exclusions.remove(&key(&exact)).is_some();
        self.type2_exclusions_changed |= exclusion_cleared;
        self.type2_converted.insert(key(&exact));
        let after = self.trip(&exact).cloned();
        let changed = self.changed.clone();
        let removed = self.removed.clone();
        log::info!(
            "timetable: single Type 2 conversion of {exact:?} written: output trip {after:?}; removed_track={removed_track:?}, kept_shared_track={kept_shared_track:?}, exclusion_cleared={exclusion_cleared}, changed={changed:?}, removed={removed:?}"
        );
        Ok(Type2TripConversion {
            trip: exact,
            stops,
            display_name,
            removed_track,
            kept_shared_track,
        })
    }
}
