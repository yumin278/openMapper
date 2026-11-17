//! Convert legacy trip stop records to the OMSI 2 station-link format.

use omsi_timetable::Trip;

use super::{key, Timetable};

/// A legacy trip can specify only a track, with no `[station]` records.
pub(crate) fn is_type1(trip: &Trip) -> bool {
    !trip.stations_legacy.is_empty() || (trip.stations.is_empty() && !trip.display_name.trim().is_empty())
}

impl Timetable {
    pub(crate) fn excludes_type2(&self, trip: &Trip) -> bool {
        self.type2_exclusions.get(&key(&trip.name)).copied().unwrap_or(trip.stations_legacy.len() < 2)
    }

    pub(crate) fn set_type2_exclusion(&mut self, trip: &Trip, excluded: bool) {
        self.type2_exclusions.insert(key(&trip.name), excluded);
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
            self.type2_converted.insert(name);
        }
        (converted.len(), skipped)
    }
}
