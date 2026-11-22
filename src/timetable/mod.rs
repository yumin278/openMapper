//! The map's timetable (`TTData`): bus stops, station links, trips, tracks and lines, read
//! with openOMSI's own `omsi-timetable` and written back in OMSI 2's formats.
//!
//! Only what was changed is written: a stock map's files stay as they are until a line,
//! trip or tour of it is edited.

pub mod caruse;
mod convert;
pub mod links;
pub mod network;
pub mod profile;
pub mod route;
pub(crate) mod tracks;
pub mod way;

pub(crate) use convert::{is_type1, Type2TripConversion};

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use omsi_timetable::{BusStopEntry, Line, StnLink, StnLinkEntry, TimetableData, Track, TrackEntry, Trip};

use crate::doc::safe::Op;
use crate::doc::{Document, TileKey};

/// Days a tour runs on: bits 0-6 Monday…Sunday, 7 public holidays, 8 school holidays,
/// 9 school days. 1023: every day.
pub const ALL_DAYS: i32 = 1023;

#[derive(Debug, Clone, Default)]
pub struct Timetable {
    pub stops: Vec<BusStopEntry>,
    pub links: Vec<StnLink>,
    pub trips: Vec<Trip>,
    pub tracks: Vec<Track>,
    pub lines: Vec<Line>,
    /// The tiles in the order the files' tile indices count them (global.cfg's `[map]` list
    /// when they were read or last written).
    pub order: Vec<TileKey>,
    /// Changed since read or saved.
    pub dirty: bool,
    /// Counts each change, so a save that is still writing can tell a later edit from the one it copied.
    pub edit: u64,
    /// Files to write (lower-case names) and to take away on the next save.
    changed: BTreeSet<String>,
    removed: BTreeSet<String>,
    /// The stop list and the station links are written again.
    stops_changed: bool,
    links_changed: bool,
    /// Trips whose route is to be found again (their stops changed).
    pub reroute: BTreeSet<String>,
    /// Trips converted since the last save, whose existing route is kept on that save.
    pub type2_converted: BTreeSet<String>,
    /// Manual overrides of the default Type 2 conversion exclusion.
    type2_exclusions: BTreeMap<String, bool>,
    type2_exclusions_changed: bool,
    /// Public holidays and school holidays (`Holidays.txt`): which tours run on a day.
    pub calendar: omsi_map::Calendar,
    calendar_changed: bool,
    /// Which vehicles drive the lines (`car_use/*.ocu`), and the files to write and take away.
    pub car_use: Vec<caruse::CarUseFile>,
    car_changed: BTreeSet<String>,
    car_removed: BTreeSet<String>,
    /// Trips whose paths were picked by hand (lower-case names): their way is kept as it is
    /// when their stops change, not found again. Their `.ttr` says so in a line the game
    /// passes over ([`BY_HAND`]).
    hand: BTreeSet<String>,
}

/// The line a track picked by hand carries in its header (text outside a `[keyword]` block,
/// which OMSI 2 and openOMSI pass over).
pub const BY_HAND: &str = "Paths picked by hand in openMapper";

fn key(name: &str) -> String {
    name.to_lowercase()
}

/// `[station]` records and `[station_typ2]` stops of a trip file as written: each block is
/// read to the next keyword line, not to a fixed eight lines (see
/// [`reread_short_station_records`]).
fn station_records(lines: &[String]) -> (Vec<Vec<String>>, Vec<i64>) {
    let mut legacy: Vec<Vec<String>> = Vec::new();
    let mut ids: Vec<i64> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let keyword = omsi_cfg::keyword_of(&lines[i]).map(str::to_ascii_lowercase);
        i += 1;
        let Some(keyword) = keyword else { continue };
        if keyword != "station" && keyword != "station_typ2" {
            continue;
        }
        let typ2 = keyword == "station_typ2";
        // (a `[station]` record ends at the next keyword or at its blank line - unless it has
        // fewer than three fields so far, where the blank is an empty name or track index; a
        // `[station_typ2]` stop is its one line)
        let ends_at_blank = if typ2 { 1 } else { 3 };
        let mut fields: Vec<String> = Vec::new();
        while i < lines.len() {
            if omsi_cfg::keyword_of(&lines[i]).is_some() {
                break;
            }
            let line = lines[i].trim_end();
            if line.is_empty() && fields.len() >= ends_at_blank {
                break;
            }
            fields.push(line.to_string());
            i += 1;
        }
        while fields.last().is_some_and(|l| l.is_empty()) {
            fields.pop();
        }
        if typ2 {
            if let Some(id) = fields.first().and_then(|s| s.trim().parse::<i64>().ok()) {
                ids.push(id);
            }
        } else {
            legacy.push(fields);
        }
    }
    (legacy, ids)
}

/// The `[profile]` entries of the file, from the first one on (the same reading as
/// openOMSI's, which the trip reader uses for the rest).
fn profile_records(f: &omsi_cfg::CfgFile) -> Vec<omsi_timetable::TripProfile> {
    let Some(at) = f.lines.iter().position(|l| omsi_cfg::keyword_of(l).is_some_and(|k| k.eq_ignore_ascii_case("profile"))) else {
        return Vec::new();
    };
    let mut profiles: Vec<omsi_timetable::TripProfile> = Vec::new();
    let mut r = f.reader();
    r.seek(at);
    while let Some(k) = r.next_keyword() {
        match k.as_str() {
            "profile" => profiles.push(omsi_timetable::TripProfile { name: r.str().to_string(), factor: r.f32(), ..Default::default() }),
            "profile_man_arr_time" => {
                let (i, v) = (r.i32(), r.f32());
                if let Some(p) = profiles.last_mut() {
                    p.man_arr_time.push((i, v));
                }
            }
            "profile_man_dep_time" => {
                let (i, v) = (r.i32(), r.f32());
                if let Some(p) = profiles.last_mut() {
                    p.man_dep_time.push((i, v));
                }
            }
            "profile_otherstopping" => {
                let (i, v) = (r.i32(), r.i32());
                if let Some(p) = profiles.last_mut() {
                    p.other_stopping.push((i, v));
                }
            }
            _ => {}
        }
    }
    profiles
}

/// A `[station]` record of an older mod map can be shorter than the eight lines OMSI 2
/// writes (or the blank line between records can be missing), and openOMSI's trip reader
/// takes the next record's `[station]` line for the missing field - it then skips that whole
/// record, and every other stop of the trip is gone (stops 2, 4, 6 of six: the Type 2
/// conversion then writes only half of them). Read such files again to the next keyword, and
/// take the profiles with them, which the same slip can leave out.
pub(crate) fn reread_short_station_records(trips: &mut [Trip]) {
    for trip in trips {
        if trip.stations_legacy.is_empty() {
            continue;
        }
        let Ok(f) = omsi_cfg::CfgFile::read(&trip.path) else { continue };
        let (legacy, ids) = station_records(&f.lines);
        let more_stops = legacy.len() > trip.stations_legacy.len();
        let more_typ2 = ids.len() > trip.stations.len();
        if !more_stops && !more_typ2 {
            continue;
        }
        log::warn!(
            "timetable: {} has {} `[station]` records and {} `[station_typ2]` stops but {} and {} were read (records shorter than the usual eight lines); read again to the next keyword",
            trip.name,
            legacy.len(),
            ids.len(),
            trip.stations_legacy.len(),
            trip.stations.len()
        );
        if more_stops {
            trip.stations_legacy = legacy;
        }
        if more_typ2 {
            trip.stations = ids;
        }
        trip.profiles = profile_records(&f);
    }
}

/// Everything a group of route writes can change, for an operation that must be all or
/// nothing: the tracks, the station links, the hand markers, and what saving would write
/// for them.
pub(crate) struct RouteState {
    tracks: Vec<Track>,
    links: Vec<StnLink>,
    hand: BTreeSet<String>,
    changed: BTreeSet<String>,
    removed: BTreeSet<String>,
    links_changed: bool,
    reroute: BTreeSet<String>,
    type2_converted: BTreeSet<String>,
}

impl Timetable {
    /// Read `TTData` of the map folder `dir`; `order` is global.cfg's tile list.
    pub fn load(dir: &Path, order: &[TileKey]) -> Timetable {
        let mut d = TimetableData::load(dir);
        for e in &d.errors {
            log::warn!("timetable: {e}");
        }
        reread_short_station_records(&mut d.trips);
        let n = (d.lines.len(), d.trips.len(), d.tracks.len(), d.bus_stops.len());
        if n != (0, 0, 0, 0) {
            log::info!("timetable: {} lines, {} trips, {} tracks, {} stops", n.0, n.1, n.2, n.3);
        }
        let mut calendar = omsi_map::Calendar::load(&omsi_cfg::resolve_path(dir, "Holidays.txt")).unwrap_or_default();
        // (in the order of the days, as the Lines window lists them)
        calendar.holidays.sort_by_key(|h| h.date);
        calendar.ranges.sort_by_key(|r| r.start);
        let hand = d.tracks.iter().filter(|t| marked_by_hand(&t.path)).map(|t| key(&track_name(t))).collect();
        Timetable { stops: d.bus_stops, links: d.stn_links, trips: d.trips, tracks: d.tracks, lines: d.lines, order: order.to_vec(), calendar, car_use: caruse::load_dir(dir), hand, type2_exclusions: convert::load_exclusions(dir), ..Default::default() }
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty || self.type2_exclusions_changed
    }

    /// The trip's paths were picked by hand.
    pub fn by_hand(&self, trip: &str) -> bool {
        // (the marker is the track's; openOMSI reads the trimmed display_name and, when it is
        // nonempty, does not fall back to the trip's name. The marker therefore describes the
        // trip only while the game reads <trip>.ttr, not another track or the station links.)
        if !self.hand.contains(&key(trip)) {
            return false;
        }
        let (name, source) = tracks::source(self, trip);
        source.is_some() && name.eq_ignore_ascii_case(trip)
    }

    /// The hand marker of a track file, by its stem. This is the marker itself, not a trip's
    /// mode (a track can be named by a differently named trip's display_name).
    pub fn by_hand_track(&self, track: &str) -> bool {
        self.hand.contains(&key(track))
    }

    /// The trip's way as picked by hand (`track`, and the station links between its stops
    /// along it), or (None) found along the roads again from now on.
    ///
    /// A hand-picked way is the only one that writes `<trip>.ttr`. Its links are checked like
    /// every other write, and the track name is checked against the other trips and the other
    /// hand-picked tracks: when anything refuses, no link, track or hand state is changed.
    pub fn set_by_hand(&mut self, trip: &str, way: Option<(Vec<TrackEntry>, Vec<StnLink>)>) -> Result<(), links::RouteRefusal> {
        match way {
            Some((track, links)) => {
                let conflicts = links::link_conflicts(self, trip, &links);
                if !conflicts.is_empty() {
                    let refusal = links::RouteRefusal::links(conflicts);
                    refusal.log(trip);
                    return Err(refusal);
                }
                let plan = tracks::hand_plan(self, trip).map_err(links::RouteRefusal::track)?;
                log_route_input(self, trip, &track, &links, true);
                self.write_hand_route(trip, track, links, plan.clear_display);
                log::info!("timetable: the way of {trip} is kept by hand; its .ttr and its station links were written");
                Ok(())
            }
            None => {
                self.hand.remove(&key(trip));
                self.touch_route(trip);
                log::info!("timetable: the way of {trip} is found automatically again (its old .ttr is kept until the new route is written)");
                Ok(())
            }
        }
    }

    pub fn trip(&self, name: &str) -> Option<&Trip> {
        self.trips.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn trip_mut(&mut self, name: &str) -> Option<&mut Trip> {
        self.trips.iter_mut().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn track(&self, name: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| track_name(t).eq_ignore_ascii_case(name))
    }

    /// The name stop object `id` has in the timetable.
    pub fn stop_name(&self, id: i64) -> Option<&str> {
        self.stops.iter().find(|s| s.object_id == id).map(|s| s.name.trim())
    }

    pub fn touch_line(&mut self, name: &str) {
        self.changed.insert(key(&format!("{name}.ttl")));
        self.removed.remove(&key(&format!("{name}.ttl")));
        self.mark();
    }

    pub fn touch_trip(&mut self, name: &str) {
        self.changed.insert(key(&format!("{name}.ttp")));
        self.removed.remove(&key(&format!("{name}.ttp")));
        self.mark();
    }

    pub fn touch_stops(&mut self) {
        self.stops_changed = true;
        self.mark();
    }

    /// A trip's stops changed: its route is found again.
    pub fn touch_route(&mut self, name: &str) {
        self.touch_trip(name);
        self.type2_converted.remove(&key(name));
        self.reroute.insert(key(name));
    }

    /// Whether the trip's `.ttp` was already to be written or taken away, before an edit.
    pub(crate) fn trip_file_state(&self, name: &str) -> (bool, bool) {
        let k = key(&format!("{name}.ttp"));
        (self.changed.contains(&k), self.removed.contains(&k))
    }

    /// Put a trip's stops and its file bookkeeping back after an edit that was undone: the
    /// `.ttp` is written only when it was to be written before the edit, so an untouched
    /// file keeps its bytes.
    pub(crate) fn restore_stop_edit(&mut self, name: &str, trip: Trip, files: (bool, bool), type2_converted: bool) {
        if let Some(t) = self.trip_mut(name) {
            let name_now = t.name.clone();
            *t = trip;
            t.name = name_now;
        }
        let k = key(&format!("{name}.ttp"));
        if files.0 {
            self.changed.insert(k.clone());
        } else {
            self.changed.remove(&k);
        }
        if files.1 {
            self.removed.insert(k);
        } else {
            self.removed.remove(&k);
        }
        if type2_converted {
            self.type2_converted.insert(key(name));
        } else {
            self.type2_converted.remove(&key(name));
        }
    }

    /// The route state before a group of writes, to put back if one of them fails.
    pub(crate) fn route_state(&self) -> RouteState {
        RouteState {
            tracks: self.tracks.clone(),
            links: self.links.clone(),
            hand: self.hand.clone(),
            changed: self.changed.clone(),
            removed: self.removed.clone(),
            links_changed: self.links_changed,
            reroute: self.reroute.clone(),
            type2_converted: self.type2_converted.clone(),
        }
    }

    pub(crate) fn restore_route_state(&mut self, state: RouteState) {
        self.tracks = state.tracks;
        self.links = state.links;
        self.hand = state.hand;
        self.changed = state.changed;
        self.removed = state.removed;
        self.links_changed = state.links_changed;
        self.reroute = state.reroute;
        self.type2_converted = state.type2_converted;
    }

    /// Put back as it was (undo, redo) in place of `replaced`: every file of it is written
    /// again on the next save, and those only `replaced` had are taken away.
    pub fn restored(&mut self, replaced: &Timetable) {        self.type2_exclusions_changed |= self.type2_exclusions != replaced.type2_exclusions;
        let names = |t: &Timetable| -> BTreeSet<String> {
            t.lines.iter().map(|l| key(&format!("{}.ttl", l.name))).chain(t.trips.iter().map(|x| key(&format!("{}.ttp", x.name)))).chain(t.tracks.iter().map(|x| key(&format!("{}.ttr", track_name(x))))).collect()
        };
        let mine = names(self);
        self.removed = names(replaced).difference(&mine).cloned().chain(replaced.removed.iter().cloned()).filter(|n| !mine.contains(n)).collect();
        self.changed = mine;
        self.stops_changed = true;
        self.links_changed = true;
        self.calendar_changed = true;
        let have: BTreeSet<String> = self.car_use.iter().map(|c| c.file.to_lowercase()).collect();
        self.car_removed = replaced.car_use.iter().map(|c| c.file.to_lowercase()).filter(|f| !have.contains(f)).chain(replaced.car_removed.iter().cloned()).filter(|f| !have.contains(f)).collect();
        self.car_changed = have;
        self.reroute.clear();
        self.mark();
    }

    pub fn touch_calendar(&mut self) {
        self.calendar_changed = true;
        self.mark();
    }

    /// The same lines, trips, tours and stops (what the Lines window edits; not the routes
    /// found for them or what is still to be saved).
    pub fn same_content(&self, o: &Timetable) -> bool {
        let hand_tracks = |t: &Timetable| -> Vec<Track> { t.tracks.iter().filter(|x| t.hand.contains(&key(&track_name(x)))).cloned().collect() };
        self.lines == o.lines && self.trips == o.trips && self.stops == o.stops && self.calendar == o.calendar && self.car_use == o.car_use && self.hand == o.hand && self.type2_exclusions == o.type2_exclusions && hand_tracks(self) == hand_tracks(o)
    }

    /// A new line (its name made unique).
    pub fn add_line(&mut self, name: &str) -> String {
        let name = self.unique(name, |t, n| t.lines.iter().any(|l| l.name.eq_ignore_ascii_case(n)));
        self.lines.push(Line { name: name.clone(), user_allowed: true, priority: 1, ..Default::default() });
        self.touch_line(&name);
        name
    }

    pub fn rename_line(&mut self, old: &str, new: &str) -> Option<String> {
        let new = new.trim();
        if new.is_empty() || !valid_file_name(new) {
            return None;
        }
        if !old.eq_ignore_ascii_case(new) && self.lines.iter().any(|l| l.name.eq_ignore_ascii_case(new)) {
            return None;
        }
        let l = self.lines.iter_mut().find(|l| l.name == old)?;
        l.name = new.to_string();
        self.removed.insert(key(&format!("{old}.ttl")));
        self.changed.remove(&key(&format!("{old}.ttl")));
        self.touch_line(new);
        self.car_use_follow(old, Some(new));
        // the trips that show it as their line number
        let trips: Vec<String> = self.trips.iter().filter(|t| t.line.trim() == old).map(|t| t.name.clone()).collect();
        for t in trips {
            if let Some(t) = self.trip_mut(&t) {
                t.line = new.to_string();
            }
            self.touch_trip(&t);
        }
        Some(new.to_string())
    }

    /// A copy of a line under a new name (a variant to change): its trips copied with its
    /// name in theirs ("76_A-B" → "76 copy_A-B"), their ways along, its tours driving the
    /// copies. The copy's name.
    pub fn duplicate_line(&mut self, name: &str) -> Option<String> {
        let src = self.lines.iter().find(|l| l.name == name)?.clone();
        let new = self.add_line(&format!("{name} {}", crate::i18n::tr("copy", "копия")));
        let mut trips: BTreeSet<String> = src.tours.iter().flat_map(|t| t.trips.iter().map(|x| x.trip.clone())).collect();
        trips.extend(self.trips.iter().filter(|t| t.line.trim() == name).map(|t| t.name.clone()));
        let mut renamed: std::collections::HashMap<String, String> = Default::default();
        for t in trips {
            let Some(old) = self.trip(&t).cloned() else { continue };
            let base = match t.strip_prefix(&format!("{name}_")) {
                Some(rest) => format!("{new}_{rest}"),
                None => format!("{new}_{t}"),
            };
            let copy = self.unique(&base, |tt, n| tt.trips.iter().any(|x| x.name.eq_ignore_ascii_case(n)));
            let line = if old.line.trim() == name { new.clone() } else { old.line.clone() };
            let legacy = is_type1(&old);
            let mut copy_trip = Trip { name: copy.clone(), line, path: Default::default(), ..old };
            // (a first line that names the original's track would make an automatic copy read
            // that track; the copy's own <name>.ttr or its station links are what it should
            // use. A Type 1 copy keeps its records as they are.)
            if !legacy {
                let display = copy_trip.display_name.trim().to_string();
                if !display.is_empty() && self.tracks.iter().any(|x| track_name(x).eq_ignore_ascii_case(&display)) {
                    copy_trip.display_name.clear();
                }
            }
            self.trips.push(copy_trip);
            self.touch_trip(&copy);
            // (a hand-picked way is copied with its track; a Type 1 trip keeps its own track
            // too; an automatic Type 2 trip keeps no .ttr and the copy's station links are
            // calculated again)
            let hand_track = self.by_hand_track(&t);
            if hand_track || legacy {
                if let Some(track) = self.track(&t).map(|x| x.entries.clone()) {
                    self.tracks.push(Track { path: format!("{copy}.ttr").into(), entries: track });
                    self.changed.insert(key(&format!("{copy}.ttr")));
                    if hand_track {
                        self.hand.insert(key(&copy));
                    }
                } else {
                    self.reroute.insert(key(&copy));
                }
            } else {
                self.reroute.insert(key(&copy));
            }
            renamed.insert(t.to_lowercase(), copy);
        }
        let l = self.lines.iter_mut().find(|l| l.name == new)?;
        l.user_allowed = src.user_allowed;
        l.priority = src.priority;
        l.tours = src.tours;
        for x in l.tours.iter_mut().flat_map(|t| t.trips.iter_mut()) {
            if let Some(n) = renamed.get(&x.trip.to_lowercase()) {
                x.trip = n.clone();
            }
        }
        self.touch_line(&new);
        Some(new)
    }

    /// Take a line away, and the trips no other line runs.
    pub fn delete_line(&mut self, name: &str) {
        let Some(i) = self.lines.iter().position(|l| l.name == name) else { return };
        let l = self.lines.remove(i);
        self.car_use_follow(name, None);
        self.removed.insert(key(&format!("{name}.ttl")));
        self.changed.remove(&key(&format!("{name}.ttl")));
        let mut trips: BTreeSet<String> = l.tours.iter().flat_map(|t| t.trips.iter().map(|x| x.trip.clone())).collect();
        trips.extend(self.trips.iter().filter(|t| t.line.trim() == name).map(|t| t.name.clone()));
        for t in trips {
            if !self.lines.iter().any(|l| l.tours.iter().any(|to| to.trips.iter().any(|x| x.trip.eq_ignore_ascii_case(&t)))) {
                self.delete_trip(&t);
            }
        }
        self.mark();
    }

    /// A new trip of a line, its stops `stops`.
    pub fn add_trip(&mut self, line: &str, name: &str, stops: Vec<i64>) -> String {
        let name = self.unique(name, |t, n| t.trips.iter().any(|x| x.name.eq_ignore_ascii_case(n)));
        let terminus = stops.last().and_then(|id| self.stop_name(*id)).unwrap_or_default().to_string();
        self.trips.push(Trip {
            name: name.clone(),
            terminus,
            line: line.to_string(),
            stations: stops,
            profiles: vec![omsi_timetable::TripProfile { name: "standard".into(), factor: 10.0, ..Default::default() }],
            ..Default::default()
        });
        self.touch_route(&name);
        name
    }

    pub fn rename_trip(&mut self, old: &str, new: &str) -> Option<String> {
        let new = new.trim();
        if new.is_empty() || !valid_file_name(new) {
            return None;
        }
        if !old.eq_ignore_ascii_case(new) && self.trip(new).is_some() {
            return None;
        }
        self.trip_mut(old)?.name = new.to_string();
        if self.type2_converted.remove(&key(old)) {
            self.type2_converted.insert(key(new));
        }
        if let Some(excluded) = self.type2_exclusions.remove(&key(old)) {
            self.type2_exclusions.insert(key(new), excluded);
            self.type2_exclusions_changed = true;
        }
        for ext in ["ttp", "ttr"] {
            self.removed.insert(key(&format!("{old}.{ext}")));
            self.changed.remove(&key(&format!("{old}.{ext}")));
        }
        // (a track of the old name is renamed with the trip; a track another trip still
        // names must not be moved or taken away)
        let old_track = self.tracks.iter().any(|t| track_name(t).eq_ignore_ascii_case(old));
        let shared = tracks::other_trip_naming(self, new, old).is_some();
        let mut hand_track = false;
        if old_track && shared {
            self.removed.remove(&key(&format!("{old}.ttr")));
            self.changed.remove(&key(&format!("{old}.ttr")));
            log::warn!("timetable: the track {old}.ttr is still named by another trip; it is left as it is");
        } else if old_track {
            if let Some(t) = self.tracks.iter_mut().find(|t| track_name(t).eq_ignore_ascii_case(old)) {
                t.path = format!("{new}.ttr").into();
                self.changed.insert(key(&format!("{new}.ttr")));
                hand_track = self.by_hand_track(old);
            }
        }
        if self.hand.remove(&key(old)) && hand_track {
            self.hand.insert(key(new));
        }
        // (a first line that named the old track would keep the game looking for that name;
        // clear it so the renamed trip's own track or its station links are read)
        if !(old_track && shared) {
            if let Some(t) = self.trip_mut(new) {
                if t.display_name.trim().eq_ignore_ascii_case(old) {
                    t.display_name.clear();
                }
            }
        }
        for l in &mut self.lines {
            let mut hit = false;
            for t in l.tours.iter_mut().flat_map(|t| t.trips.iter_mut()) {
                if t.trip.eq_ignore_ascii_case(old) {
                    t.trip = new.to_string();
                    hit = true;
                }
            }
            if hit {
                self.changed.insert(key(&format!("{}.ttl", l.name)));
            }
        }
        self.touch_trip(new);
        Some(new.to_string())
    }

    /// Take a trip away, its track and the departures of it.
    pub fn delete_trip(&mut self, name: &str) {
        // (a track another retained trip still names is that trip's way, not this one's)
        let shared = tracks::other_trip_naming(self, name, name).is_some();
        self.trips.retain(|t| !t.name.eq_ignore_ascii_case(name));
        self.removed.insert(key(&format!("{name}.ttp")));
        self.changed.remove(&key(&format!("{name}.ttp")));
        if shared {
            log::warn!("timetable: the track {name}.ttr is still named by another trip; it is left as it is");
            self.changed.remove(&key(&format!("{name}.ttr")));
        } else {
            self.tracks.retain(|t| !track_name(t).eq_ignore_ascii_case(name));
            self.removed.insert(key(&format!("{name}.ttr")));
            self.changed.remove(&key(&format!("{name}.ttr")));
        }
        let mut touched = Vec::new();
        for l in &mut self.lines {
            let before: usize = l.tours.iter().map(|t| t.trips.len()).sum();
            for t in &mut l.tours {
                t.trips.retain(|x| !x.trip.eq_ignore_ascii_case(name));
            }
            if l.tours.iter().map(|t| t.trips.len()).sum::<usize>() != before {
                touched.push(l.name.clone());
            }
        }
        for l in touched {
            self.touch_line(&l);
        }
        self.reroute.remove(&key(name));
        self.type2_converted.remove(&key(name));
        self.type2_exclusions_changed |= self.type2_exclusions.remove(&key(name)).is_some();
        self.hand.remove(&key(name));
        self.mark();
    }

    /// Replace a hand-picked way with the automatic route found for it. The new links and the
    /// track repair are both checked first: a failure leaves the hand-picked way, its track,
    /// the station list, the links and the queues exactly as they were.
    pub fn set_automatic_route(&mut self, trip: &str, track: Vec<TrackEntry>, links: Vec<StnLink>) -> Result<(), links::RouteRefusal> {
        self.set_automatic_route_among(trip, track, links, &std::collections::HashSet::new())
    }

    /// The automatic route of a trip written together with a batch (`peers`: the trips the
    /// batch writes, by lower-case name; its own check already compared their links).
    pub(crate) fn set_automatic_route_among(
        &mut self, trip: &str, track: Vec<TrackEntry>, links: Vec<StnLink>, peers: &std::collections::HashSet<String>,
    ) -> Result<(), links::RouteRefusal> {
        self.write_route_among(trip, track, links, peers, false)
    }

    /// The route found for a trip: its station links (`StnLinks.cfg`, which OMSI 2 drives a
    /// Type 2 bus trip by). An automatic route writes no `.ttr`; a same-name or `display_name`
    /// track that would intercept the links is repaired away first. The single write gate:
    /// when a link would change one other trips drive on, or the track cannot be repaired
    /// safely, nothing of the route is written.
    pub fn set_route(&mut self, trip: &str, track: Vec<TrackEntry>, links: Vec<StnLink>) -> Result<(), links::RouteRefusal> {
        self.set_route_among(trip, track, links, &std::collections::HashSet::new())
    }

    /// [`set_route`](Self::set_route) for a batch that has already compared the links of the
    /// trips it writes together (`peers`).
    pub(crate) fn set_route_among(
        &mut self, trip: &str, track: Vec<TrackEntry>, links: Vec<StnLink>, peers: &std::collections::HashSet<String>,
    ) -> Result<(), links::RouteRefusal> {
        self.write_route_among(trip, track, links, peers, false)
    }

    /// A route written with its mode chosen: `by_hand` writes `<trip>.ttr`, otherwise only the
    /// station links are written and an intercepting track is repaired away.
    pub(crate) fn write_route_among(
        &mut self, trip: &str, track: Vec<TrackEntry>, links: Vec<StnLink>, peers: &std::collections::HashSet<String>, by_hand: bool,
    ) -> Result<(), links::RouteRefusal> {
        let conflicts = links::link_conflicts_among(self, trip, &links, peers);
        if !conflicts.is_empty() {
            let refusal = links::RouteRefusal::links(conflicts);
            refusal.log(trip);
            return Err(refusal);
        }
        log_route_input(self, trip, &track, &links, by_hand);
        if by_hand {
            let plan = tracks::hand_plan(self, trip).map_err(links::RouteRefusal::track)?;
            log::info!("timetable: hand route of {trip}: clear display_name = {}", plan.clear_display);
            self.write_hand_route(trip, track, links, plan.clear_display);
        } else {
            let plan = tracks::automatic_plan(self, trip).map_err(links::RouteRefusal::track)?;
            log::info!("timetable: automatic route of {trip}: remove {plan:?}");
            self.write_automatic_route(trip, track, links, &plan);
        }
        Ok(())
    }

    /// Write the links although other trips drive on them: only after the user confirmed the
    /// overwrite. Returns what was overwritten (the affected trips are logged). A track that
    /// cannot be written or repaired safely is still refused: the user confirmed the links,
    /// not another trip's loss of its way.
    pub fn overwrite_route(
        &mut self, trip: &str, track: Vec<TrackEntry>, links: Vec<StnLink>, by_hand: bool,
    ) -> Result<Vec<links::SharedLinkConflict>, links::RouteRefusal> {
        let conflicts = links::link_conflicts(self, trip, &links);
        log::info!("timetable: the user overwrites the shared station links of {trip}; {} link(s) affected", conflicts.len());
        for c in &conflicts {
            log::info!("timetable: overwritten by the user: {}", c.log_text());
        }
        log_route_input(self, trip, &track, &links, by_hand);
        if by_hand {
            let plan = tracks::hand_plan(self, trip).map_err(links::RouteRefusal::track)?;
            log::info!("timetable: overwritten hand route of {trip}: clear display_name = {}", plan.clear_display);
            self.write_hand_route(trip, track, links, plan.clear_display);
        } else {
            let plan = tracks::automatic_plan(self, trip).map_err(links::RouteRefusal::track)?;
            log::info!("timetable: overwritten automatic route of {trip}: remove {plan:?}");
            self.write_automatic_route(trip, track, links, &plan);
        }
        Ok(conflicts)
    }

    /// Write a hand-picked way: the trip's `<trip>.ttr` with the hand header, its station
    /// links, and the hand mark. A nonempty `display_name` that is not the trip's own name is
    /// cleared: the game would look only for that name instead of `<trip>.ttr`.
    fn write_hand_route(&mut self, trip: &str, track: Vec<TrackEntry>, links: Vec<StnLink>, clear_display: bool) {
        if clear_display {
            if let Some(t) = self.trip_mut(trip) {
                if !t.display_name.is_empty() {
                    log::info!("timetable: clearing the display_name {:?} of {trip}: its hand-picked track is {trip}.ttr", t.display_name);
                    t.display_name.clear();
                }
            }
            self.touch_trip(trip);
        }
        match self.tracks.iter_mut().find(|t| track_name(t).eq_ignore_ascii_case(trip)) {
            Some(t) => t.entries = track,
            None => self.tracks.push(Track { path: format!("{trip}.ttr").into(), entries: track }),
        }
        let name = key(&format!("{trip}.ttr"));
        self.changed.insert(name.clone());
        self.removed.remove(&name);
        self.hand.insert(key(trip));
        for l in &links {
            log::info!("timetable: writing the hand route of {trip}: StnLink {}", links::describe_link("new", l));
        }
        let count = links.len();
        self.write_links(links);
        log::info!("timetable: the way of {trip} is kept by hand: {trip}.ttr and {count} station link(s) were written");
        self.mark();
    }

    /// Write an automatic Type 2 route: the station links only. The tracks in the plan are
    /// taken away (and their files queued for removal) and a `display_name` that would make
    /// the game read a track is cleared. The hand mark is dropped.
    fn write_automatic_route(&mut self, trip: &str, track: Vec<TrackEntry>, links: Vec<StnLink>, plan: &tracks::AutomaticPlan) {
        if plan.clear_display {
            let before = self.trip(trip).map(|t| t.display_name.clone()).unwrap_or_default();
            if let Some(t) = self.trip_mut(trip) {
                t.display_name.clear();
            }
            log::info!("timetable: cleared the display_name {before:?} of {trip}: the game reads its station links now");
            self.touch_trip(trip);
        }
        for name in &plan.remove {
            log::info!("timetable: repairing {trip}: removing the intercepting track {name}.ttr");
            self.drop_track(name);
        }
        for name in &plan.left_shared {
            log::info!("timetable: {trip}: the track {name}.ttr is left for another trip that still names it");
        }
        for name in &plan.left {
            log::info!("timetable: {trip}: the old track {name}.ttr is left in place; it no longer intercepts");
        }
        if self.hand.remove(&key(trip)) {
            log::info!("timetable: the way of {trip} is no longer picked by hand");
        }
        for l in &links {
            log::info!("timetable: writing the automatic route of {trip}: StnLink {}", links::describe_link("new", l));
        }
        let count = links.len();
        self.write_links(links);
        log::info!(
            "timetable: automatic route of {trip} written: {count} station link(s), {} track(s) removed, {} left for another trip, {} old track(s) left in place; the calculated track has {} entry(ies) and is not written",
            plan.remove.len(),
            plan.left_shared.len(),
            plan.left.len(),
            track.len()
        );
    }

    /// Remove a track from the timetable and queue its file for removal on the next save.
    fn drop_track(&mut self, stem: &str) {
        self.tracks.retain(|t| !track_name(t).eq_ignore_ascii_case(stem));
        let file = key(&format!("{stem}.ttr"));
        self.changed.remove(&file);
        self.removed.insert(file);
        // (the hand mark names the track, so it goes with it)
        self.hand.remove(&key(stem));
        self.mark();
    }

    /// Write the links (one per pair, replacing the map's) and mark the stop links changed.
    fn write_links(&mut self, links: Vec<StnLink>) {
        for l in links {
            self.links.retain(|x| !(x.from_id == l.from_id && x.to_id == l.to_id));
            self.links.push(l);
        }
        self.links_changed = true;
        self.mark();
    }

    /// The stop list brought in line with the map's stop objects (`stop_entries`): names and
    /// tiles follow the objects, new stops are added. True when anything changed.
    pub fn sync_stops(&mut self, entries: Vec<BusStopEntry>) -> bool {
        let mut changed = false;
        for e in entries {
            match self.stops.iter_mut().find(|s| s.object_id == e.object_id) {
                Some(s) => {
                    if s.name.trim() != e.name || s.group != e.group || s.params[0] != e.params[0] {
                        s.name = e.name;
                        s.group = e.group;
                        s.params[0] = e.params[0];
                        changed = true;
                    }
                }
                None => {
                    self.stops.push(e);
                    changed = true;
                }
            }
        }
        if changed {
            self.touch_stops();
        }
        changed
    }

    /// `base`, or `base 2`, `base 3`… when `taken` says that is used.
    fn unique(&self, base: &str, taken: impl Fn(&Timetable, &str) -> bool) -> String {
        let base = base.trim();
        let base = if base.is_empty() { "1" } else { base };
        if !taken(self, base) {
            return base.to_string();
        }
        (2..).map(|i| format!("{base} {i}")).find(|n| !taken(self, n)).unwrap()
    }

    /// The tile indices made to count in `order` (tiles were added or taken away since
    /// the files were read): every file that holds one is written again.
    pub(crate) fn renumber_tiles(&mut self, order: &[TileKey]) {
        if self.order.is_empty() || self.order == order {
            self.order = order.to_vec();
            return;
        }
        let map = |i: f64| renumbered(&self.order, i, order);
        for s in &mut self.stops {
            s.group = map(s.group as f64) as i32;
        }
        for l in &mut self.links {
            for e in &mut l.entries {
                e.values[2] = map(e.values[2]);
            }
        }
        let names: Vec<String> = self.tracks.iter().map(track_name).collect();
        for t in &mut self.tracks {
            for e in &mut t.entries {
                if e.values.len() > 2 {
                    e.values[2] = map(e.values[2]);
                }
            }
        }
        for n in names {
            self.changed.insert(key(&format!("{n}.ttr")));
        }
        self.stops_changed = true;
        self.links_changed = true;
        self.order = order.to_vec();
    }
}

/// The whole input of a route write: what the game would read now, the calculated track and
/// every station link, so the log holds both sides of the decision.
fn log_route_input(tt: &Timetable, trip: &str, track: &[TrackEntry], links: &[StnLink], by_hand: bool) {
    let (name, existing) = tracks::source(tt, trip);
    log::info!(
        "timetable: route input for {trip} ({}): the game reads {name:?} ({}) with {} track entry(ies); {} calculated track entry(ies), {} station link(s)",
        if by_hand { "hand" } else { "automatic" },
        if existing.is_some() { "a track" } else { "the station links" },
        existing.map(|t| t.entries.len()).unwrap_or(0),
        track.len(),
        links.len()
    );
    let entries: Vec<String> = track.iter().map(|e| e.values.iter().map(|v| format!("{v:.3}")).collect::<Vec<_>>().join("/")).collect();
    log::info!("timetable: route input for {trip}: calculated track entries [{}]", entries.join(", "));
    for l in links {
        log::info!("timetable: route input for {trip}: StnLink {}", links::describe_link("new", l));
    }
}

/// Tile index `i` of files written with the tile list `old`, counted in `order`.
fn renumbered(old: &[TileKey], i: f64, order: &[TileKey]) -> f64 {
    old.get(i as usize).and_then(|k| order.iter().position(|o| o == k)).map(|n| n as f64).unwrap_or(i)
}

/// The stops' tiles as their objects stand now (one moved over into another tile names
/// its new one), written on the next save.
pub fn follow_stop_tiles(doc: &mut Document) {
    if doc.timetable.stops.is_empty() {
        return;
    }
    let mut at: std::collections::HashMap<i64, i32> = std::collections::HashMap::new();
    for (i, k) in doc.order.iter().enumerate() {
        if let Some(t) = doc.tiles.get(k) {
            at.extend(t.tile.objects.iter().map(|o| (o.id, i as i32)));
        }
    }
    let tt = &mut doc.timetable;
    let moved = tt.stops.iter().any(|s| at.get(&s.object_id).is_some_and(|g| *g as f64 != renumbered(&tt.order, s.group as f64, &doc.order)));
    if !moved {
        return;
    }
    // (the indices counted in today's list first, then the moved ones set)
    tt.renumber_tiles(&doc.order);
    for s in &mut tt.stops {
        if let Some(g) = at.get(&s.object_id) {
            s.group = *g;
        }
    }
    tt.touch_stops();
}

/// The `Busstops.cfg` entries of stop objects (tile, IDCode): the name is the object's
/// first label, the number after the id its fourth (as OMSI's editor writes them).
pub fn stop_entries(doc: &Document, ids: &[(TileKey, i64)]) -> Vec<BusStopEntry> {
    ids.iter()
        .filter_map(|&(k, id)| {
            let strings = doc.object(k, id).map(|o| &o.extra).or_else(|| doc.attachment(k, id).map(|a| &a.strings))?;
            let tile = doc.order.iter().position(|t| *t == k)?;
            let name = strings.first().map(|s| s.trim().to_string()).unwrap_or_default();
            let offset = strings.get(3).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
            Some(BusStopEntry { name, group: tile as i32, object_id: id, params: [offset, 0.0, 0.0] })
        })
        .collect()
}

/// Where stop object `id` stands (world).
pub fn stop_world(doc: &Document, files: &mut network::Files, id: i64) -> Option<glam::DVec3> {
    let (k, o) = doc.tiles.iter().find_map(|(k, t)| t.tile.objects.iter().find(|o| o.id == id).map(|o| (*k, o)))?;
    let abs = files.get(&o.file).map(|f| f.abs_height).unwrap_or(false);
    Some(doc.object_world(k, o, abs))
}

/// A name a file can have on every system.
pub fn valid_file_name(n: &str) -> bool {
    !n.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()) && n.trim() == n && !n.ends_with('.')
}

pub fn track_name(t: &Track) -> String {
    t.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// What saving the timetable writes and takes away in the map folder `map_dir` (`order`:
/// the tile list as saved): only what changed. The map's save carries it out with its own
/// files, all or nothing; `saved` then marks it done.
pub fn plan(tt: &mut Timetable, map_dir: &Path, order: &[TileKey]) -> Vec<Op> {
    let mut ops = Vec::new();
    ops.extend(tt.plan_type2_exclusions(map_dir));
    // (a tile taken away renumbers the files' tile indices even when nothing of the
    // timetable was edited)
    let held = !(tt.stops.is_empty() && tt.links.is_empty() && tt.tracks.is_empty());
    if !tt.dirty && !(held && !tt.order.is_empty() && tt.order != order) {
        tt.order = order.to_vec();
        return ops;
    }
    tt.renumber_tiles(order);
    let dir = map_dir.join("TTData");
    for f in &tt.removed {
        if let Some(p) = find_file(&dir, f) {
            ops.push(Op::Remove(p));
        }
    }
    // (a file there in another case is written under its own name: on a disk that ignores
    // case, writing the new name and removing the old would remove both)
    let at = |name: &str| find_file(&dir, &name.to_lowercase()).unwrap_or_else(|| dir.join(name));
    for f in &tt.changed {
        let (stem, ext) = f.rsplit_once('.').unwrap_or((f.as_str(), ""));
        let text = match ext {
            "ttl" => tt.lines.iter().find(|l| key(&l.name) == stem).map(|l| (l.name.clone(), line_text(l))),
            "ttp" => tt.trips.iter().find(|t| key(&t.name) == stem).map(|t| (t.name.clone(), trip_text(t))),
            "ttr" => tt.tracks.iter().find(|t| key(&track_name(t)) == stem).map(|t| (track_name(t), track_text_of(t, tt.hand.contains(stem)))),
            _ => None,
        };
        if let Some((name, text)) = text {
            ops.push(Op::Write(at(&format!("{name}.{ext}")), encode(&text)));
        }
    }
    if tt.stops_changed || !dir.join("Busstops.cfg").is_file() {
        ops.push(Op::Write(at("Busstops.cfg"), encode(&busstops_text(&tt.stops))));
    }
    if tt.links_changed {
        ops.push(Op::Write(at("StnLinks.cfg"), encode(&stnlinks_text(&tt.links, &tt.stops))));
    }
    if tt.calendar_changed {
        let p = find_file(map_dir, "holidays.txt").unwrap_or_else(|| map_dir.join("Holidays.txt"));
        ops.push(Op::Write(p, encode(&holidays_text(&tt.calendar))));
    }
    ops.extend(caruse::plan(tt, map_dir));
    ops
}

impl Timetable {
    /// (a number never given twice: undo puts an older timetable back, with its older count)
    pub(crate) fn mark(&mut self) {
        self.dirty = true;
        self.edit = crate::doc::next_rev();
    }

    /// Bring the tile indices in line with the map's tile list here, where `plan` would on the
    /// copy a background save writes from: this timetable must end up the same as the files.
    pub(crate) fn follow_order(&mut self, order: &[TileKey]) {
        let held = !(self.stops.is_empty() && self.links.is_empty() && self.tracks.is_empty());
        if held && !self.order.is_empty() && self.order != order {
            self.renumber_tiles(order);
            self.mark();
        } else {
            self.order = order.to_vec();
        }
    }
}

/// The planned files were written.
pub fn saved(tt: &mut Timetable) {
    tt.removed.clear();
    tt.changed.clear();
    tt.type2_converted.clear();
    tt.type2_exclusions_changed = false;
    tt.stops_changed = false;
    tt.links_changed = false;
    tt.calendar_changed = false;
    caruse::saved(tt);
    tt.dirty = false;
}

/// Write what changed into `map_dir/TTData` by itself (all or nothing).
#[cfg(test)]
pub fn save_to(tt: &mut Timetable, map_dir: &Path, order: &[TileKey]) -> anyhow::Result<()> {
    let ops = plan(tt, map_dir, order);
    crate::doc::safe::commit(&ops)?;
    saved(tt);
    Ok(())
}

/// The track file says its paths were picked by hand ([`BY_HAND`]).
fn marked_by_hand(path: &Path) -> bool {
    let Ok(b) = std::fs::read(path) else { return false };
    let text = if b.starts_with(&[0xFF, 0xFE]) {
        String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
    } else {
        String::from_utf8_lossy(&b).into_owned()
    };
    // (in the header, before the first entry)
    text.split("[track_entry]").next().unwrap_or_default().contains(BY_HAND)
}

/// A file of the folder by its lower-case name.
fn find_file(dir: &Path, lower: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase() == lower).unwrap_or(false))
}

/// The trip file in the folder the map was read from, or in its working copy after saving.
pub(crate) fn saved_trip_path(map_dir: &Path, source: Option<&Path>, fresh: bool, name: &str) -> Option<std::path::PathBuf> {
    if fresh {
        return None;
    }
    let dir = find_file(source.unwrap_or(map_dir), "ttdata")?;
    find_file(&dir, &key(&format!("{name}.ttp"))).filter(|p| p.is_file())
}

/// The text as OMSI 2 writes TTData: Windows-1252 with CR LF; text that code page cannot
/// hold (Cyrillic stop names) as UTF-16 with a byte order mark, which openOMSI reads too.
pub fn encode(text: &str) -> Vec<u8> {
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let (bytes, _, lossy) = omsi_cfg::codepage::CodePage::Windows1252.encoding().encode(&crlf);
    if !lossy {
        return bytes.into_owned();
    }
    let mut out = vec![0xFF, 0xFE];
    for u in crlf.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

fn header(kind: &str) -> String {
    let rule = "-".repeat(kind.len() + 2);
    format!("{rule}\n{kind}\n{rule}\n\nCreated with openMapper {}\n\n", env!("CARGO_PKG_VERSION"))
}

/// A number as OMSI writes the fractional ones in TTData (ten decimals).
fn f10(v: f64) -> String {
    format!("{v:.10}")
}

/// An id or index kept whole.
fn int(v: f64) -> String {
    format!("{}", v.round() as i64)
}

pub fn busstops_text(stops: &[BusStopEntry]) -> String {
    let mut o = header("Time Table BusStopList File");
    for s in stops {
        o.push_str(&format!("[busstop]\n{}\n{}\n{}\n{}\n{}\n{}\n\n\n", s.name.trim(), s.group, s.object_id, f10(s.params[0]), int(s.params[1]), int(s.params[2])));
    }
    o
}

pub fn stnlinks_text(links: &[StnLink], stops: &[BusStopEntry]) -> String {
    let name = |id: i64| stops.iter().find(|s| s.object_id == id).map(|s| s.name.trim().to_string()).unwrap_or_else(|| id.to_string());
    let mut o = header("Time Table StnLinkList File");
    for l in links {
        o.push_str(&format!("{} ==> {}\n\n[StnLink]\n{}\n{}\n{}\n", name(l.from_id), name(l.to_id), f10(l.length), l.from_id, l.to_id));
        for (i, p) in l.params.iter().enumerate() {
            // (the last two: a flag and the index of the last entry)
            o.push_str(&if i < 4 { f10(*p) } else { int(*p) });
            o.push('\n');
        }
        o.push('\n');
        for (i, e) in l.entries.iter().enumerate() {
            o.push_str(&stnlink_entry_text(i, e));
        }
    }
    o
}

fn stnlink_entry_text(i: usize, e: &StnLinkEntry) -> String {
    let v = &e.values;
    format!("   {i}:\n[StnLink_entry]\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n", int(v[0]), int(v[1]), int(v[2]), f10(v[3]), int(v[4]), int(v[5]), int(v[6]))
}

pub fn trip_text(t: &Trip) -> String {
    let mut o = header("Time Table Trip File");
    o.push_str(&format!("[trip]\n{}\n{}\n{}\n\n", t.display_name, t.terminus, t.line));
    if t.train_reverse {
        o.push_str("[trainreverse]\n\n");
    }
    o.push_str(".........................\n        Stations\n.........................\n\n");
    for s in &t.stations {
        o.push_str(&format!("[station_typ2]\n{s}\n\n"));
    }
    for s in &t.stations_legacy {
        o.push_str("[station]\n");
        for l in s {
            o.push_str(l);
            o.push('\n');
        }
        o.push('\n');
    }
    o.push_str("\n.........................\n        Profiles\n.........................\n\n");
    for p in &t.profiles {
        o.push_str(&format!("[profile]\n{}\n{:.3}\n\n", p.name, p.factor));
        for (i, v) in &p.man_arr_time {
            o.push_str(&format!("[profile_man_arr_time]\n{i}\n{v:.3}\n"));
        }
        for (i, v) in &p.man_dep_time {
            o.push_str(&format!("[profile_man_dep_time]\n{i}\n{v:.3}\n"));
        }
        for (i, v) in &p.other_stopping {
            o.push_str(&format!("[profile_otherstopping]\n{i}\n{v}\n"));
        }
        o.push('\n');
    }
    o
}

pub fn track_text(t: &Track) -> String {
    track_text_of(t, false)
}

/// The track's text; picked by hand, it says so in its header.
pub fn track_text_of(t: &Track, by_hand: bool) -> String {
    let mut o = header("Time Table Track File");
    if by_hand {
        o.push_str(BY_HAND);
        o.push_str("\n\n");
    }
    for (i, e) in t.entries.iter().enumerate() {
        o.push_str(&format!("   {i}:\n[track_entry]\n"));
        for (k, v) in e.values.iter().enumerate() {
            // id, path, tile, internal number: whole; then the length and the rest
            o.push_str(&if k == 4 { f10(*v) } else if v.fract() == 0.0 { int(*v) } else { f10(*v) });
            o.push('\n');
        }
        o.push('\n');
    }
    o
}

/// `Holidays.txt` as OMSI 2 has it: per year its school holidays (`[holidays]` first day,
/// last day, name), then its public holidays (`[holiday]` day, name).
pub fn holidays_text(c: &omsi_map::Calendar) -> String {
    let mut years: BTreeSet<i32> = c.ranges.iter().map(|r| r.start / 10_000).chain(c.holidays.iter().map(|h| h.date / 10_000)).collect();
    if years.is_empty() {
        years.insert(0);
    }
    let mut o = String::new();
    for y in years {
        o.push_str(&format!("-----------------{y}------------------------------\n\n"));
        let mut ranges: Vec<_> = c.ranges.iter().filter(|r| r.start / 10_000 == y).collect();
        ranges.sort_by_key(|r| r.start);
        for r in ranges {
            o.push_str(&format!("[holidays]\n{}\n{}\n{}\n\n", r.start, r.end, r.name.trim()));
        }
        o.push_str("..............................\n\n");
        let mut days: Vec<_> = c.holidays.iter().filter(|h| h.date / 10_000 == y).collect();
        days.sort_by_key(|h| h.date);
        for h in days {
            o.push_str(&format!("[holiday]\n{}\n{}\n\n", h.date, h.name.trim()));
        }
    }
    o
}

pub fn line_text(l: &Line) -> String {
    let mut o = header("Time Table Line File");
    if l.user_allowed {
        o.push_str("[userallowed]\n\n");
    }
    o.push_str(&format!("[priority]\n{}\n", l.priority));
    for t in &l.tours {
        o.push_str("------------------------------------\n\n");
        o.push_str(&format!("[newtour]\n{}\n{}\n{}\n\n------------------------------------\n\n", t.number, t.ai_group, t.extra));
        for tr in &t.trips {
            let secs = (tr.departure as f64 * 60.0).round() as i64;
            o.push_str(&format!("  Dep.: {}:{}:{}\n", secs / 3600, secs / 60 % 60, secs % 60));
            o.push_str(&format!("[addtrip]\n{}\n{}\n{:.3}\n\n", tr.trip, tr.profile, tr.departure));
        }
    }
    o
}

/// When a trip is at each of its stops (minutes after it leaves): (arrival, departure, the
/// profile fixes it), as openOMSI reckons them: the first stop at 0, the last at the
/// profile's time, the fixed ones as given and the others spread between them by the
/// distance along the route (`legs`: metres from each stop to the next; a missing leg
/// counts 500 m, as there).
pub fn stop_times(n: usize, legs: &[f64], p: &omsi_timetable::TripProfile) -> Vec<(f32, f32, bool)> {
    let duration = if p.factor > 0.0 { p.factor } else { 10.0 };
    let mut arr: Vec<Option<f32>> = vec![None; n];
    let mut dep: Vec<Option<f32>> = vec![None; n];
    for &(i, m) in &p.man_arr_time {
        if let Some(a) = usize::try_from(i).ok().and_then(|i| arr.get_mut(i)) {
            *a = Some(m);
        }
    }
    for &(i, m) in &p.man_dep_time {
        if let Some(d) = usize::try_from(i).ok().and_then(|i| dep.get_mut(i)) {
            *d = Some(m);
        }
    }
    let fixed: Vec<bool> = (0..n).map(|i| arr[i].is_some() || dep[i].is_some()).collect();
    if n > 0 && !fixed[0] {
        dep[0] = Some(0.0);
    }
    if n > 1 && !fixed[n - 1] {
        arr[n - 1] = Some(duration);
    }
    let mut along = vec![0.0f64; n];
    for i in 1..n {
        along[i] = along[i - 1] + legs.get(i - 1).copied().filter(|l| *l > 0.0).unwrap_or(500.0).max(1.0);
    }
    let timed: Vec<usize> = (0..n).filter(|&i| arr[i].is_some() || dep[i].is_some()).collect();
    let mut out = Vec::with_capacity(n);
    let mut last = 0.0f32;
    for i in 0..n {
        let (a, d) = match (arr[i], dep[i]) {
            (Some(a), Some(d)) => (a, d.max(a)),
            (Some(a), None) => (a, a),
            (None, Some(d)) => (d, d),
            (None, None) => {
                let p = timed.iter().rev().find(|&&k| k < i).copied();
                let q = timed.iter().find(|&&k| k > i).copied();
                let t = match (p, q) {
                    (Some(p), Some(q)) => {
                        let t0 = dep[p].or(arr[p]).unwrap_or(0.0);
                        let t1 = arr[q].or(dep[q]).unwrap_or(t0);
                        let span = along[q] - along[p];
                        if span > 0.0 { t0 + (t1 - t0) * ((along[i] - along[p]) / span) as f32 } else { t0 }
                    }
                    (Some(p), None) => dep[p].or(arr[p]).unwrap_or(0.0),
                    (None, Some(q)) => arr[q].or(dep[q]).unwrap_or(0.0),
                    (None, None) => 0.0,
                };
                (t, t)
            }
        };
        let a = a.max(last);
        let d = d.max(a);
        last = d;
        out.push((a, d, fixed[i]));
    }
    out
}

/// Fix stop `i`'s time in a trip's profile (minutes after it leaves; the last stop its
/// arrival, the others their departure), or let it follow the distance again (None).
pub fn set_stop_time(p: &mut omsi_timetable::TripProfile, n: usize, i: usize, m: Option<f32>) {
    let k = i as i32;
    p.man_arr_time.retain(|x| x.0 != k);
    p.man_dep_time.retain(|x| x.0 != k);
    if let Some(m) = m {
        if i + 1 == n {
            p.man_arr_time.push((k, m));
            p.man_arr_time.sort_by_key(|x| x.0);
        } else {
            p.man_dep_time.push((k, m));
            p.man_dep_time.sort_by_key(|x| x.0);
        }
    }
}

/// Minutes after midnight as "h:mm" (past midnight: 24:10 …).
pub fn clock(minutes: f32) -> String {
    let m = minutes.round().max(0.0) as i64;
    format!("{}:{:02}", m / 60, m % 60)
}


#[cfg(test)]
pub(crate) mod tests;
