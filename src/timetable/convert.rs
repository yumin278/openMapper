//! Convert legacy trip stop records to the OMSI 2 station-link format.

use super::{key, Timetable};

impl Timetable {
    /// Convert every legacy trip with a valid stop sequence. Returns the numbers converted and skipped.
    /// Like the OMSI editor, clear the track reference and leave the track files and station links alone.
    pub fn convert_all_to_type2(&mut self) -> (usize, usize) {
        let mut converted = Vec::new();
        let mut skipped = 0;
        for trip in self.trips.iter_mut().filter(|t| !t.stations_legacy.is_empty()) {
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
