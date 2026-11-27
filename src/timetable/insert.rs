//! Planning departures without changing the original tour.
use std::collections::{BTreeSet, HashMap};

use omsi_timetable::TourTrip;

pub type Profiles = HashMap<String, Vec<(String, f32)>>;
pub type Endpoints = HashMap<String, (i64, i64)>;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Basis {
    #[default]
    AfterArrival,
    BeforeDeparture,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    Any,
    Missing,
    Trip(String),
}

#[derive(Default)]
pub struct Selection {
    pub positions: BTreeSet<usize>,
    pub anchor: Option<usize>,
}

impl Selection {
    pub fn click(&mut self, index: usize, shift: bool, checked: bool) {
        if let (true, Some(anchor)) = (shift, self.anchor) {
            self.positions.extend(anchor.min(index)..=anchor.max(index));
        } else if checked {
            self.positions.insert(index);
        } else {
            self.positions.remove(&index);
        }
        self.anchor = Some(index);
    }

    pub fn clear(&mut self) {
        self.positions.clear();
        self.anchor = None;
    }
}

pub fn duration(profiles: &Profiles, trip: &str, profile: i32) -> f32 {
    profiles.get(&trip.to_lowercase()).and_then(|p| p.get(profile.max(0) as usize).or(p.first())).map(|p| p.1).unwrap_or(0.0)
}

pub fn arrival(profiles: &Profiles, trip: &TourTrip) -> f32 {
    trip.departure + duration(profiles, &trip.trip, trip.profile)
}

pub fn positions(trips: &[TourTrip]) -> std::ops::RangeInclusive<usize> {
    0..=trips.len()
}

pub fn neighbours(trips: &[TourTrip], index: usize) -> (Option<&TourTrip>, Option<&TourTrip>) {
    (index.checked_sub(1).and_then(|i| trips.get(i)), trips.get(index))
}

pub fn gap(trips: &[TourTrip], index: usize, profiles: &Profiles) -> Option<f32> {
    let (prev, next) = neighbours(trips, index);
    Some(next?.departure - arrival(profiles, prev?))
}

fn matches(filter: &Filter, trip: Option<&TourTrip>) -> bool {
    match filter {
        Filter::Any => true,
        Filter::Missing => trip.is_none(),
        Filter::Trip(name) => trip.is_some_and(|t| t.trip.to_lowercase() == name.to_lowercase()),
    }
}

pub fn matching_positions(trips: &[TourTrip], before: &Filter, after: &Filter) -> impl Iterator<Item = usize> {
    positions(trips).filter(move |&i| {
        let (prev, next) = neighbours(trips, i);
        matches(before, prev) && matches(after, next)
    })
}

pub struct Request<'a> {
    pub trip: &'a str,
    pub profile: i32,
    pub basis: Basis,
    pub offset_seconds: i32,
}

#[derive(Default)]
pub struct Plan {
    pub departures: Vec<TourTrip>,
    pub skipped: usize,
}

pub fn plan(trips: &[TourTrip], profiles: &Profiles, selected: &BTreeSet<usize>, request: &Request<'_>) -> Plan {
    let mut result = Plan::default();
    let offset = request.offset_seconds as f32 / 60.0;
    for &index in selected {
        if index > trips.len() {
            result.skipped += 1;
            continue;
        }
        let (prev, next) = neighbours(trips, index);
        let dep = match request.basis {
            Basis::AfterArrival => prev.map(|t| arrival(profiles, t) + offset),
            Basis::BeforeDeparture => next.map(|t| t.departure - duration(profiles, request.trip, request.profile) - offset),
        };
        if let Some(dep) = dep {
            let departure = (dep * 60.0).round().clamp(0.0, 172799.0) / 60.0;
            result.departures.push(TourTrip { trip: request.trip.to_owned(), profile: request.profile, departure });
        } else {
            result.skipped += 1;
        }
    }
    result
}

/// Order by time without moving the rows being edited.
pub fn chronological_indices(trips: &[TourTrip]) -> Vec<usize> {
    let mut order: Vec<_> = (0..trips.len()).collect();
    order.sort_by(|&a, &b| trips[a].departure.total_cmp(&trips[b].departure));
    order
}

/// Report overlap on the later departure, keyed by its original row index.
pub fn overlaps(trips: &[TourTrip], profiles: &Profiles) -> HashMap<usize, f32> {
    chronological_indices(trips).windows(2).filter_map(|pair| {
        let overlap = arrival(profiles, &trips[pair[0]]) - trips[pair[1]].departure;
        (overlap > 0.0).then_some((pair[1], overlap))
    }).collect()
}

pub fn discontinuity(prev: &TourTrip, next: &TourTrip, endpoints: &Endpoints) -> Option<(i64, i64)> {
    let &(_, end) = endpoints.get(&prev.trip.to_lowercase())?;
    let &(start, _) = endpoints.get(&next.trip.to_lowercase())?;
    (end != start).then_some((end, start))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::timetable::clock_secs;

    pub(crate) fn fixture() -> (Vec<TourTrip>, Profiles, Endpoints) {
        let profiles = HashMap::from([
            ("77_kk-kk".into(), vec![("standard".into(), 20.0), ("slow".into(), 21.0)]),
            ("77_kk_layover".into(), vec![("standard".into(), 2.0)]),
            ("depot_from_kk".into(), vec![("standard".into(), 5.0)]),
        ]);
        let endpoints = HashMap::from([
            ("77_kk-kk".into(), (4891, 4890)),
            ("77_kk_layover".into(), (4890, 593153)),
            ("depot_from_kk".into(), (4890, 4890)),
        ]);
        let trips = [524.25, 551.25, 578.25, 598.25].into_iter().enumerate().map(|(i, departure)| TourTrip {
            trip: if i == 3 { "Depot_from_kk" } else { "77_kk-kk" }.into(), profile: 0, departure,
        }).collect();
        (trips, profiles, endpoints)
    }

    pub(crate) fn print_tour(label: &str, trips: &[TourTrip], profiles: &Profiles) {
        println!("{label}:");
        for t in trips {
            println!("  {} profile {} {} -> {}", t.trip, t.profile, clock_secs(t.departure), clock_secs(arrival(profiles, t)));
        }
    }

    fn same_trip_positions(trips: &[TourTrip]) -> BTreeSet<usize> {
        let f = Filter::Trip("77_kk-kk".into());
        matching_positions(trips, &f, &f).collect()
    }

    fn request(basis: Basis, offset_seconds: i32) -> Request<'static> {
        Request { trip: "77_kk_layover", profile: 0, basis, offset_seconds }
    }

    fn inserted(trips: &[TourTrip], profiles: &Profiles, plan: Plan) -> Vec<TourTrip> {
        print_tour("Before", trips, profiles);
        let mut result = trips.to_vec();
        result.extend(plan.departures);
        result.sort_by(|a, b| a.departure.total_cmp(&b.departure));
        print_tour("After", &result, profiles);
        result
    }

    #[test]
    fn insert_matching_neighbours() {
        let (trips, profiles, endpoints) = fixture();
        print_tour("Before selection", &trips, &profiles);
        let selected = same_trip_positions(&trips);
        println!("Selected: {selected:?}");
        assert_eq!(selected, BTreeSet::from([1, 2]));
        assert_eq!(positions(&trips).count(), 5);
        assert_eq!(positions(&[]).count(), 1);
        assert_eq!(gap(&trips, 1, &profiles), Some(7.0));
        assert_eq!(discontinuity(&trips[0], &trips[1], &endpoints), Some((4890, 4891)));
        assert_eq!(discontinuity(&trips[2], &trips[3], &endpoints), None);
        print_tour("After selection", &trips, &profiles);
    }

    #[test]
    fn insert_after_arrivals_preserves_originals() {
        let (trips, profiles, _) = fixture();
        let p = plan(&trips, &profiles, &same_trip_positions(&trips), &request(Basis::AfterArrival, 0));
        assert_eq!(p.skipped, 0);
        assert_eq!(p.departures.iter().map(|t| clock_secs(t.departure)).collect::<Vec<_>>(), ["9:04:15", "9:31:15"]);
        let result = inserted(&trips, &profiles, p);
        assert_eq!(result.len(), 6);
        assert_eq!(result.iter().filter(|t| t.trip != "77_kk_layover").cloned().collect::<Vec<_>>(), trips);
        assert!(result.windows(2).all(|p| p[0].departure <= p[1].departure));
    }

    #[test]
    fn insert_uses_each_departures_profile() {
        let (mut trips, profiles, _) = fixture();
        trips[0].profile = 1;
        let p = plan(&trips, &profiles, &same_trip_positions(&trips), &request(Basis::AfterArrival, 0));
        assert_eq!(p.departures.iter().map(|t| clock_secs(t.departure)).collect::<Vec<_>>(), ["9:05:15", "9:31:15"]);
        let _ = inserted(&trips, &profiles, p);
        assert_eq!(duration(&profiles, "77_kk-kk", 99), 20.0);
        let p = plan(&trips, &profiles, &BTreeSet::from([1]), &Request { trip: "77_kk-kk", profile: 1, basis: Basis::BeforeDeparture, offset_seconds: 0 });
        assert_eq!(clock_secs(p.departures[0].departure), "8:50:15");
        let _ = inserted(&trips, &profiles, p);
    }

    #[test]
    fn insert_negative_offset_warns_but_does_not_block() {
        let (trips, profiles, _) = fixture();
        let p = plan(&trips, &profiles, &same_trip_positions(&trips), &request(Basis::AfterArrival, -90));
        let result = inserted(&trips, &profiles, p);
        assert_eq!(result.len(), 6);
        assert_eq!(overlaps(&result, &profiles), HashMap::from([(1, 1.5), (3, 1.5)]));
    }

    #[test]
    fn insert_before_departure_reserves_offset() {
        let (trips, profiles, _) = fixture();
        let p = plan(&trips, &profiles, &same_trip_positions(&trips), &request(Basis::BeforeDeparture, 60));
        assert_eq!(p.departures.iter().map(|t| clock_secs(t.departure)).collect::<Vec<_>>(), ["9:08:15", "9:35:15"]);
        let _ = inserted(&trips, &profiles, p);
    }

    #[test]
    fn insert_missing_reference_is_skipped() {
        let (trips, profiles, _) = fixture();
        let p = plan(&trips, &profiles, &BTreeSet::from([0]), &request(Basis::AfterArrival, 0));
        assert_eq!(p.skipped, 1);
        assert!(p.departures.is_empty());
        assert_eq!(inserted(&trips, &profiles, p), trips);
    }

    #[test]
    fn insert_shift_range_is_additive() {
        let (trips, profiles, _) = fixture();
        print_tour("Before", &trips, &profiles);
        let mut selection = Selection::default();
        selection.click(1, false, true);
        selection.click(3, true, true);
        assert_eq!(selection.positions, BTreeSet::from([1, 2, 3]));
        println!("Selected: {:?}", selection.positions);
        print_tour("After", &trips, &profiles);
    }
}
