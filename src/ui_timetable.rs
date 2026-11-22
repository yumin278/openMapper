//! The Lines window: the map's bus lines, the trips each runs (the stops in order, picked on
//! the map or from a list, and the way between them found along the roads) and its tours
//! (a bus's day of trips with their departure times). The trip being edited is drawn in the
//! view.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use egui::{Color32, RichText, Sense, Vec2};
use glam::DVec3;
use omsi_timetable::{Line, Tour, TourTrip, Trip};

use crate::doc::{Document, Sel, TileKey};
use crate::editor::{Editor, Tool};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Roots;
use crate::render::FrameExtras;
use crate::timetable::network::{Files, Network};
use crate::timetable::route::{self, Problem, Route, Ways};
use crate::timetable::{self, clock, Timetable, ALL_DAYS};
use crate::ui::Action;
use crate::theme::pal;

mod times;
mod trips;
mod way;

const ROUTE: [f32; 4] = [0.25, 0.75, 1.0, 0.95];

/// How near a click must be to a stop to pick it (m).
const PICK_REACH: f64 = 12.0;

/// An average bus with its stops: what a trip's time is guessed from (km/h).
const BUS_SPEED: f64 = 22.0;

/// A stop of the map: tile, IDCode, its name (the object's label) and where it stands.
#[derive(Clone)]
struct MapStop {
    tile: TileKey,
    id: i64,
    name: String,
    pos: DVec3,
    kind: crate::stops::StopKind,
}

struct ReplaceStopId {
    index: usize,
    old_id: i64,
    input: String,
    all: bool,
}

/// What the window keeps between frames.
#[derive(Default)]
pub struct TimetableUi {
    pub open: bool,
    files: Files,
    net: Option<Network>,
    net_key: u64,
    net_built: Option<std::time::Instant>,
    stops: Vec<MapStop>,
    line: Option<String>,
    trip: Option<String>,
    tour: Option<usize>,
    /// Clicking stops on the map adds them to the trip.
    picking: bool,
    replace_id: Option<ReplaceStopId>,
    insert_before: Option<usize>,
    pending_insert: Option<(usize, i64)>,
    stop_edit_undo: Option<&'static str>,
    line_name: String,
    trip_name: String,
    stop_filter: String,
    /// The Vehicles tab of a line is shown; the vehicles the installation has (read once).
    vehicles_tab: bool,
    roots: Option<Vec<std::path::PathBuf>>,
    fleet: Option<crate::traffic::catalogue::Catalogue>,
    vehicle_filter: String,
    /// The route of the trip shown: (trip, its stops, network key, route).
    shown: Option<(String, Vec<i64>, u64, Route)>,
    /// Trips whose route was found here, with the network it was found on (found again on
    /// saving when the roads changed since).
    routed: HashMap<String, u64>,
    /// What went wrong finding a trip's route.
    problems: HashMap<String, Vec<Problem>>,
    /// What the trips set here drive on (by lower-case name); the others drive on what their
    /// way was found on, or what their stops stand by.
    ways: HashMap<String, Ways>,
    /// The map the state belongs to.
    map: Option<std::path::PathBuf>,
    confirm_type2_conversion: bool,
    want_line: Option<String>,
    /// The line's tours are shown (else its trips).
    tours_tab: bool,
    fill_from: (u32, u32),
    fill_to: (u32, u32),
    layover: u32,
    /// The year the holidays are shown of.
    holiday_year: i32,
    holidays_open: bool,
    /// Bulk rerouting is destructive to shared station links, so require confirmation.
    confirm_recalculate: bool,
    overwrite_hand_routes: bool,
    bulk_report: Option<String>,
    /// Why the last batch recalculation dropped trips (root causes first).
    bulk_conflicts: Vec<ConflictCause>,
    /// Route-only edits need a timetable snapshot for Undo as well.
    bulk_changed: bool,
    /// Trips deliberately left unchanged by a bulk operation: avoid a subsequent
    /// automatic stale-route pass silently bypassing the shared-link safeguards.
    bulk_blocked: HashSet<String>,
    /// Clicking paths in the view puts them into the trip's way (picked by hand).
    way_edit: bool,
    /// Every trip of the map is listed (not a line's).
    all_trips: bool,
    trip_filter: String,
    /// The trip's time profile shown (its place in the list).
    profile: usize,
}

fn type2_conversion_label() -> &'static str {
    tr("Convert all Type 1 trips to Type 2", "Преобразовать все рейсы типа 1 в тип 2")
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Resolve the *saved* Type 1 track exactly as openOMSI would: an explicit
/// display_name does not fall back to the trip name when its file is absent.
/// Keep unmapped entries as None so the preview never silently joins across a
/// deleted road or substitutes a newly found route.
fn type1_track_lanes(
    tt: &Timetable, trip: &Trip, net: &Network, order: &[TileKey],
) -> (String, Option<Vec<Option<usize>>>) {
    let track_name = if trip.display_name.trim().is_empty() {
        trip.name.clone()
    } else {
        trip.display_name.trim().to_string()
    };
    let mapped = tt.track(&track_name).map(|t| timetable::way::lanes_of_track(net, &t.entries, order));
    (track_name, mapped)
}


/// A newly calculated trip track and the station links between its stops.
type PlannedRoute = (String, Vec<omsi_timetable::TrackEntry>, Vec<omsi_timetable::StnLink>);

/// Type-2 stop IDs can be routed using station links; legacy track stop records cannot.
fn eligible_for_batch(trip: &Trip) -> bool {
    trip.stations.len() >= 2 && trip.stations_legacy.is_empty()
}

/// Why a shared station link stopped the batch recalculation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ConflictKind {
    /// The trips that calculated the pair disagree with each other.
    Disagree,
    /// The pair was protected before the batch began (its trip was hand-picked, failed or
    /// is a legacy trip) and the calculated link differs from the map's.
    Protected,
    /// The pair became protected because another trip using it was already in conflict.
    Propagated,
}

impl ConflictKind {
    fn name(self) -> &'static str {
        match self {
            ConflictKind::Disagree => "Disagree",
            ConflictKind::Protected => "Protected",
            ConflictKind::Propagated => "Propagated",
        }
    }

    /// Root causes first, then propagated ones (only used for the report's order).
    fn rank(self) -> u8 {
        match self {
            ConflictKind::Disagree => 0,
            ConflictKind::Protected => 1,
            ConflictKind::Propagated => 2,
        }
    }
}

/// One trip's candidate link for a conflicting pair.
#[derive(Clone, Debug)]
struct ConflictCandidate {
    index: usize,
    name: String,
    link: omsi_timetable::StnLink,
}

impl ConflictCandidate {
    /// The link's numbers, for the log and the report file.
    fn describe(&self) -> String {
        let p = self.link.params;
        format!(
            "{}: length {:.3}, stop A off {:.3} / along {:.3}, stop B off {:.3} / along {:.3}, \
             flag {:.0}, last entry {:.0}, AI path {}",
            self.name, self.link.length, p[0], p[2], p[1], p[3], p[4], p[5], entries_text(&self.link.entries)
        )
    }
}

/// One field two candidate links (or a candidate and the map's link) disagree on.
#[derive(Clone, Debug)]
struct FieldDiff {
    /// Short name for the summary line ("AI path sequence", "length", ...).
    field: &'static str,
    /// Both sides' values, for the log and the report file.
    detail: String,
}

/// Where a propagated conflict came from: the trip already in conflict whose other links
/// protected this pair, and the pair it was itself rejected on.
#[derive(Clone, Debug)]
struct ConflictVia {
    trip: String,
    pair: Option<(i64, i64)>,
    /// The round the source trip was rejected in (the cause here is a round later).
    round: usize,
}

/// Why the batch recalculation dropped a shared station link, and what each trip computed.
#[derive(Clone, Debug)]
struct ConflictCause {
    kind: ConflictKind,
    /// 0: a root cause; later: pulled in by an earlier round's conflict.
    round: usize,
    from_id: i64,
    to_id: i64,
    from_name: String,
    to_name: String,
    /// Every trip that calculated a link for the pair, in planned order.
    trips: Vec<ConflictCandidate>,
    /// The map's link for the pair, when it differs from the candidate links.
    original: Option<omsi_timetable::StnLink>,
    /// What differs, field by field (the first candidate against the others and the original).
    diffs: Vec<FieldDiff>,
    via: Option<ConflictVia>,
    /// The trips this cause first rejected (not already in conflict from another pair).
    new_trips: Vec<usize>,
}

impl ConflictCause {
    /// The fields that differ, once each, for the UI's summary line.
    fn fields(&self) -> String {
        let mut names: Vec<&str> = Vec::new();
        for d in &self.diffs {
            if !names.contains(&d.field) {
                names.push(d.field);
            }
        }
        if names.is_empty() { "(values not compared)".to_string() } else { names.join(", ") }
    }

    /// `Station A(id) → Station B(id) | kind | n trips involved | differing fields`.
    fn summary(&self) -> String {
        format!(
            "{}({}) → {}({}) | {} | {} trips | {}",
            self.from_name, self.from_id, self.to_name, self.to_id, self.kind.name(), self.trips.len(), self.fields()
        )
    }

    /// One log line with both sides of every difference (AI path sequences in full).
    fn log_text(&self) -> String {
        let trips: Vec<&str> = self.trips.iter().map(|t| t.name.as_str()).collect();
        let mut o = format!(
            "round {} {}: {} ({}) -> {} ({}) | trips: {}",
            self.round, self.kind.name(), self.from_name, self.from_id, self.to_name, self.to_id, trips.join(", ")
        );
        if let Some(orig) = &self.original {
            o.push_str(&format!(" | original: length {:.3}, AI path {}", orig.length, entries_text(&orig.entries)));
        }
        for d in &self.diffs {
            o.push_str(&format!(" | {} [{}]", d.field, d.detail));
        }
        if let Some(via) = &self.via {
            o.push_str(&format!(" | propagated from trip {} on {} (round {})", via.trip, pair_text(via.pair), via.round));
        }
        o
    }

    /// Which trip a click in the UI should show: one this cause first rejected, else any.
    fn pick_trip(&self) -> Option<String> {
        self.new_trips.first().and_then(|i| self.trips.iter().find(|t| t.index == *i)).map(|t| t.name.clone())
            .or_else(|| self.trips.first().map(|t| t.name.clone()))
    }
}

fn pair_text(pair: Option<(i64, i64)>) -> String {
    match pair {
        Some((a, b)) => format!("{a} -> {b}"),
        None => "an earlier pair".to_string(),
    }
}

/// An AI path sequence in full: every entry's seven numbers (`id/path/tile/length/-1/0/0`).
fn entries_text(entries: &[omsi_timetable::StnLinkEntry]) -> String {
    let items: Vec<String> = entries.iter().map(|e| e.values.iter().map(|v| format!("{v:.3}")).collect::<Vec<_>>().join("/")).collect();
    format!("[{}]", items.join(", "))
}

/// The fields two links disagree on (the station pair is the same by construction).
fn link_diffs(a: &str, first: &omsi_timetable::StnLink, b: &str, other: &omsi_timetable::StnLink) -> Vec<FieldDiff> {
    let (p, q) = (first.params, other.params);
    let mut out = Vec::new();
    if first.entries != other.entries {
        out.push(FieldDiff {
            field: "AI path sequence",
            detail: format!("{a}: {} | {b}: {}", entries_text(&first.entries), entries_text(&other.entries)),
        });
    }
    if first.length != other.length {
        out.push(FieldDiff { field: "length", detail: format!("{a}: {:.3} | {b}: {:.3}", first.length, other.length) });
    }
    if p[0] != q[0] || p[2] != q[2] {
        out.push(FieldDiff {
            field: "position of stop A on its lane",
            detail: format!("{a}: off {:.3} / along {:.3} | {b}: off {:.3} / along {:.3}", p[0], p[2], q[0], q[2]),
        });
    }
    if p[1] != q[1] || p[3] != q[3] {
        out.push(FieldDiff {
            field: "position of stop B on its lane",
            detail: format!("{a}: off {:.3} / along {:.3} | {b}: off {:.3} / along {:.3}", p[1], p[3], q[1], q[3]),
        });
    }
    if p[4] != q[4] {
        out.push(FieldDiff { field: "other fields", detail: format!("{a}: flag {:.0} | {b}: flag {:.0}", p[4], q[4]) });
    }
    if p[5] != q[5] {
        out.push(FieldDiff { field: "other fields", detail: format!("{a}: last entry {:.0} | {b}: last entry {:.0}", p[5], q[5]) });
    }
    out
}

/// When and by which skipped trip a pair became protected (for the diagnostics only).
struct ProtectedPair {
    round: usize,
    by: Option<usize>,
    root: Option<(i64, i64)>,
}

/// Station links belong to the whole map and are keyed only by their two stop IDs.
/// Do not pick an arbitrary winner when two trips calculate different paths between
/// the same stops. Nor replace a link used by a trip that could not be recalculated.
/// Returns the trips to skip and why, root causes (round 0) first.
fn conflicting_batch_routes(
    planned: &[PlannedRoute],
    original: &[omsi_timetable::StnLink],
    protected: &HashSet<(i64, i64)>,
    names: &HashMap<i64, String>,
) -> (HashSet<usize>, Vec<ConflictCause>) {
    let mut uses: HashMap<(i64, i64), Vec<(usize, &omsi_timetable::StnLink)>> = HashMap::new();
    for (index, (_, _, links)) in planned.iter().enumerate() {
        for link in links {
            uses.entry((link.from_id, link.to_id)).or_default().push((index, link));
        }
    }
    let mut protected = protected.clone();
    let mut origins: HashMap<(i64, i64), ProtectedPair> = protected
        .iter()
        .map(|pair| (*pair, ProtectedPair { round: 0, by: None, root: None }))
        .collect();
    let mut conflicts = HashSet::new();
    // (the pair each trip was first rejected on, for the propagation's provenance)
    let mut entered: Vec<Option<(usize, i64, i64)>> = vec![None; planned.len()];
    let mut causes: Vec<ConflictCause> = Vec::new();
    let mut reported: HashSet<(i64, i64)> = HashSet::new();
    let mut round = 0usize;
    loop {
        let mut added = false;
        let mut this_round: Vec<((i64, i64), Vec<usize>)> = Vec::new();
        for (pair, group) in &uses {
            let first = group[0].1;
            let disagree = group.iter().any(|(_, link)| *link != first);
            let would_change_protected = protected.contains(pair)
                && original.iter().any(|link| (link.from_id, link.to_id) == *pair && link != first);
            if disagree || would_change_protected {
                let mut new_trips = Vec::new();
                for (index, _) in group {
                    if conflicts.insert(*index) {
                        added = true;
                        new_trips.push(*index);
                    }
                }
                if new_trips.is_empty() || !reported.insert(*pair) {
                    continue;
                }
                let origin = origins.get(pair);
                let kind = if disagree {
                    ConflictKind::Disagree
                } else if origin.map(|o| o.round == 0).unwrap_or(false) {
                    ConflictKind::Protected
                } else {
                    ConflictKind::Propagated
                };
                let original_link = original.iter().find(|link| (link.from_id, link.to_id) == *pair);
                let first_name = &planned[group[0].0].0;
                let mut diffs = Vec::new();
                for (index, link) in group.iter().skip(1) {
                    if *link != first {
                        diffs.extend(link_diffs(first_name, first, &planned[*index].0, link));
                    }
                }
                if let Some(orig) = original_link.filter(|link| *link != first) {
                    diffs.extend(link_diffs(first_name, first, "original", orig));
                }
                let via = origin.and_then(|o| {
                    let by = o.by?;
                    Some(ConflictVia {
                        trip: planned.get(by)?.0.clone(),
                        pair: o.root,
                        round: o.round.saturating_sub(1),
                    })
                });
                causes.push(ConflictCause {
                    kind,
                    round,
                    from_id: pair.0,
                    to_id: pair.1,
                    from_name: stop_label(names, pair.0),
                    to_name: stop_label(names, pair.1),
                    trips: group.iter().map(|(index, link)| ConflictCandidate {
                        index: *index,
                        name: planned[*index].0.clone(),
                        link: (*link).clone(),
                    }).collect(),
                    original: original_link.filter(|link| *link != first).cloned(),
                    diffs,
                    via,
                    new_trips: new_trips.clone(),
                });
                this_round.push((*pair, new_trips));
            }
        }
        if !added {
            break;
        }
        // (a fixed order: the report and the log must read the same each run)
        this_round.sort_by_key(|(pair, _)| *pair);
        for (pair, trips) in &this_round {
            for &index in trips {
                entered[index].get_or_insert((round, pair.0, pair.1));
            }
        }
        let mut order: Vec<usize> = conflicts.iter().copied().collect();
        order.sort_unstable();
        // A skipped trip must also retain any other shared links it depends on.
        // Propagate its protection until no additional trip would change those links.
        for index in order {
            for link in &planned[index].2 {
                let pair = (link.from_id, link.to_id);
                protected.insert(pair);
                origins.entry(pair).or_insert(ProtectedPair {
                    round: round + 1,
                    by: Some(index),
                    root: entered[index].map(|(_, a, b)| (a, b)),
                });
            }
        }
        round += 1;
    }
    causes.sort_by(|a, b| (a.round, a.kind.rank(), a.from_id, a.to_id).cmp(&(b.round, b.kind.rank(), b.from_id, b.to_id)));
    (conflicts, causes)
}

fn stop_label(names: &HashMap<i64, String>, id: i64) -> String {
    names.get(&id).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| format!("#{id}"))
}

/// The diagnostics written beside the map after every batch recalculation.
fn conflict_report(map: &std::path::Path, planned: &[PlannedRoute], conflicts: &HashSet<usize>, causes: &[ConflictCause]) -> String {
    let mut o = String::from("openMapper: timetable batch recalculation conflicts\n");
    o.push_str(&format!("map: {}\n", map.display()));
    o.push_str(&format!("planned trips: {} | conflicting trips: {}\n", planned.len(), conflicts.len()));
    o.push_str("(written again by every Recalculate all; root causes first)\n");
    if causes.is_empty() {
        o.push_str("\nNo shared station link conflicted: every planned trip was written.\n");
        return o;
    }
    for (n, c) in causes.iter().enumerate() {
        o.push_str(&format!(
            "\n{}. round {} | {} | station {} ({}) -> {} ({})\n",
            n + 1, c.round, c.kind.name(), c.from_name, c.from_id, c.to_name, c.to_id
        ));
        o.push_str(&format!("   trips involved ({}):\n", c.trips.len()));
        for t in &c.trips {
            o.push_str(&format!("     - {}\n", t.describe()));
        }
        if let Some(orig) = &c.original {
            o.push_str(&format!(
                "   the map's link: length {:.3}, AI path {}\n",
                orig.length, entries_text(&orig.entries)
            ));
        }
        o.push_str("   differing fields:\n");
        if c.diffs.is_empty() {
            o.push_str("     (none recorded)\n");
        }
        for d in &c.diffs {
            o.push_str(&format!("     - {}: {}\n", d.field, d.detail));
        }
        if let Some(via) = &c.via {
            o.push_str(&format!(
                "   propagated from trip {} on {} (round {})\n",
                via.trip, pair_text(via.pair), via.round
            ));
        }
        let first: Vec<&str> = c.trips.iter().filter(|t| c.new_trips.contains(&t.index)).map(|t| t.name.as_str()).collect();
        o.push_str(&format!("   first rejected by this cause: {}\n", first.join(", ")));
    }
    o
}

/// Writes the diagnostics beside the map (overwriting the previous batch's).
fn write_conflict_report(
    dir: &std::path::Path,
    planned: &[PlannedRoute],
    conflicts: &HashSet<usize>,
    causes: &[ConflictCause],
) -> std::io::Result<std::path::PathBuf> {
    let path = dir.join("openmapper_conflicts.txt");
    std::fs::write(&path, conflict_report(dir, planned, conflicts, causes))?;
    Ok(path)
}

/// What trip `name` drives on: as set here, else as its saved way does (None: as its stops
/// say).
fn ways_for(set: &HashMap<String, Ways>, doc: &Document, net: &Network, name: &str) -> Option<Ways> {
    set.get(&lower(name)).copied().or_else(|| route::ways_of(&doc.timetable, name, net, &doc.order))
}

/// A trip's time (minutes): its first profile's.
fn minutes(t: &Trip) -> f32 {
    t.profiles.first().map(|p| p.factor).filter(|f| *f > 0.0).unwrap_or(10.0)
}

/// The trips a line runs: those its tours name and those that show its number.
fn line_trips(tt: &Timetable, line: &Line) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in line.tours.iter().flat_map(|t| t.trips.iter()) {
        if !out.iter().any(|x| x.eq_ignore_ascii_case(&t.trip)) && tt.trip(&t.trip).is_some() {
            out.push(t.trip.clone());
        }
    }
    for t in tt.trips.iter().filter(|t| t.line.trim().eq_ignore_ascii_case(line.name.trim())) {
        if !out.iter().any(|x| x.eq_ignore_ascii_case(&t.name)) {
            out.push(t.name.clone());
        }
    }
    out
}

fn small_button(ui: &mut egui::Ui, icon: Icon, hint: &str) -> bool {
    ui.add(egui::Button::new(icon.text(16.0)).frame(false).min_size(Vec2::new(20.0, 20.0))).on_hover_text(hint).clicked()
}

fn other_trip_uses_id(trip: &Trip, current: &str, id: i64) -> bool {
    !trip.name.eq_ignore_ascii_case(current) && trip.stations_legacy.is_empty() && trip.stations.contains(&id)
}

fn replace_station_id(
    trips: &mut [Trip], current: &str, index: usize, old: i64, new: i64, all: bool,
    old_names: &[&str], new_name: &str,
) -> Vec<(String, Vec<usize>)> {
    if old == new || !trips.iter().any(|t| {
        t.name.eq_ignore_ascii_case(current) && t.stations_legacy.is_empty() && t.stations.get(index) == Some(&old)
    }) {
        return Vec::new();
    }

    let mut changed = Vec::new();
    for trip in trips {
        let indices = if trip.name.eq_ignore_ascii_case(current) {
            trip.stations[index] = new;
            vec![index]
        } else if all && other_trip_uses_id(trip, current, old) {
            trip.stations.iter_mut().enumerate().filter_map(|(i, id)| {
                if *id != old {
                    return None;
                }
                *id = new;
                Some(i)
            }).collect()
        } else {
            continue;
        };
        let sign = trip.terminus.trim();
        if indices.last().is_some_and(|&i| i + 1 == trip.stations.len()) && (sign.is_empty() || old_names.contains(&sign)) {
            trip.terminus = new_name.to_string();
        }
        changed.push((trip.name.clone(), indices));
    }
    changed
}

/// Move profile entries with their old rows, even when a stop ID occurs more than once.
/// An inserted row has no old index and gets its stopping rule from the map.
fn remap_station_profiles(
    profiles: &mut [omsi_timetable::TripProfile], old_len: usize, stations: &[i64],
    old_rows: &[Option<usize>], default_serve: impl Fn(i64) -> Option<i32>,
) {
    let mut new_rows = vec![None; old_len];
    for (new, old) in old_rows.iter().enumerate() {
        if let Some(old) = *old {
            new_rows[old] = Some(new);
        }
    }
    let remap_time = |list: &mut Vec<(i32, f32)>| {
        list.retain_mut(|(i, _)| {
            let Some(new) = new_rows.get(*i as usize).copied().flatten().filter(|k| k + 1 < stations.len()) else { return false };
            *i = new as i32;
            true
        });
        list.sort_by_key(|x| x.0);
    };
    for profile in profiles {
        // The final stop's arrival is represented by the trip's duration.
        profile.man_arr_time.retain(|(i, _)| Some(*i as usize) != old_len.checked_sub(1));
        remap_time(&mut profile.man_arr_time);
        remap_time(&mut profile.man_dep_time);
        let old_serve = std::mem::take(&mut profile.other_stopping);
        profile.other_stopping = old_rows.iter().enumerate().filter_map(|(new, old)| {
            let serve = match old {
                Some(old) => old_serve.iter().find(|(i, _)| *i as usize == *old).map(|(_, v)| *v),
                None => default_serve(stations[new]),
            }?;
            (serve != 0).then_some((new as i32, serve))
        }).collect();
    }
}

fn open_text_file(path: &std::path::Path) -> std::io::Result<()> {
    use std::process::Command;

    #[cfg(target_os = "windows")]
    let mut editor = Command::new("notepad.exe");
    #[cfg(target_os = "macos")]
    let mut editor = {
        let mut cmd = Command::new("open");
        cmd.arg("-t");
        cmd
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut editor = Command::new("xdg-open");

    editor.arg(path).spawn().map(|_| ())
}

/// A check box like the rest of the interface's.
fn check(ui: &mut egui::Ui, on: bool, text: &str) -> bool {
    let galley = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    let size = Vec2::new(24.0 + galley.size().x, galley.size().y.max(20.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let vis = ui.style().interact(&resp);
    let col = if on { pal().accent } else { vis.fg_stroke.color };
    icons::paint(ui.painter(), if on { Icon::CheckOn } else { Icon::CheckOff }, egui::pos2(rect.left() + 9.0, rect.center().y), 20.0, col);
    ui.painter().galley(egui::pos2(rect.left() + 24.0, rect.center().y - galley.size().y * 0.5), galley, vis.text_color());
    resp.clicked()
}

/// A label with an icon in a colour.
fn coloured(mut job: egui::text::LayoutJob, c: Color32) -> egui::text::LayoutJob {
    for s in &mut job.sections {
        s.format.color = c;
    }
    job.wrap.max_width = 420.0;
    job
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).strong().color(pal().text_strong));
}

impl TimetableUi {
    pub fn new() -> TimetableUi {
        TimetableUi { fill_from: (6, 0), fill_to: (20, 0), layover: 5, ..Default::default() }
    }

    /// Open the window on a line (`OPENMAPPER_TIMETABLE`, for checks; empty: the first).
    pub fn open_at(&mut self, line: &str) {
        self.open = true;
        let (l, tab) = line.split_once(':').unwrap_or((line, ""));
        self.want_line = Some(l.to_string());
        self.tours_tab = tab == "tours" || tab == "holidays";
        self.holidays_open = tab == "holidays";
    }

    /// The demo map's line (`OPENMAPPER_DEMO` with `OPENMAPPER_TIMETABLE`): stops on both
    /// sides of its roads, a line there and back through the junction, a tour.
    pub fn demo(&mut self, ed: &mut Editor, roots: Option<&Roots>) {
        let Some(doc) = ed.doc.as_mut() else { return };
        let mut ids = Vec::new();
        for (x, y, name) in [(-150.0, 13.0, "Westend"), (-60.0, 13.0, "Mitte"), (87.0, 150.0, "Nordpark"), (73.0, 140.0, "Nordpark"), (-60.0, 27.0, "Mitte"), (-170.0, 27.0, "Westend")] {
            let id = doc.new_id();
            let k = crate::doc::tile_of(glam::DVec2::new(x, y));
            let org = crate::doc::tile_origin(k);
            let Some(t) = doc.tiles.get_mut(&k) else { continue };
            t.tile.objects.push(omsi_map::MapObject {
                file: "Sceneryobjects\\Generic\\bus_stop.sco".into(),
                id,
                pos: [x - org.x, y - org.y, 0.0],
                flag: 7,
                extra: crate::stops::StopSettings::new_strings(name, Default::default()),
                ..Default::default()
            });
            t.touch_content();
            ids.push(id);
        }
        let tt = &mut doc.timetable;
        let line = tt.add_line("1");
        let there = tt.add_trip(&line, "1_Westend-Nordpark", ids[..3].to_vec());
        let back = tt.add_trip(&line, "1_Nordpark-Westend", ids[3..].to_vec());
        for (t, term) in [(&there, "Nordpark"), (&back, "Westend")] {
            tt.trip_mut(t).unwrap().terminus = term.into();
        }
        // (a time fixed at the middle stop)
        if let Some(p) = tt.trip_mut(&there).and_then(|t| t.profiles.first_mut()) {
            timetable::set_stop_time(p, 3, 1, Some(4.0));
        }
        let l = tt.lines.iter_mut().find(|l| l.name == line).unwrap();
        let mut trips = Vec::new();
        for k in 0..6 {
            trips.push(TourTrip { trip: if k % 2 == 0 { there.clone() } else { back.clone() }, profile: 0, departure: 6.0 * 60.0 + k as f32 * 15.0 });
        }
        l.tours.push(Tour { number: "1".into(), ai_group: String::new(), extra: ALL_DAYS.to_string(), trips });
        add_depot(doc);
        // (and the cars the template's traffic names but does not have)
        if let Some(r) = roots {
            for g in crate::traffic::stock_groups(&r.omsi).into_iter().filter(|g| g.name == "NormalCars") {
                doc.traffic.groups.push(g);
                doc.traffic.touch(crate::traffic::File::AiLists);
            }
        }
        if self.ensure_net(doc, roots) {
            self.reroute(doc);
        }
        for t in [&there, &back] {
            log::info!("demo: trip {t}: {:?}", self.problems.get(&lower(t)).map(|p| format!("{p:?}")).unwrap_or_else(|| "route found".into()));
        }
        self.map = Some(doc.dir.clone());
        self.select_line(&doc.timetable, Some(line));
    }

    /// The car lanes and stops of the map, read again when its tiles changed (at most
    /// once a second while they keep changing).
    fn ensure_net(&mut self, doc: &Document, roots: Option<&Roots>) -> bool {
        if !self.files.has_roots() {
            match roots {
                Some(r) => self.files = Files::new(vec![r.content.clone(), r.omsi.clone()]),
                None => return false,
            }
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (k, t) in &doc.tiles {
            k.hash(&mut h);
            t.content_rev.hash(&mut h);
        }
        doc.order.hash(&mut h);
        let key = h.finish();
        if self.net.is_some() && key == self.net_key {
            return true;
        }
        if self.net.is_some() && self.net_built.map(|t| t.elapsed().as_secs_f32() < 1.0).unwrap_or(false) {
            return true;
        }
        let t0 = std::time::Instant::now();
        let net = Network::build(doc, &mut self.files);
        log::info!("timetable: {} lanes, {} stops ({:.0} ms)", net.lanes.len(), net.stops.len(), t0.elapsed().as_secs_f64() * 1000.0);
        self.stops = net
            .stops
            .iter()
            .map(|&(tile, id, pos)| {
                // (the strings of a stop on a spline are its attachment's)
                let strings: &[String] = doc.object(tile, id).map(|o| o.extra.as_slice()).or_else(|| doc.attachment(tile, id).map(|a| a.strings.as_slice())).unwrap_or(&[]);
                MapStop { tile, id, pos, name: strings.first().cloned().unwrap_or_default(), kind: crate::stops::StopSettings::read(strings).kind }
            })
            .collect();
        self.stops.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
        self.net = Some(net);
        self.net_key = key;
        self.net_built = Some(std::time::Instant::now());
        true
    }

    fn stop_name(&self, tt: &Timetable, id: i64) -> String {
        self.stops.iter().find(|s| s.id == id).map(|s| s.name.clone()).or_else(|| tt.stop_name(id).map(str::to_string)).unwrap_or_else(|| format!("#{id}"))
    }

    /// Find the routes of the trips whose stops changed and keep them in the timetable.
    fn reroute(&mut self, doc: &mut Document) {
        let Some(net) = &self.net else { return };
        let names: Vec<String> = std::mem::take(&mut doc.timetable.reroute).into_iter().filter(|name| !doc.timetable.type2_converted.contains(name)).collect();
        for name in names {
            // An explicit stop edit is a new request to route this trip.
            self.bulk_blocked.remove(&lower(&name));
            let Some(trip) = doc.timetable.trip(&name).cloned() else { continue };
            if timetable::is_type1(&trip) {
                continue;
            }
            if doc.timetable.by_hand(&name) {
                // (paths picked by hand stay: the stops are found along them again)
                if let Some(lanes) = way::saved_lanes(net, doc, &name) {
                    let r = way::keep_by_hand(net, &mut self.files, doc, &name, &lanes);
                    if r.problems.is_empty() {
                        self.problems.remove(&name);
                    } else {
                        self.problems.insert(name.clone(), r.problems);
                    }
                }
                continue;
            }
            if trip.stations.len() < 2 {
                self.problems.remove(&name);
                continue;
            }
            let pos: Vec<Option<DVec3>> = trip.stations.iter().map(|id| timetable::stop_world(doc, &mut self.files, *id)).collect();
            let r = route::find(net, &pos, doc.global.left_hand_traffic, ways_for(&self.ways, doc, net, &name));
            if r.ok() {
                let track = r.track(net, &doc.order);
                let links = r.links(net, &doc.order, &trip.stations);
                doc.timetable.set_route(&trip.name, track, links);
                self.problems.remove(&name);
            } else {
                self.problems.insert(name.clone(), r.problems.clone());
            }
            self.routed.insert(name, self.net_key);
        }
    }

    /// Explicitly reroute all station-link trips, even when their saved .ttr still fits.
    /// The normal stale-trip check cannot be used for this: it prefers an existing .ttr.
    /// Calculate everything first, then avoid replacing shared links inconsistently.
    fn recalculate_all(&mut self, doc: &mut Document, overwrite_hand: bool) -> String {
        let Some(net) = &self.net else {
            return "Road and track network unavailable / 道路與軌道網路無法使用".to_string();
        };
        let stop_positions: HashMap<i64, DVec3> = self.stops.iter().map(|s| (s.id, s.pos)).collect();
        self.bulk_blocked.clear();
        self.bulk_conflicts.clear();
        let mut planned: Vec<PlannedRoute> = Vec::new();
        let mut protected: HashSet<(i64, i64)> = HashSet::new();
        let mut failures = Vec::new();
        let mut skipped = 0;
        let mut kept_hand = 0;
        for trip in &doc.timetable.trips {
            // A legacy [station] trip specifies a track and its own stop records.
            // Never convert it to [station_typ2] as a side effect of this action.
            if !eligible_for_batch(trip) {
                skipped += 1;
                protected.extend(trip.stations.windows(2).map(|p| (p[0], p[1])));
                continue;
            }
            if !overwrite_hand && doc.timetable.by_hand(&trip.name) {
                skipped += 1;
                kept_hand += 1;
                self.bulk_blocked.insert(lower(&trip.name));
                protected.extend(trip.stations.windows(2).map(|p| (p[0], p[1])));
                continue;
            }
            let positions: Vec<Option<DVec3>> = trip.stations.iter()
                .map(|id| stop_positions.get(id).copied())
                .collect();
            let route = route::find(net, &positions, doc.global.left_hand_traffic, ways_for(&self.ways, doc, net, &trip.name));
            if route.ok() {
                planned.push((trip.name.clone(), route.track(net, &doc.order), route.links(net, &doc.order, &trip.stations)));
            } else {
                failures.push(trip.name.clone());
                self.bulk_blocked.insert(lower(&trip.name));
                protected.extend(trip.stations.windows(2).map(|p| (p[0], p[1])));
                self.problems.insert(lower(&trip.name), route.problems.clone());
                log::warn!("timetable: batch route for {} not found: {:?}", trip.name, route.problems);
            }
        }

        let mut stop_names: HashMap<i64, String> = HashMap::new();
        for (_, _, links) in &planned {
            for link in links {
                for id in [link.from_id, link.to_id] {
                    if !stop_names.contains_key(&id) {
                        stop_names.insert(id, self.stop_name(&doc.timetable, id));
                    }
                }
            }
        }
        let (conflicts, causes) = conflicting_batch_routes(&planned, &doc.timetable.links, &protected, &stop_names);
        let conflicting = conflicts.len();
        // (every cause, root causes first: the set is already sorted that way)
        for cause in &causes {
            log::warn!("timetable: conflict {}", cause.log_text());
        }
        let mut cause_of_trip: HashMap<usize, usize> = HashMap::new();
        for (ci, cause) in causes.iter().enumerate() {
            for index in &cause.new_trips {
                cause_of_trip.entry(*index).or_insert(ci);
            }
        }
        let conflict_samples: Vec<String> = planned.iter().enumerate()
            .filter(|(index, _)| conflicts.contains(index))
            .map(|(_, (name, _, _))| name.clone()).take(5).collect();
        // (overwritten every time, so it can be attached to a bug report as it is now)
        let report_path = doc.dir.join("openmapper_conflicts.txt");
        match write_conflict_report(&doc.dir, &planned, &conflicts, &causes) {
            Ok(path) => log::warn!("timetable: conflicts written to {}", path.display()),
            Err(e) => log::warn!("timetable: could not write {}: {e}", report_path.display()),
        }
        let mut completed = 0;
        for (index, (name, track, links)) in planned.into_iter().enumerate() {
            if conflicts.contains(&index) {
                self.bulk_blocked.insert(lower(&name));
                match cause_of_trip.get(&index).map(|ci| &causes[*ci]) {
                    Some(cause) => {
                        let why = if cause.round == 0 { "root cause" } else { "propagated" };
                        log::warn!(
                            "timetable: batch route for {name} skipped: shared station link {} ({}) -> {} ({}) {why}, round {}",
                            cause.from_name, cause.from_id, cause.to_name, cause.to_id, cause.round
                        );
                    }
                    // (cannot happen: every rejected trip enters through a cause)
                    None => log::warn!("timetable: batch route for {name} skipped: conflicting shared station link"),
                }
                continue;
            }
            doc.timetable.set_automatic_route(&name, track, links);
            doc.timetable.reroute.remove(&lower(&name));
            self.problems.remove(&lower(&name));
            self.routed.insert(lower(&name), self.net_key);
            completed += 1;
        }
        if !failures.is_empty() {
            log::warn!("timetable: batch route failures: {}", failures.join(", "));
        }
        self.shown = None;
        self.bulk_changed |= completed > 0;
        self.bulk_conflicts = causes;
        let mut report = format!(
            "Recalculated / 已重算: {completed}  |  Failed / 失敗（保留原檔）: {}  |  Conflicts / 衝突（保留原檔）: {conflicting}  |  Skipped / 略過: {skipped}",
            failures.len(),
        );
        if kept_hand > 0 {
            report.push('\n');
            report.push_str(&tr("Hand-picked routes kept: {count}", "Сохранено путей, выбранных вручную: {count}").replace("{count}", &kept_hand.to_string()));
        }
        if !failures.is_empty() {
            report.push_str(&format!("\nFailed / 失敗: {}", failures.iter().take(5).cloned().collect::<Vec<_>>().join(", ")));
        }
        if !conflict_samples.is_empty() {
            report.push_str(&format!("\nConflicts / 衝突: {}", conflict_samples.join(", ")));
        }
        report
    }

    /// Before the map is saved: the stop list follows the map's stops, and the routes of
    /// the trips made here are found again where the roads changed since.
    pub fn before_save(&mut self, doc: &mut Document, roots: Option<&Roots>) {
        // (a map whose roads changed: the trips over them may run on lanes no longer there)
        let roads_changed = !doc.timetable.trips.is_empty() && doc.tiles.values().any(|t| t.dirty);
        if !(doc.timetable.dirty || roads_changed) || !self.ensure_net(doc, roots) {
            return;
        }
        // (no waiting a second: the save wants the roads as they are)
        self.net_built = None;
        self.ensure_net(doc, roots);
        if let Some(net) = &self.net {
            let on_map: std::collections::HashSet<i64> = doc.tiles.values().flat_map(|t| t.tile.splines.iter().filter(|s| !s.deleted).map(|s| s.id).chain(t.tile.objects.iter().map(|o| o.id))).collect();
            let changed: std::collections::HashSet<usize> = doc.order.iter().enumerate().filter(|(_, k)| doc.tiles.get(k).map(|t| t.dirty).unwrap_or(false)).map(|(i, _)| i).collect();
            for name in route::stale_trips(&doc.timetable, net, &doc.order, &on_map, &changed) {
                if self.bulk_blocked.contains(&lower(&name)) {
                    continue;
                }
                log::info!("timetable: the way of {name} is found again (its roads changed)");
                doc.timetable.reroute.insert(name.to_lowercase());
            }
        }
        for (name, key) in &self.routed {
            if *key != self.net_key && !self.bulk_blocked.contains(name) {
                doc.timetable.reroute.insert(name.clone());
            }
        }
        doc.timetable.reroute.retain(|name| !self.bulk_blocked.contains(name));
        self.reroute(doc);
        let ids: Vec<(TileKey, i64)> = self.stops.iter().map(|s| (s.tile, s.id)).collect();
        let entries = timetable::stop_entries(doc, &ids);
        doc.timetable.sync_stops(entries);
    }

    /// The trip being edited, drawn in the view: its route and its stops (the first one
    /// green, the last one red).
    pub fn overlay(&self, ed: &Editor, cam: &crate::camera::Camera, x: &mut FrameExtras) {
        if !self.open {
            return;
        }
        let (Some(doc), Some(net)) = (&ed.doc, &self.net) else { return };
        let w = (cam.distance * 0.004).clamp(0.08, 6.0) as f32;
        if let Some((name, stations, _, r)) = &self.shown {
            let lift = DVec3::Z * 0.45;
            if let Some(trip) = doc.timetable.trip(name).filter(|t| timetable::is_type1(t)) {
                // Draw each original path individually: a missing TrackEntry must
                // *not* be connected by a made-up straight line or a new search.
                let (_, mapped) = type1_track_lanes(&doc.timetable, trip, net, &doc.order);
                let mut previous: Option<usize> = None;
                let mut after_missing = false;
                for lane in mapped.into_iter().flatten() {
                    match lane {
                        Some(i) => {
                            let pts = &net.lanes[i].pts;
                            for pair in pts.windows(2) {
                                crate::editor::line(&mut x.overlay, cam, pair[0] + lift, pair[1] + lift, w * 1.2, ROUTE);
                            }
                            if after_missing {
                                crate::editor::disc(&mut x.overlay, cam, pts[0] + lift, w * 7.0, [1.0, 0.25, 0.2, 0.95]);
                            }
                            if let Some(p) = previous {
                                if crate::timetable::way::gap(net, p, i) > 0.0 {
                                    let end = *net.lanes[p].pts.last().unwrap();
                                    crate::editor::disc(&mut x.overlay, cam, end + lift, w * 7.0, [1.0, 0.25, 0.2, 0.95]);
                                    crate::editor::disc(&mut x.overlay, cam, pts[0] + lift, w * 7.0, [1.0, 0.25, 0.2, 0.95]);
                                }
                            }
                            previous = Some(i);
                            after_missing = false;
                        }
                        None => {
                            if let Some(p) = previous {
                                let end = *net.lanes[p].pts.last().unwrap();
                                crate::editor::disc(&mut x.overlay, cam, end + lift, w * 7.0, [1.0, 0.25, 0.2, 0.95]);
                            }
                            previous = None;
                            after_missing = true;
                        }
                    }
                }
            } else {
                let pts = r.polyline(net);
                for pair in pts.windows(2) {
                    crate::editor::line(&mut x.overlay, cam, pair[0] + lift, pair[1] + lift, w * 1.2, ROUTE);
                }
                for (k, _) in crate::timetable::way::gaps(net, &r.lanes) {
                    let end = *net.lanes[r.lanes[k]].pts.last().unwrap();
                    crate::editor::disc(&mut x.overlay, cam, end + DVec3::Z * 0.5, w * 7.0, [1.0, 0.25, 0.2, 0.95]);
                }
            }
            let n = stations.len();
            for (i, id) in stations.iter().enumerate() {
                let Some(s) = self.stops.iter().find(|s| s.id == *id) else { continue };
                let c = if i == 0 {
                    [0.35, 0.85, 0.45, 0.95]
                } else if i + 1 == n {
                    [0.95, 0.3, 0.25, 0.95]
                } else {
                    [1.0, 1.0, 1.0, 0.9]
                };
                crate::editor::disc(&mut x.overlay, cam, s.pos + DVec3::Z * 0.3, w * 5.0, c);
            }
            let _ = (name, doc);
        }
        // the stops that can be picked
        if self.picking || self.insert_before.is_some() {
            for s in &self.stops {
                crate::editor::disc(&mut x.overlay, cam, s.pos + DVec3::Z * 0.2, w * 8.0, [0.96, 0.5, 0.19, 0.35]);
            }
        }
    }

    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, roots: Option<&Roots>, actions: &mut Vec<Action>) {
        let Some(dir) = ed.doc.as_ref().map(|d| d.dir.clone()) else {
            self.open = false;
            self.picking = false;
            self.way_edit = false;
            self.clear_stop_edits();
            ed.pick_mode = false;
            return;
        };
        if self.map.as_ref() != Some(&dir) {
            // another map: start afresh
            let (open, want_line, tours_tab, holidays_open) = (self.open, self.want_line.take(), self.tours_tab, self.holidays_open);
            *self = TimetableUi { open, want_line, tours_tab, holidays_open, map: Some(dir), ..TimetableUi::new() };
        }
        if !self.open {
            self.picking = false;
            self.clear_stop_edits();
            self.way_edit = false;
            ed.pick_mode = false;
            self.confirm_type2_conversion = false;
            self.confirm_recalculate = false;
            return;
        }
        self.roots = roots.map(|r| vec![r.content.clone(), r.omsi.clone()]);
        {
            let doc = ed.doc.as_ref().unwrap();
            self.ensure_net(doc, roots);
        }
        self.reroute(ed.doc.as_mut().unwrap());
        if let Some(w) = self.want_line.take() {
            let tt = &ed.doc.as_ref().unwrap().timetable;
            let l = tt.lines.iter().find(|l| l.name.eq_ignore_ascii_case(&w)).or(tt.lines.first()).map(|l| l.name.clone());
            self.select_line(tt, l);
            self.tour = tt.lines.iter().find(|l| Some(&l.name) == self.line.as_ref()).filter(|l| !l.tours.is_empty()).map(|_| 0);
        }
        if (self.replace_id.is_some() || self.insert_before.is_some())
            && !self.all_trips && !self.line.as_ref().is_some_and(|name| ed.doc.as_ref().unwrap().timetable.lines.iter().any(|l| &l.name == name))
        {
            self.clear_stop_edits();
        }
        // (what it was before this frame's edits, for undo)
        let before = ed.doc.as_ref().unwrap().timetable.clone();
        let traffic_before = ed.doc.as_ref().unwrap().traffic.clone();
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.clear_stop_edits();
        }
        self.pick_from_view(ed);
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Bus, tr("Lines and timetable", "Линии и расписание")))
            .id(egui::Id::new("timetable window"))
            .open(&mut open)
            .default_pos([310.0, 70.0])
            .default_size([760.0, 760.0])
            .min_width(560.0)
            .resizable(true)
            .show(ctx, |ui| self.contents(ui, ed, actions));
        self.open = open;
        if !open {
            self.confirm_type2_conversion = false;
            self.confirm_recalculate = false;
            self.picking = false;
            self.clear_stop_edits();
            self.way_edit = false;
        }
        ed.pick_mode = self.picking || self.way_edit || self.insert_before.is_some();
        if ed.pick_mode && ed.tool != Tool::Select {
            ed.set_tool(Tool::Select);
        }
        let doc = ed.doc.as_ref().unwrap();
        if doc.timetable.type2_converted.len() > before.type2_converted.len() {
            let traffic = (traffic_before != doc.traffic).then_some(traffic_before);
            ed.history.push_snapshot(type2_conversion_label(), crate::history::Snapshot { timetable: Some(before), traffic, ..Default::default() });
            self.update_shown(ed);
            return;
        }
        // (a depot added for tours that all name a group changes only the traffic: a step too)
        let timetable = self.bulk_changed || !before.same_content(&doc.timetable);
        self.bulk_changed = false;
        let traffic = (traffic_before != doc.traffic).then_some(traffic_before);
        let label = self.stop_edit_undo.take().unwrap_or_else(|| tr("Edit timetable", "Правка расписания"));
        if timetable || traffic.is_some() {
            ed.history.push_snapshot(label, crate::history::Snapshot { timetable: timetable.then_some(before), traffic, ..Default::default() });
        }
        self.update_shown(ed);
    }

    /// While picking, a click on the map (the editor's pick mode: nothing is selected or
    /// moved) adds the stop under the cursor, or the one nearest it, to the trip.
    /// Inserting queues the stop for the same profile remapping as edits in the stop list.
    fn pick_from_view(&mut self, ed: &mut Editor) {
        ed.pick_mode = self.picking || self.way_edit || self.insert_before.is_some();
        if ed.pick_mode && self.trip.as_deref()
            .and_then(|name| ed.doc.as_ref()?.timetable.trip(name))
            .is_some_and(timetable::is_type1)
        {
            self.picking = false;
            self.way_edit = false;
            self.clear_stop_edits();
            ed.pick_mode = false;
        }
        if !ed.pick_mode {
            ed.picked = None;
            return;
        }
        if ed.tool != Tool::Select {
            ed.set_tool(Tool::Select);
        }
        if self.way_edit {
            self.click_path(ed);
            return;
        }
        let Some((hit, at)) = ed.picked.take() else { return };
        let Some(trip) = self.trip.clone() else { return };
        let hit = match hit {
            Some(Sel::Object(_, id) | Sel::Attached(_, id)) if self.stops.iter().any(|s| s.id == id) => Some(id),
            _ => at.and_then(|p| {
                self.stops.iter().map(|s| (s.id, (s.pos - p).truncate().length())).filter(|(_, d)| *d < PICK_REACH).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(id, _)| id)
            }),
        };
        let Some(id) = hit else {
            ed.toast(tr("No bus stop there: click one of the orange marks", "Здесь нет остановки: кликните по оранжевой метке"));
            return;
        };
        let Some(doc) = ed.doc.as_mut() else { return };
        if let Some(index) = self.insert_before {
            let target = doc.timetable.trip(&trip).filter(|t| t.stations_legacy.is_empty()).and_then(|t| {
                let next = t.stations.get(index)?;
                let previous = index.checked_sub(1).and_then(|i| t.stations.get(i));
                Some(*next == id || previous == Some(&id))
            });
            match target {
                Some(false) => self.pending_insert = Some((index, id)),
                Some(true) => ed.error(tr("That stop is already next to the insertion point.", "Эта остановка уже рядом с местом вставки.")),
                None => self.insert_before = None,
            }
            return;
        }
        let terminus = self.stop_name(&doc.timetable, id);
        if let Some(t) = doc.timetable.trip_mut(&trip) {
            if t.stations.last() == Some(&id) {
                return;
            }
            t.stations.push(id);
            t.terminus = terminus;
        }
        doc.timetable.touch_route(&trip);
    }

    /// The route of the trip being edited, found again when it or the roads changed.
    fn update_shown(&mut self, ed: &Editor) {
        let (Some(doc), Some(net)) = (&ed.doc, &self.net) else { return };
        let Some(trip) = self.trip.as_ref().and_then(|n| doc.timetable.trip(n)) else {
            self.shown = None;
            return;
        };
        if timetable::is_type1(trip) {
            let (_, mapped) = type1_track_lanes(&doc.timetable, trip, net, &doc.order);
            self.shown = mapped.map(|mapped| {
                let ids: Vec<i64> = trip.stations_legacy.iter()
                    .filter_map(|row| row.first().and_then(|v| v.trim().parse::<i64>().ok()))
                    .collect();
                let pos: Vec<Option<DVec3>> = ids.iter()
                    .map(|id| self.stops.iter().find(|s| s.id == *id).map(|s| s.pos))
                    .collect();
                let lanes: Vec<usize> = mapped.into_iter().flatten().collect();
                let route = crate::timetable::way::along(net, &lanes, &pos, doc.global.left_hand_traffic);
                (trip.name.clone(), ids, self.net_key, route)
            });
            return;
        }
        if let Some((n, s, k, _)) = &self.shown {
            if n == &trip.name && s == &trip.stations && *k == self.net_key && !doc.timetable.by_hand(&trip.name) {
                return;
            }
        }
        let pos: Vec<Option<DVec3>> = trip.stations.iter().map(|id| self.stops.iter().find(|s| s.id == *id).map(|s| s.pos)).collect();
        // (a way picked by hand: as saved, read again every frame, being cheap)
        if doc.timetable.by_hand(&trip.name) {
            let lanes = way::saved_lanes(net, doc, &trip.name).unwrap_or_default();
            let r = crate::timetable::way::along(net, &lanes, &pos, doc.global.left_hand_traffic);
            self.shown = Some((trip.name.clone(), trip.stations.clone(), self.net_key, r));
            return;
        }
        let r = if pos.len() >= 2 { route::find(net, &pos, doc.global.left_hand_traffic, ways_for(&self.ways, doc, net, &trip.name)) } else { Route::default() };
        self.shown = Some((trip.name.clone(), trip.stations.clone(), self.net_key, r));
    }

    fn type2_conversion(&mut self, ui: &mut egui::Ui, doc: &mut Document, toasts: &mut Vec<crate::editor::Toast>) {
        let legacy = || doc.timetable.trips.iter().filter(|t| timetable::is_type1(t));
        let total = legacy().count();
        if total == 0 {
            self.confirm_type2_conversion = false;
            return;
        }
        let count = legacy().filter(|t| !doc.timetable.excludes_type2(t)).count();
        let label = type2_conversion_label();
        if ui.add_enabled(count > 0, egui::Button::new(label)).clicked() {
            self.confirm_type2_conversion = true;
        }
        if self.confirm_type2_conversion {
            let modal = egui::Modal::new(egui::Id::new("timetable type 2 conversion")).show(ui.ctx(), |ui| {
                ui.set_max_width(400.0);
                ui.label(tr("Convert the stop lists of {count} Type 1 trips, then save the map. Use route recalculation separately.", "Преобразуйте списки остановок {count} рейсов типа 1, затем сохраните карту. Пересчёт маршрутов запускается отдельно.").replace("{count}", &count.to_string()));
                ui.horizontal(|ui| {
                    if ui.button(tr("Cancel", "Отмена")).clicked() {
                        ui.close();
                    }
                    ui.button(label).clicked()
                }).inner
            });
            if modal.inner {
                self.confirm_type2_conversion = false;
                let (converted, skipped) = doc.timetable.convert_all_to_type2();
                self.shown = None;
                let text = tr("Converted: {converted}. Skipped: {skipped}.", "Преобразовано: {converted}. Пропущено: {skipped}.")
                    .replace("{converted}", &converted.to_string()).replace("{skipped}", &skipped.to_string());
                toasts.push(crate::editor::Toast { text, at: std::time::Instant::now(), error: skipped > total - count });
            } else if modal.should_close() {
                self.confirm_type2_conversion = false;
            }
        }
        ui.separator();
    }

    fn contents(&mut self, ui: &mut egui::Ui, ed: &mut Editor, actions: &mut Vec<Action>) {
        let Some(doc) = ed.doc.as_mut() else { return };
        self.type2_conversion(ui, doc, &mut ed.toasts);
        if self.net.is_none() {
            ui.label(RichText::new(tr("The roads can't be read without the OMSI 2 folder.", "Без папки OMSI 2 дороги не прочитать.")).color(pal().warn));
        }
        if self.stops.len() < 2 {
            ui.label(RichText::new(tr("A line needs at least two bus stops on the map: place them with Game logic > Bus stops, beside a road.", "Линии нужны хотя бы две остановки на карте: поставьте их через Служебное > Остановки, рядом с дорогой.")).color(pal().warn));
        }
        let eligible = doc.timetable.trips.iter()
            .filter(|t| eligible_for_batch(t)).count();
        if eligible > 0 {
            ui.horizontal(|ui| {
                if ui.add_enabled(self.net.is_some(), egui::Button::new(
                    icons::labelled(Icon::Route, "Recalculate all trips / 批次重算")
                )).clicked() {
                    self.confirm_recalculate = true;
                    self.overwrite_hand_routes = false;
                    self.bulk_report = None;
                    self.bulk_conflicts.clear();
                }
                ui.label(format!("Eligible trips / 可重算班次: {eligible}"));
            });
            if self.confirm_recalculate {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.label("Recalculate all station-link trips and replace successful trips' .ttr and shared StnLinks.cfg. Unroutable/conflicting trips retain their data; save the map afterward. / 批次重算所有站間連結路線，成功的班次會更新 .ttr 與共用 StnLinks.cfg；失敗或路徑衝突的班次保留原資料。完成後請存檔。");
                    if doc.timetable.trips.iter().any(|t| eligible_for_batch(t) && doc.timetable.by_hand(&t.name)) {
                        ui.checkbox(&mut self.overwrite_hand_routes, tr("Overwrite hand-picked routes", "Заменить пути, выбранные вручную"));
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Confirm recalculation / 確認重算").clicked() {
                            self.confirm_recalculate = false;
                            let report = self.recalculate_all(doc, self.overwrite_hand_routes);
                            self.bulk_report = Some(report);
                        }
                        if ui.button(tr("Cancel", "Отмена")).clicked() {
                            self.confirm_recalculate = false;
                        }
                    });
                });
            }
            if let Some(report) = &self.bulk_report {
                ui.label(report);
            }
            if !self.bulk_conflicts.is_empty() {
                ui.label(RichText::new(tr("Conflicting segments", "Конфликтующие участки")).strong());
                ui.label(RichText::new(doc.dir.join("openmapper_conflicts.txt").display().to_string()).small().color(pal().text_dim));
                let mut pick = None;
                egui::ScrollArea::vertical().id_salt("tt conflicts").max_height(180.0).show(ui, |ui| {
                    for (ci, cause) in self.bulk_conflicts.iter().enumerate() {
                        let details = cause.diffs.iter().map(|d| format!("{}: {}", d.field, d.detail)).collect::<Vec<_>>().join("\n");
                        let line = egui::Button::new(cause.summary()).frame(false);
                        if ui.add(line).on_hover_text(details).clicked() {
                            pick = Some(ci);
                        }
                    }
                });
                if let Some(ci) = pick {
                    if let Some(name) = self.bulk_conflicts[ci].pick_trip() {
                        // (show the trip where it belongs, so the click shows it)
                        let line = doc.timetable.trip(&name).map(|t| t.line.trim().to_string()).unwrap_or_default();
                        if !line.is_empty() && self.line.as_deref() != Some(line.as_str())
                            && doc.timetable.lines.iter().any(|l| l.name == line) {
                            self.select_line(&doc.timetable, Some(line));
                        }
                        self.select_trip(Some(name));
                    }
                }
            }
            ui.separator();
        }
        ui.horizontal_top(|ui| {
            // the lines
            ui.vertical(|ui| {
                ui.set_width(150.0);
                heading(ui, tr("Lines", "Линии"));
                egui::ScrollArea::vertical().id_salt("tt lines").max_height(ui.available_height() - 34.0).show(ui, |ui| {
                    let mut names: Vec<String> = doc.timetable.lines.iter().map(|l| l.name.clone()).collect();
                    names.sort_by_key(|n| (n.len(), n.to_lowercase()));
                    if names.is_empty() {
                        ui.label(RichText::new(tr("No lines yet.", "Линий пока нет.")).color(pal().text_dim));
                    }
                    for n in names {
                        let on = self.line.as_deref() == Some(&n);
                        if ui.add(egui::Button::selectable(on, icons::labelled(Icon::Bus, &n)).min_size(Vec2::new(140.0, 0.0))).clicked() && !on {
                            self.select_line(&doc.timetable, Some(n));
                        }
                    }
                    ui.add_space(6.0);
                    let all = format!("{} ({})", tr("All trips", "Все рейсы"), doc.timetable.trips.len());
                    if ui.add(egui::Button::selectable(self.all_trips, icons::labelled(Icon::Route, &all)).min_size(Vec2::new(140.0, 0.0))).on_hover_text(tr("Every trip of the map, those no tour drives too", "Все рейсы карты, и те, по которым не ездит ни один выход")).clicked() && !self.all_trips {
                        self.show_all_trips(&doc.timetable);
                    }
                });
                if ui.button(icons::labelled(Icon::Add, tr("New line", "Новая линия"))).clicked() {
                    let n = (1..).map(|i| i.to_string()).find(|n| !doc.timetable.lines.iter().any(|l| l.name.eq_ignore_ascii_case(n))).unwrap();
                    let name = doc.timetable.add_line(&n);
                    self.select_line(&doc.timetable, Some(name));
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                egui::ScrollArea::vertical().id_salt("tt line").show(ui, |ui| {
                    ui.set_min_width(300.0);
                    match self.line.clone() {
                        _ if self.all_trips => self.all_trips_panel(ui, doc, &mut ed.toasts, actions),
                        Some(l) if doc.timetable.lines.iter().any(|x| x.name == l) => self.line_panel(ui, doc, &l, &mut ed.toasts, actions),
                        _ => {
                            self.line = None;
                            ui.add_space(20.0);
                            ui.label(RichText::new(tr("Pick a line on the left, or make a new one.\n\nA line has trips (a way from stop to stop) and tours (one bus's day: which trip it drives when).", "Выберите линию слева или создайте новую.\n\nУ линии есть рейсы (путь от остановки к остановке) и выходы (день одного автобуса: какой рейс и когда)."))
                                .color(pal().text_dim));
                        }
                    }
                });
            });
        });
    }

    fn select_line(&mut self, tt: &Timetable, name: Option<String>) {
        self.line_name = name.clone().unwrap_or_default();
        self.line = name;
        self.tour = None;
        self.picking = false;
        self.all_trips = false;
        let first = self.line.as_ref().and_then(|l| tt.lines.iter().find(|x| &x.name == l)).map(|l| line_trips(tt, l)).and_then(|t| t.into_iter().next());
        self.select_trip(first);
    }

    fn select_trip(&mut self, name: Option<String>) {
        self.trip_name = name.clone().unwrap_or_default();
        self.trip = name;
        self.picking = false;
        self.way_edit = false;
        self.profile = 0;
        self.clear_stop_edits();
    }

    fn clear_stop_edits(&mut self) {
        self.replace_id = None;
        self.insert_before = None;
        self.pending_insert = None;
    }

    fn replace_stop_id_panel(
        &mut self, ui: &mut egui::Ui, tt: &mut Timetable, current: &str, toasts: &mut Vec<crate::editor::Toast>,
    ) -> bool {
        let mut edit = self.replace_id.take().unwrap();
        let other_count = tt.trips.iter().filter(|t| other_trip_uses_id(t, current, edit.old_id)).count();
        let mut replacement = None;
        let mut cancel = false;
        ui.indent("tt replace stop id", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("New stop ID", "Новый ID остановки"));
                ui.add(egui::TextEdit::singleline(&mut edit.input).desired_width(105.0));
                let found = edit.input.trim().parse::<i64>().ok().and_then(|id| self.stops.iter().find(|s| s.id == id));
                if let Some(stop) = found {
                    ui.label(RichText::new(format!("→ {}", stop.name)).color(pal().ok));
                    if let Some(name) = tt.stop_name(stop.id).filter(|n| *n != stop.name) {
                        ui.label(tr("Timetable name: {name}", "Название в расписании: {name}").replace("{name}", name));
                    }
                } else {
                    ui.label(RichText::new(tr("No stop with this ID on the map", "На карте нет остановки с таким ID")).color(pal().warn));
                }
                let valid = found.filter(|s| s.id != edit.old_id);
                if ui.add_enabled(valid.is_some(), egui::Button::new(tr("Confirm", "Подтвердить"))).clicked() {
                    replacement = valid;
                }
                cancel = ui.button(tr("Cancel", "Отмена")).clicked();
            });
            if other_count > 0 {
                let label = tr("Also apply to other trips with this ID ({count})", "Применить также к другим рейсам с этим ID ({count})")
                    .replace("{count}", &other_count.to_string());
                ui.checkbox(&mut edit.all, label);
            }
        });
        let Some(stop) = replacement else {
            if !cancel {
                self.replace_id = Some(edit);
            }
            return false;
        };

        let old = edit.old_id;
        let new = stop.id;
        let old_name = self.stop_name(tt, old);
        let timetable_name = tt.stop_name(old).unwrap_or_default().trim().to_string();
        let old_names = [old_name.trim(), timetable_name.as_str()];
        let affected = replace_station_id(&mut tt.trips, current, edit.index, old, new, edit.all, &old_names, &stop.name);
        for (trip, indices) in &affected {
            tt.touch_route(trip);
            log::info!("timetable: replaced stop ID {old} with {new} ({:?}) in trip {trip:?} at indices {indices:?}", stop.name);
        }
        if !affected.is_empty() {
            let text = tr("Changed stop ID {old} to {new} ({name}) in {count} trip(s)", "ID остановки {old} заменён на {new} ({name}) в рейсах: {count}")
                .replace("{old}", &old.to_string()).replace("{new}", &new.to_string())
                .replace("{count}", &affected.len().to_string()).replace("{name}", &stop.name);
            toasts.push(crate::editor::Toast { text, at: std::time::Instant::now(), error: false });
            self.stop_edit_undo = Some(tr("Change ID", "Заменить ID"));
        }
        !affected.is_empty()
    }

    fn line_panel(&mut self, ui: &mut egui::Ui, doc: &mut Document, line: &str, toasts: &mut Vec<crate::editor::Toast>, actions: &mut Vec<Action>) {
        let toast = |toasts: &mut Vec<crate::editor::Toast>, t: &str| toasts.push(crate::editor::Toast { text: t.to_string(), at: std::time::Instant::now(), error: true });
        // the line itself
        ui.horizontal(|ui| {
            ui.label(tr("Line", "Линия"));
            let r = ui.add(egui::TextEdit::singleline(&mut self.line_name).desired_width(90.0));
            if r.lost_focus() && self.line_name != line {
                match doc.timetable.rename_line(line, &self.line_name) {
                    Some(n) => {
                        self.line = Some(n);
                        self.clear_stop_edits();
                    }
                    None => {
                        toast(toasts, tr("That name is taken or can't be a file name", "Это имя занято или не годится для файла"));
                        self.line_name = line.to_string();
                    }
                }
            }
            if small_button(ui, Icon::Copy, tr("A copy of the line with its trips and tours (a variant to change)", "Копия линии с её рейсами и выходами (вариант для изменений)")) {
                if let Some(n) = doc.timetable.duplicate_line(line) {
                    self.line_name = n.clone();
                    self.line = Some(n);
                    self.select_trip(None);
                    return;
                }
            }
            if small_button(ui, Icon::Delete, tr("Delete the line and its trips", "Удалить линию и её рейсы")) {
                doc.timetable.delete_line(line);
                self.line = None;
                self.select_trip(None);
            }
        });
        let Some(li) = doc.timetable.lines.iter().position(|l| Some(&l.name) == self.line.as_ref()) else { return };
        let line = doc.timetable.lines[li].name.clone();
        let mut user = doc.timetable.lines[li].user_allowed;
        if check(ui, user, tr("Players can drive it", "Игрок может её водить")) {
            user = !user;
            doc.timetable.lines[li].user_allowed = user;
            doc.timetable.touch_line(&line);
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::selectable(!self.tours_tab && !self.vehicles_tab, icons::labelled(Icon::Route, tr("Trips", "Рейсы")))).clicked() {
                self.tours_tab = false;
                self.vehicles_tab = false;
            }
            if ui.add(egui::Button::selectable(self.tours_tab, icons::labelled(Icon::Schedule, tr("Tours and times", "Выходы и время")))).clicked() {
                self.tours_tab = true;
                self.vehicles_tab = false;
                self.picking = false;
                self.clear_stop_edits();
            }
            if ui.add(egui::Button::selectable(self.vehicles_tab, icons::labelled(Icon::Bus, tr("Vehicles", "Транспорт")))).on_hover_text(tr("Which buses drive this line", "Какие автобусы ходят по этой линии")).clicked() {
                self.vehicles_tab = !self.vehicles_tab;
                self.tours_tab = false;
                self.picking = false;
                self.clear_stop_edits();
            }
        });
        ui.separator();
        if self.vehicles_tab {
            self.vehicles_panel(ui, doc, &line);
            return;
        }
        let trips = line_trips(&doc.timetable, &doc.timetable.lines[li]);
        if self.tours_tab {
            ui.label(RichText::new(tr("A tour is one bus's day: the trips it drives, one after the other, and when each leaves.", "Выход — день одного автобуса: рейсы, которые он едет друг за другом, и время отправления каждого.")).small().color(pal().text_dim));
            self.holidays_panel(ui, &mut doc.timetable);
            ui.add_space(4.0);
            self.tours_panel(ui, doc, li, &trips);
            return;
        }
        ui.label(RichText::new(tr("A trip is one way along the line: its stops in order. Most lines have two, there and back.", "Рейс: одна поездка по линии: остановки по порядку. Обычно их два: туда и обратно.")).small().color(pal().text_dim));
        for t in &trips {
            let Some(trip) = doc.timetable.trip(t) else { continue };
            let on = self.trip.as_deref().map(|x| x.eq_ignore_ascii_case(t)).unwrap_or(false);
            let warn = self.problems.contains_key(&lower(t));
            let text = format!("{}  »  {}   ({} {})", t, trip.terminus.trim(), trip.stations.len().max(trip.stations_legacy.len()), tr("stops", "ост."));
            let r = ui.add(egui::Button::selectable(on, icons::labelled(if warn { Icon::Warning } else { Icon::Route }, &text)));
            if r.clicked() && !on {
                self.select_trip(Some(t.clone()));
            }
        }
        ui.horizontal(|ui| {
            if ui.button(icons::labelled(Icon::Add, tr("New trip", "Новый рейс"))).clicked() {
                let name = doc.timetable.add_trip(&line, &format!("{line}_{}", trips.len() + 1), Vec::new());
                self.select_trip(Some(name));
                self.picking = true;
            }
            let can_back = self.trip.as_ref().and_then(|t| doc.timetable.trip(t)).map(|t| t.stations.len() >= 2).unwrap_or(false);
            if ui.add_enabled(can_back, egui::Button::new(icons::labelled(Icon::Swap, tr("Make the way back", "Сделать обратный")))).on_hover_text(tr("A new trip with the same stops the other way round", "Новый рейс с теми же остановками в обратном порядке")).clicked() {
                let src = doc.timetable.trip(self.trip.as_ref().unwrap()).unwrap().clone();
                let mut stops = src.stations.clone();
                stops.reverse();
                let terminus = stops.last().map(|id| self.stop_name(&doc.timetable, *id)).unwrap_or_default();
                let name = doc.timetable.add_trip(&line, &format!("{}_back", src.name), stops);
                if let Some(t) = doc.timetable.trip_mut(&name) {
                    t.profiles = src.profiles.clone();
                    t.terminus = terminus;
                }
                self.select_trip(Some(name));
            }
        });
        if let Some(t) = self.trip.clone() {
            if doc.timetable.trip(&t).is_some() {
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.trip_panel(ui, doc, &line, &t, toasts, actions);
                });
            } else {
                self.select_trip(None);
            }
        }

    }

    fn trip_panel(&mut self, ui: &mut egui::Ui, doc: &mut Document, line: &str, trip: &str, toasts: &mut Vec<crate::editor::Toast>, actions: &mut Vec<Action>) {
        let tt = &mut doc.timetable;
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Trip", "Рейс"));
            let r = ui.add(egui::TextEdit::singleline(&mut self.trip_name).desired_width(140.0));
            if r.lost_focus() && self.trip_name != trip {
                match tt.rename_trip(trip, &self.trip_name) {
                    Some(n) => {
                        if let Some(k) = self.routed.remove(&lower(trip)) {
                            self.routed.insert(lower(&n), k);
                        }
                        if let Some(w) = self.ways.remove(&lower(trip)) {
                            self.ways.insert(lower(&n), w);
                        }
                        self.trip = Some(n);
                        self.clear_stop_edits();
                    }
                    None => {
                        toasts.push(crate::editor::Toast { text: tr("That name is taken or can't be a file name", "Это имя занято или не годится для файла").into(), at: std::time::Instant::now(), error: true });
                        self.trip_name = trip.to_string();
                    }
                }
            }
            if small_button(ui, Icon::Delete, tr("Delete the trip", "Удалить рейс")) {
                tt.delete_trip(trip);
                self.select_trip(None);
            }
            if let Some(t) = self.trip.as_deref().and_then(|name| tt.trip(name)) {
                let response = ui.button(tr("Open in text editor", "Открыть в текстовом редакторе"))
                    .on_hover_text(tr("Open the saved .ttp file", "Открыть сохранённый файл .ttp"));
                if response.clicked() {
                    let result = match timetable::saved_trip_path(&doc.dir, doc.source.as_deref(), doc.fresh, &t.name) {
                        Some(path) => open_text_file(&path).map_err(|e| format!("{}: {} ({e})", tr("Could not open the trip file", "Не удалось открыть файл рейса"), path.display())),
                        None => Err(tr("Save the map to create this trip's .ttp file first.", "Сначала сохраните карту, чтобы создать файл .ttp этого рейса.").into()),
                    };
                    if let Err(text) = result {
                        toasts.push(crate::editor::Toast { text, at: std::time::Instant::now(), error: true });
                    }
                }
            }
        });
        let Some(name) = self.trip.clone() else { return };
        let Some(t) = tt.trip(&name).cloned() else { return };
        ui.horizontal(|ui| {
            ui.label(tr("Destination sign", "Табличка"));
            let mut term = t.terminus.clone();
            if ui.add(egui::TextEdit::singleline(&mut term).desired_width(160.0)).changed() {
                tt.trip_mut(&name).unwrap().terminus = term;
                tt.touch_trip(&name);
            }
            ui.label(tr("Line", "Линия")).on_hover_text(tr("The line number the trip shows; the line of this number lists it among its trips", "Номер линии, который показывает рейс; линия с этим номером показывает его среди своих рейсов"));
            let mut l = t.line.clone();
            if ui.add(egui::TextEdit::singleline(&mut l).desired_width(60.0)).changed() {
                tt.trip_mut(&name).unwrap().line = l;
                tt.touch_trip(&name);
            }
        });
        if timetable::is_type1(&t) {
            self.clear_stop_edits();
            let mut excluded = tt.excludes_type2(&t);
            if ui.checkbox(&mut excluded, tr("Exclude from Type 2 conversion", "Исключить из преобразования в тип 2")).changed() {
                tt.set_type2_exclusion(&t, excluded);
            }
            let track_name = if t.display_name.trim().is_empty() { t.name.clone() } else { t.display_name.trim().to_string() };
            let track = tt.track(&track_name);
            ui.add_space(4.0);
            ui.label(RichText::new(format!("{}: {}.ttr", tr("Original Type 1 track (read-only)", "Исходный путь типа 1 (только просмотр)"), track_name)).color(pal().accent));
            match (track, self.net.as_ref()) {
                (None, _) => {
                    let message = tr("Track not found: {name}.ttr", "Путь не найден: {name}.ttr")
                        .replace("{name}", &track_name);
                    ui.label(coloured(icons::labelled(Icon::Warning, &message), pal().warn));
                }
                (Some(_), None) => {
                    ui.label(RichText::new(tr("Map AI paths are not loaded yet", "Пути ИИ на карте ещё не загружены")).color(pal().text_dim));
                }
                (Some(track), Some(net)) => {
                    let (_, Some(mapped)) = type1_track_lanes(tt, &t, net, &doc.order) else { unreachable!() };
                    let valid = mapped.iter().flatten().count();
                    let total = track.entries.len();
                    let status = tr("Mapped {mapped} of {total} original track entries", "Найдено {mapped} из {total} исходных участков пути")
                        .replace("{mapped}", &valid.to_string()).replace("{total}", &total.to_string());
                    ui.label(status);
                    if let Some(&first) = mapped.iter().flatten().next() {
                        if ui.button(icons::labelled(Icon::Focus, tr("Show start of original track", "Показать начало исходного пути"))).clicked() {
                            actions.push(Action::LookAt(net.lanes[first].pts[0].truncate()));
                        }
                    }
                    let missing = total - valid;
                    if missing > 0 {
                        let warning = tr("{count} original paths are missing from the current map; no replacement route was calculated.", "{count} исходных участков отсутствуют на карте; новый маршрут не рассчитывался.")
                            .replace("{count}", &missing.to_string());
                        ui.label(coloured(icons::labelled(Icon::Warning, &warning), pal().warn));
                    }
                    egui::CollapsingHeader::new(icons::labelled(Icon::Route, &format!("{} ({total})", tr("Paths of the way", "Пути маршрута"))))
                        .id_salt(("type1 track entries", lower(&t.name)))
                        .default_open(missing > 0)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(270.0).show(ui, |ui| {
                                for (i, (entry, lane)) in track.entries.iter().zip(&mapped).enumerate() {
                                    ui.horizontal_wrapped(|ui| {
                                        let (id, path, tile) = match entry.values.as_slice() {
                                            [id, path, tile, ..] => (*id as i64, *path as i64, *tile as i64),
                                            _ => (-1, -1, -1),
                                        };
                                        let label = format!("{:>3}. ID {id}, path {path}, tile {tile} · {:?}", i + 1, entry.values);
                                        ui.label(RichText::new(label).monospace().color(if lane.is_some() { pal().text } else { pal().warn }));
                                        if let Some(l) = lane {
                                            let path = &net.lanes[*l];
                                            if small_button(ui, Icon::Focus, tr("Show on the map", "Показать на карте")) {
                                                actions.push(Action::LookAt(path.point_at(path.length() * 0.5).truncate()));
                                            }
                                        } else if tile >= 0 && doc.order.get(tile as usize).is_some() {
                                            let origin = crate::doc::tile_origin(doc.order[tile as usize]);
                                            if small_button(ui, Icon::Focus, tr("Locate the tile; the missing path's exact position is unknown", "Показать плитку; точное место отсутствующего пути неизвестно")) {
                                                actions.push(Action::LookAt(origin + glam::DVec2::splat(crate::doc::tile_size() * 0.5)));
                                            }
                                        }
                                    });
                                    if i > 0 {
                                        if let (Some(a), Some(b)) = (mapped[i - 1], *lane) {
                                            let gap = crate::timetable::way::gap(net, a, b);
                                            if gap > 0.0 {
                                                ui.horizontal(|ui| {
                                                    ui.label(coloured(icons::labelled(Icon::Warning, &format!("{} {} ({gap:.1} m)", tr("The way breaks after path", "Путь обрывается после участка"), i)), pal().warn));
                                                    if small_button(ui, Icon::Focus, tr("Show on the map", "Показать на карте")) {
                                                        actions.push(Action::LookAt(net.lanes[a].pts.last().unwrap().truncate()));
                                                    }
                                                });
                                            }
                                        }
                                    }
                                }
                            });
                        });
                }
            }
            ui.label(RichText::new(tr("Only the saved TrackEntry paths are shown; the trip is not rerouted or edited.", "Показаны только сохранённые TrackEntry; маршрут не пересчитывается и не изменяется.")).small().color(pal().text_dim));
            return;
        }
        let _ = line;
        // what it drives on: as set here, else as its way was found
        let found = self.shown.as_ref().filter(|x| x.0 == name).and_then(|x| x.3.ways);
        let ways = self.ways.get(&lower(&name)).copied().or(found).unwrap_or(Ways::Road);
        ui.horizontal(|ui| {
            ui.label(tr("Drives on", "Ездит по")).on_hover_text(tr("Found from where its stops stand; set it for a tram whose stops stand by a road as well", "Определяется по тому, у чего стоят остановки; задайте сами для трамвая, если его остановки стоят и у дороги"));
            for (w, label) in [(Ways::Road, tr("Roads (bus)", "Дорогам (автобус)")), (Ways::Rail, tr("Tracks (tram, train)", "Рельсам (трамвай, поезд)"))] {
                if ui.selectable_label(ways == w, label).clicked() && ways != w {
                    self.ways.insert(lower(&name), w);
                    tt.touch_route(&name);
                    self.shown = None;
                }
            }
        });

        // the stops
        ui.add_space(4.0);
        let mut stations = t.stations.clone();
        let mut old_rows: Vec<_> = (0..stations.len()).map(Some).collect();
        let mut changed = false;
        let n = stations.len();
        let mut fly = None;
        if self.insert_before.is_some_and(|i| i >= n) {
            self.clear_stop_edits();
        }
        if self.replace_id.as_ref().is_some_and(|edit| {
            stations.get(edit.index) != Some(&edit.old_id) || self.stops.iter().any(|s| s.id == edit.old_id)
        }) {
            self.replace_id = None;
        }
        for i in 0..n {
            let id = stations[i];
            ui.horizontal(|ui| {
                let inserting = self.insert_before == Some(i);
                let dot = if inserting { pal().accent } else if i == 0 { pal().ok } else if i + 1 == n { pal().error } else { pal().text };
                ui.label(RichText::new(format!("{:>2}.", i + 1)).color(dot).monospace());
                let known = self.stops.iter().any(|s| s.id == id);
                let mut name = self.stop_name(tt, id);
                if !known {
                    name = format!("{name} ({})", tr("not on the map", "нет на карте"));
                }
                let label = RichText::new(name).color(if inserting { pal().accent } else if known { pal().text_strong } else { pal().warn });
                let label = if inserting { label.strong().background_color(ui.visuals().selection.bg_fill) } else { label };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_button(ui, Icon::Delete, tr("Take out", "Убрать")) {
                        stations.remove(i);
                        old_rows.remove(i);
                        changed = true;
                    } else if i + 1 < n && small_button(ui, Icon::Lower, tr("Move down", "Ниже")) {
                        stations.swap(i, i + 1);
                        old_rows.swap(i, i + 1);
                        changed = true;
                    } else if i > 0 && small_button(ui, Icon::Raise, tr("Move up", "Выше")) {
                        stations.swap(i, i - 1);
                        old_rows.swap(i, i - 1);
                        changed = true;
                    } else if known && small_button(ui, Icon::Focus, tr("Show on the map", "Показать на карте")) {
                        fly = Some(id);
                    } else if small_button(ui, Icon::Add, tr("Insert before: click a stop on the map to insert it before this one", "Вставить перед: выберите остановку на карте, чтобы вставить её перед этой")) {
                        self.clear_stop_edits();
                        self.picking = false;
                        self.way_edit = false;
                        self.insert_before = Some(i);
                    } else if !known && small_button(ui, Icon::Swap, tr("Change ID", "Заменить ID")) {
                        self.clear_stop_edits();
                        self.picking = false;
                        self.way_edit = false;
                        self.replace_id = Some(ReplaceStopId { index: i, old_id: id, input: String::new(), all: false });
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(label).truncate()).on_hover_text(format!("ID {id}"));
                    });
                });
            });
            if changed {
                self.clear_stop_edits();
                break;
            }
            if self.replace_id.as_ref().is_some_and(|edit| edit.index == i) && self.replace_stop_id_panel(ui, tt, &name, toasts) {
                return;
            }
        }
        if let Some(id) = fly {
            if let Some(s) = self.stops.iter().find(|s| s.id == id) {
                actions.push(Action::LookAt(s.pos.truncate()));
            }
        }
        if n == 0 {
            ui.label(RichText::new(tr("No stops yet.", "Остановок пока нет.")).color(pal().text_dim));
        }
        ui.horizontal(|ui| {
            let label = if self.picking { tr("Done picking", "Готово") } else { tr("Pick stops on the map", "Выбрать на карте") };
            if ui.add(egui::Button::selectable(self.picking, icons::labelled(Icon::Select, label))).on_hover_text(tr("Click the stops in the view one after the other, in the order the bus calls at them", "Кликайте остановки в окне карты одну за другой, в том порядке, в каком их проезжает автобус")).clicked() {
                self.clear_stop_edits();
                self.way_edit = false;
                self.picking = !self.picking;
            }
            let mut add = None;
            egui::ComboBox::from_id_salt("tt add stop").selected_text(tr("Add stop…", "Добавить…")).width(150.0).height(320.0).show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.stop_filter).hint_text(tr("Search", "Поиск")).desired_width(170.0));
                let f = self.stop_filter.to_lowercase();
                for s in self.stops.iter().filter(|s| f.is_empty() || s.name.to_lowercase().contains(&f)) {
                    if ui.selectable_label(false, format!("{}  ·  {}", s.name, s.id)).clicked() {
                        add = Some(s.id);
                    }
                }
            });
            if let Some(id) = add {
                self.clear_stop_edits();
                stations.push(id);
                old_rows.push(None);
                changed = true;
            }
        });
        if let Some((index, id)) = self.pending_insert.take() {
            stations.insert(index, id);
            old_rows.insert(index, None);
            changed = true;
            self.insert_before = None;
            self.stop_edit_undo = Some(tr("Insert stop before", "Вставить остановку перед"));
            let stop = self.stop_name(tt, id);
            log::info!("timetable: inserted stop ID {id} ({stop:?}) in trip {name:?} at index {index}");
        }
        if self.picking {
            ui.label(RichText::new(tr("Click stops on the map (they are marked orange)…", "Кликайте остановки на карте (отмечены оранжевым)…")).color(pal().accent));
        } else if let Some(index) = self.insert_before {
            let stop = self.stop_name(tt, stations[index]);
            let hint = tr("Click a stop on the map to insert it before {stop} (Esc to cancel)", "Выберите остановку на карте, чтобы вставить её перед «{stop}» (Esc — отмена)")
                .replace("{stop}", &stop);
            ui.label(RichText::new(hint).color(pal().accent));
        }
        if changed {
            let terminus = stations.last().map(|id| self.stop_name(tt, *id));
            let tm = tt.trip_mut(&name).unwrap();
            // (the sign follows the last stop while it showed the last stop's name)
            if let Some(term) = terminus {
                let old_last = t.stations.last().map(|id| self.stop_name_raw(id));
                if tm.terminus.trim().is_empty() || old_last.as_deref() == Some(tm.terminus.trim()) {
                    tm.terminus = term;
                }
            }
            remap_station_profiles(&mut tm.profiles, tm.stations.len(), &stations, &old_rows, |id| {
                self.stops.iter().find(|s| s.id == id).map(|s| s.kind.serve())
            });
            tm.stations.clone_from(&stations);
            tt.touch_route(&name);
        }

        // the route
        ui.add_space(4.0);
        let shown = self.shown.as_ref().filter(|(n, s, _, _)| n == &name && s == &stations).map(|x| x.3.clone());
        let shown = shown.as_ref();
        match shown {
            _ if stations.len() < 2 => {
                ui.label(RichText::new(tr("Add two stops or more: the way between them is found along the roads.", "Добавьте две остановки или больше: путь между ними найдётся по дорогам.")).color(pal().text_dim));
            }
            None => {
                ui.label(RichText::new(tr("Finding the way…", "Ищем путь…")).color(pal().text_dim));
            }
            Some(r) if r.ok() => {
                let along = if r.ways == Some(Ways::Rail) { tr("km along the tracks (blue in the view)", "км по рельсам (синим на карте)") } else { tr("km along the roads (blue in the view)", "км по дорогам (синим на карте)") };
                let what = if r.by_hand { tr("Way picked by hand:", "Путь выбран вручную:") } else { tr("Route found:", "Путь найден:") };
                ui.label(RichText::new(format!("{} {:.2} {}", what, r.length() / 1000.0, along)).color(pal().ok));
                for &leg in &r.against {
                    let name_of = |i: usize| stations.get(i).map(|id| self.stop_name(tt, *id)).unwrap_or_default();
                    let text = format!("«{}» – «{}»: {}", name_of(leg), name_of(leg + 1), tr("the way drives a one-way lane against its direction; is a path drawn the wrong way round?", "путь идёт по односторонней полосе против её направления; может, путь нарисован не в ту сторону?"));
                    ui.label(coloured(icons::labelled(Icon::Warning, &text), pal().warn));
                }
            }
            Some(r) => {
                let rail = r.ways == Some(Ways::Rail);
                for p in &r.problems {
                    let name_of = |i: usize| stations.get(i).map(|id| self.stop_name(tt, *id)).unwrap_or_default();
                    let text = match p {
                        Problem::NoObject(i) => format!("{} «{}» {}", tr("Stop", "Остановка"), name_of(*i), tr("is not on the map", "нет на карте")),
                        Problem::OffWay(i) => format!("«{}»: {}", name_of(*i), tr("the paths picked do not pass this stop", "выбранные пути не проходят у этой остановки")),
                        Problem::NoRoad(i) if rail => format!("«{}»: {}", name_of(*i), tr("no track within 15 m", "в 15 м нет рельсов")),
                        Problem::NoRoad(i) => format!("«{}»: {}", name_of(*i), tr("no road with car lanes within 15 m", "в 15 м нет дороги с полосами для машин")),
                        Problem::NoWay(i) if rail => format!("{} «{}» – «{}»: {}", tr("No way", "Нет пути"), name_of(*i), name_of(*i + 1), tr("the tracks are not joined, or run the other way (see View > AI lanes)", "рельсы не соединены или идут в другую сторону (Вид > Полосы ИИ)")),
                        Problem::NoWay(i) => format!("{} «{}» – «{}»: {}", tr("No way", "Нет пути"), name_of(*i), name_of(*i + 1), tr("the roads are not joined, or run the other way (see View > AI lanes)", "дороги не соединены или идут в другую сторону (Вид > Полосы ИИ)")),
                    };
                    ui.label(coloured(icons::labelled(Icon::Warning, &text), pal().warn)).on_hover_text(tr("The trip is saved, but its way is not until this is mended", "Рейс сохранится, а его путь: когда это будет исправлено"));
                }
            }
        }

        // the way, picked by hand or found
        self.way_section(ui, doc, &name, actions);

        // the times and how the stops are served
        let legs: Vec<f64> = match shown.filter(|r| r.ok() && r.legs.len() + 1 == stations.len()) {
            Some(r) => r.legs.clone(),
            None => stations.windows(2).map(|w| doc.timetable.links.iter().find(|l| l.from_id == w[0] && l.to_id == w[1]).map(|l| l.length).unwrap_or(0.0)).collect(),
        };
        let route_len = shown.filter(|r| r.ok()).map(|r| r.length());
        self.times_section(ui, &mut doc.timetable, &name, &stations, &legs, route_len);
        self.serve_section(ui, &mut doc.timetable, &name, &stations);
    }

    /// Which vehicles drive the line (`car_use/*.ocu`): for a span of dates the types its tours
    /// are given (only these, or these with a chance), and a fleet number fixed for a tour.
    fn vehicles_panel(&mut self, ui: &mut egui::Ui, doc: &mut Document, line: &str) {
        use crate::timetable::caruse::{date, TypeMode};
        ui.label(RichText::new(tr("The AI buses of this line's tours are taken from the depot group; here the vehicles can be limited by date: only some types, mostly some types, or a fleet number for one tour. A line with nothing here takes any vehicle of the group.", "ИИ-автобусы выходов этой линии берутся из группы депо; здесь транспорт можно ограничить по датам: только некоторые типы, преимущественно некоторые типы или парковый номер для одного выхода. Линия без записей берёт любой транспорт группы.")).small().color(pal().text_dim));
        if self.fleet.is_none() {
            if let Some(roots) = self.roots.clone() {
                self.fleet = Some(crate::traffic::catalogue::Catalogue::scan(&roots));
            }
        }
        let tours: Vec<String> = doc.timetable.lines.iter().find(|l| l.name == line).map(|l| l.tours.iter().map(|t| t.number.clone()).collect()).unwrap_or_default();
        let mut remove = None;
        let mut changed: Vec<String> = Vec::new();
        let n = doc.timetable.car_use.len();
        for i in 0..n {
            if !doc.timetable.car_use[i].use_.line.trim().eq_ignore_ascii_case(line.trim()) {
                continue;
            }
            let mut c = doc.timetable.car_use[i].clone();
            ui.add_space(4.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    crate::ui::long_label(ui, &c.file, egui::TextStyle::Body, ui.visuals().text_color());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if small_button(ui, Icon::Delete, tr("Delete this assignment", "Удалить это назначение")) {
                            remove = Some(i);
                        }
                    });
                });
                // the dates
                ui.horizontal(|ui| {
                    ui.label(tr("From", "С"));
                    let mut a = (c.use_.valid.0 / 10_000, c.use_.valid.0 / 100 % 100, c.use_.valid.0 % 100);
                    ui.add(egui::DragValue::new(&mut a.2).range(1..=31));
                    ui.add(egui::DragValue::new(&mut a.1).range(1..=12));
                    ui.add(egui::DragValue::new(&mut a.0).range(1900..=2100));
                    c.use_.valid.0 = date(a.0, a.1, a.2);
                    ui.label(tr("to", "по"));
                    let mut b = (c.use_.valid.1 / 10_000, c.use_.valid.1 / 100 % 100, c.use_.valid.1 % 100);
                    ui.add(egui::DragValue::new(&mut b.2).range(1..=31));
                    ui.add(egui::DragValue::new(&mut b.1).range(1..=12));
                    ui.add(egui::DragValue::new(&mut b.0).range(1900..=2100));
                    c.use_.valid.1 = date(b.0, b.1, b.2);
                    ui.label(RichText::new(tr("(day, month, year)", "(день, месяц, год)")).small().color(pal().text_dim));
                });
                if c.use_.valid.0 > c.use_.valid.1 {
                    ui.label(RichText::new(tr("It ends before it starts: the game ignores it", "Оно кончается раньше начала: игра его игнорирует")).small().color(pal().warn));
                }
                // the types
                let mut mode = c.mode();
                let mut types = c.types();
                let mut chance = c.use_.types_prefered.as_ref().map(|t| t.0).unwrap_or(0.9);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut mode, TypeMode::Only, tr("Only these types", "Только эти типы"));
                    ui.selectable_value(&mut mode, TypeMode::Preferred, tr("Mostly these types", "Преимущественно эти типы"));
                });
                if mode == TypeMode::Preferred {
                    ui.add(egui::Slider::new(&mut chance, 0.0..=1.0).text(tr("share of the tours", "доля выходов")).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)));
                }
                let mut drop = None;
                for (k, t) in types.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let name = self.fleet.as_ref().map(|f| f.name_of(t).to_string()).unwrap_or_else(|| t.clone());
                        ui.add(egui::Label::new(name).truncate()).on_hover_text(t);
                        if small_button(ui, Icon::Delete, tr("Take out", "Убрать")) {
                            drop = Some(k);
                        }
                    });
                }
                if let Some(k) = drop {
                    types.remove(k);
                }
                let mut add = None;
                if let Some(f) = &self.fleet {
                    egui::ComboBox::from_id_salt(("car use add", i)).selected_text(tr("Add a vehicle…", "Добавить транспорт…")).width(240.0).height(320.0).show_ui(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.vehicle_filter).hint_text(tr("Search", "Поиск")).desired_width(220.0));
                        let q = self.vehicle_filter.to_lowercase();
                        for v in f.vehicles.iter().filter(|v| q.is_empty() || v.name.to_lowercase().contains(&q) || v.rel.to_lowercase().contains(&q)).take(200) {
                            if ui.selectable_label(false, &v.name).on_hover_text(&v.rel).clicked() {
                                add = Some(v.rel.clone());
                            }
                        }
                    });
                }
                if let Some(v) = add {
                    if !types.iter().any(|t| t.eq_ignore_ascii_case(&v)) {
                        types.push(v);
                    }
                }
                c.set_types(mode, types, chance);
                // the fleet numbers of single tours
                ui.add_space(2.0);
                ui.label(RichText::new(tr("Fleet numbers of single tours", "Парковые номера отдельных выходов")).small().color(pal().text_dim));
                let mut drop = None;
                for (k, (num, tour)) in c.use_.number_tour.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(num).desired_width(60.0));
                        ui.label(crate::icons::Icon::ChevronRight.text(14.0));
                        egui::ComboBox::from_id_salt(("car use tour", i, k)).selected_text(tour.as_str()).width(110.0).show_ui(ui, |ui| {
                            for t in &tours {
                                ui.selectable_value(tour, t.clone(), t);
                            }
                        });
                        if small_button(ui, Icon::Delete, tr("Take out", "Убрать")) {
                            drop = Some(k);
                        }
                    });
                }
                if let Some(k) = drop {
                    c.use_.number_tour.remove(k);
                }
                if !tours.is_empty() && ui.small_button(icons::labelled(Icon::Add, tr("A fleet number for a tour", "Парковый номер для выхода"))).clicked() {
                    c.use_.number_tour.push((String::new(), tours[0].clone()));
                }
            });
            if c != doc.timetable.car_use[i] {
                changed.push(c.file.clone());
                doc.timetable.car_use[i] = c;
            }
        }
        for f in changed {
            doc.timetable.touch_car_use(&f);
        }
        if let Some(i) = remove {
            doc.timetable.remove_car_use(i);
        }
        if ui.button(icons::labelled(Icon::Add, tr("A vehicle assignment", "Назначение транспорта"))).clicked() {
            doc.timetable.add_car_use(line);
        }
    }

    /// The stop's name on the map (without the timetable's).
    fn stop_name_raw(&self, id: &i64) -> String {
        self.stops.iter().find(|s| s.id == *id).map(|s| s.name.trim().to_string()).unwrap_or_default()
    }

    /// The public and school holidays (`Holidays.txt`): a tour's Holidays box and its
    /// school day / school holiday choice look them up.
    fn holidays_panel(&mut self, ui: &mut egui::Ui, tt: &mut Timetable) {
        let before = tt.calendar.clone();
        egui::CollapsingHeader::new(icons::labelled(Icon::Schedule, tr("Holidays and school holidays", "Праздники и каникулы"))).id_salt("tt holidays").default_open(self.holidays_open).show(ui, |ui| {
            let c = &mut tt.calendar;
            let mut years: Vec<i32> = c.ranges.iter().map(|r| r.start / 10_000).chain(c.holidays.iter().map(|h| h.date / 10_000)).collect();
            years.sort();
            years.dedup();
            if self.holiday_year == 0 {
                let now = crate::doc::today_code() / 10_000;
                self.holiday_year = years.iter().copied().find(|y| *y == now).or(years.last().copied()).unwrap_or(now);
            }
            let y = &mut self.holiday_year;
            ui.label(RichText::new(tr("On a public holiday only the tours with Holidays ticked run; school holidays pick the tours for school holidays or school days.", "В праздник ходят только выходы с отметкой «Праздники»; каникулы выбирают выходы для каникул или учебных дней.")).small().color(pal().text_dim));
            ui.horizontal(|ui| {
                ui.label(tr("Year", "Год"));
                ui.add(egui::DragValue::new(y).range(1900..=2200));
                let cur = *y;
                for k in years.iter().filter(|k| **k != cur).take(8) {
                    if ui.small_button(k.to_string()).clicked() {
                        *y = *k;
                    }
                }
            });
            let year = *y;
            let date_edit = |ui: &mut egui::Ui, d: &mut i32| -> bool {
                let (mut yy, mut mm, mut dd) = (*d / 10_000, *d / 100 % 100, *d % 100);
                let mut ch = false;
                ui.spacing_mut().item_spacing.x = 2.0;
                ch |= ui.add(egui::DragValue::new(&mut dd).range(1..=31).custom_formatter(|v, _| format!("{:02}", v as i32))).changed();
                ui.label(".");
                ch |= ui.add(egui::DragValue::new(&mut mm).range(1..=12).custom_formatter(|v, _| format!("{:02}", v as i32))).changed();
                ui.label(".");
                ch |= ui.add(egui::DragValue::new(&mut yy).range(1900..=2200)).changed();
                ui.spacing_mut().item_spacing.x = 8.0;
                if ch {
                    *d = yy * 10_000 + mm * 100 + dd;
                }
                ch
            };
            ui.label(RichText::new(tr("Public holidays", "Праздники")).strong());
            let mut remove = None;
            egui::Grid::new("tt public holidays").num_columns(3).spacing(Vec2::new(8.0, 3.0)).show(ui, |ui| {
                for (i, h) in c.holidays.iter_mut().enumerate().filter(|(_, h)| h.date / 10_000 == year) {
                    ui.horizontal(|ui| {
                        date_edit(ui, &mut h.date);
                    });
                    ui.add_sized([160.0, 20.0], egui::TextEdit::singleline(&mut h.name));
                    if small_button(ui, Icon::Delete, tr("Take out", "Убрать")) {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
            if let Some(i) = remove {
                c.holidays.remove(i);
            }
            if ui.button(icons::labelled(Icon::Add, tr("Holiday", "Праздник"))).clicked() {
                c.holidays.push(omsi_map::Holiday { date: year * 10_000 + 101, name: tr("Holiday", "Праздник").into() });
            }
            ui.add_space(4.0);
            ui.label(RichText::new(tr("School holidays", "Каникулы")).strong());
            let mut remove = None;
            egui::Grid::new("tt school holidays").num_columns(4).spacing(Vec2::new(8.0, 3.0)).show(ui, |ui| {
                for (i, r) in c.ranges.iter_mut().enumerate().filter(|(_, r)| r.start / 10_000 == year) {
                    ui.horizontal(|ui| {
                        date_edit(ui, &mut r.start);
                        ui.label("–");
                        date_edit(ui, &mut r.end);
                    });
                    ui.add_sized([120.0, 20.0], egui::TextEdit::singleline(&mut r.name));
                    if small_button(ui, Icon::Delete, tr("Take out", "Убрать")) {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
            if let Some(i) = remove {
                c.ranges.remove(i);
            }
            ui.horizontal(|ui| {
                if ui.button(icons::labelled(Icon::Add, tr("School holidays", "Каникулы"))).clicked() {
                    c.ranges.push(omsi_map::HolidayRange { start: year * 10_000 + 720, end: year * 10_000 + 831, name: tr("Summer", "Лето").into() });
                }
                if ui.button(icons::labelled(Icon::Copy, &format!("{} {}", tr("Copy to", "Копировать в"), year + 1))).on_hover_text(tr("The same days a year later (feasts that move, like Easter, are to be set by hand)", "Те же дни годом позже (переходящие, как Пасха, поправьте вручную)")).clicked() {
                    let next = |d: i32| d + 10_000;
                    let add_h: Vec<_> = c.holidays.iter().filter(|h| h.date / 10_000 == year && !c.holidays.iter().any(|x| x.date == next(h.date))).map(|h| omsi_map::Holiday { date: next(h.date), name: h.name.clone() }).collect();
                    let add_r: Vec<_> = c.ranges.iter().filter(|r| r.start / 10_000 == year && !c.ranges.iter().any(|x| x.start == next(r.start))).map(|r| omsi_map::HolidayRange { start: next(r.start), end: next(r.end), name: r.name.clone() }).collect();
                    c.holidays.extend(add_h);
                    c.ranges.extend(add_r);
                    self.holiday_year = year + 1;
                }
            });
        });
        if tt.calendar != before {
            tt.touch_calendar();
        }
    }

    fn tours_panel(&mut self, ui: &mut egui::Ui, doc: &mut Document, li: usize, trips: &[String]) {
        let depots: Vec<String> = doc.traffic.depots().map(|d| d.name.clone()).collect();
        if depots.is_empty() {
            ui.label(RichText::new(tr("The map has no depot (ailists.cfg): only you drive these tours, no AI buses.", "У карты нет депо (ailists.cfg): по выходам ездите только вы, автобусов ИИ не будет.")).small().color(pal().warn));
            if self.files.exists(crate::traffic::DEPOT_BUS) && ui.button(icons::labelled(Icon::Bus, tr("Add a depot with OMSI 2's MAN buses", "Добавить депо с автобусами MAN из OMSI 2"))).on_hover_text(tr("Its buses run the tours (Map > Traffic and AI shows it)", "Его автобусы ездят по выходам (Карта > Трафик и ИИ)")).clicked() {
                add_depot(doc);
            }
        }
        let tt = &mut doc.timetable;
        let line = tt.lines[li].name.clone();
        let n = tt.lines[li].tours.len();
        ui.horizontal_wrapped(|ui| {
            for i in 0..n {
                let t = &tt.lines[li].tours[i];
                let text = format!("{} {}", tr("Tour", "Выход"), t.number);
                if ui.add(egui::Button::selectable(self.tour == Some(i), text)).clicked() {
                    self.tour = Some(i);
                }
            }
            if ui.button(icons::labelled(Icon::Add, tr("New tour", "Новый выход"))).clicked() {
                let number = tt.lines[li].tours.iter().filter_map(|t| t.number.trim().parse::<i64>().ok()).max().unwrap_or(0) + 1;
                tt.lines[li].tours.push(Tour { number: number.to_string(), ai_group: depots.first().cloned().unwrap_or_default(), extra: ALL_DAYS.to_string(), trips: Vec::new() });
                self.tour = Some(n);
                tt.touch_line(&line);
            }
        });
        let Some(ti) = self.tour.filter(|i| *i < n) else {
            if n > 0 {
                ui.label(RichText::new(tr("Pick a tour to see its departures.", "Выберите выход, чтобы увидеть отправления.")).color(pal().text_dim));
            }
            return;
        };
        if trips.is_empty() {
            ui.label(RichText::new(tr("Make a trip first.", "Сначала создайте рейс.")).color(pal().warn));
            return;
        }
        // trip times, for the arrivals and the next departure
        let times: HashMap<String, f32> = trips.iter().filter_map(|t| tt.trip(t).map(|x| (lower(t), minutes(x)))).collect();
        // (every trip's profiles: a departure can run any trip of the map, to any of its profiles)
        let profiles: HashMap<String, Vec<(String, f32)>> = tt.trips.iter().map(|t| (lower(&t.name), t.profiles.iter().map(|p| (p.name.trim().to_string(), if p.factor > 0.0 { p.factor } else { 10.0 })).collect())).collect();
        let others: Vec<String> = {
            let mut v: Vec<String> = tt.trips.iter().map(|t| t.name.clone()).filter(|n| !trips.iter().any(|x| x.eq_ignore_ascii_case(n))).collect();
            v.sort_by_key(|n| n.to_lowercase());
            v
        };
        let mut changed = false;
        let mut resort = false;
        let mut delete_tour = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let tour = &mut tt.lines[li].tours[ti];
            ui.horizontal(|ui| {
                ui.label(tr("Number", "Номер"));
                changed |= ui.add(egui::TextEdit::singleline(&mut tour.number).desired_width(50.0)).changed();
                ui.label(tr("Buses from", "Автобусы из"));
                egui::ComboBox::from_id_salt("tt depot").selected_text(if tour.ai_group.is_empty() { tr("(none)", "(нет)").to_string() } else { tour.ai_group.clone() }).show_ui(ui, |ui| {
                    for d in &depots {
                        changed |= ui.selectable_value(&mut tour.ai_group, d.clone(), d).changed();
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_button(ui, Icon::Delete, tr("Delete the tour", "Удалить выход")) {
                        delete_tour = true;
                    }
                });
            });
            // the days
            let mut mask = tour.extra.trim().parse::<i32>().unwrap_or(ALL_DAYS);
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("Runs on", "Дни"));
                let days = [tr("Mo", "Пн"), tr("Tu", "Вт"), tr("We", "Ср"), tr("Th", "Чт"), tr("Fr", "Пт"), tr("Sa", "Сб"), tr("Su", "Вс"), tr("Holidays", "Праздники")];
                for (b, d) in days.iter().enumerate() {
                    if check(ui, mask & (1 << b) != 0, d) {
                        mask ^= 1 << b;
                    }
                }
                ui.menu_button("…", |ui| {
                    if ui.button(tr("Every day", "Каждый день")).clicked() {
                        mask = ALL_DAYS;
                    }
                    if ui.button(tr("Monday to Friday", "Пн–Пт")).clicked() {
                        mask = 0x1f | 0x300;
                    }
                    if ui.button(tr("Weekends and holidays", "Выходные и праздники")).clicked() {
                        mask = 0x60 | 0x80 | 0x300;
                    }
                    ui.separator();
                    if check(ui, mask & 0x100 != 0, tr("In the school holidays", "В каникулы")) {
                        mask ^= 0x100;
                    }
                    if check(ui, mask & 0x200 != 0, tr("On school days", "В учебные дни")) {
                        mask ^= 0x200;
                    }
                });
            });
            if mask & 0x300 == 0 {
                // (a tour needs school days or holidays, or it never runs)
                mask |= 0x300;
            }
            if mask.to_string() != tour.extra.trim() {
                tour.extra = mask.to_string();
                changed = true;
            }

            ui.horizontal(|ui| {
                if ui.button(icons::labelled(Icon::Add, tr("Next departure", "Следующее отправление"))).on_hover_text(tr("The next trip of the line, when the bus is back and has had its break", "Следующий рейс линии, когда автобус вернулся и отстоял перерыв")).clicked() {
                    let (trip, dep) = match tour.trips.last() {
                        Some(last) => {
                            let k = trips.iter().position(|t| t.eq_ignore_ascii_case(&last.trip)).unwrap_or(0);
                            (trips[(k + 1) % trips.len()].clone(), last.departure + profiles.get(&lower(&last.trip)).and_then(|p| p.get(last.profile.max(0) as usize).or(p.first())).map(|p| p.1).unwrap_or(10.0) + self.layover as f32)
                        }
                        None => (trips[0].clone(), (self.fill_from.0 * 60 + self.fill_from.1) as f32),
                    };
                    tour.trips.push(TourTrip { trip, profile: 0, departure: dep });
                    resort = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("Fill from", "Заполнить с"));
                time_edit(ui, &mut self.fill_from);
                ui.label(tr("to", "до"));
                time_edit(ui, &mut self.fill_to);
                ui.label(tr("break", "перерыв"));
                ui.add(egui::DragValue::new(&mut self.layover).range(0..=120).suffix(tr(" min", " мин")));
                if ui.button(icons::labelled(Icon::Schedule, tr("Fill", "Заполнить"))).on_hover_text(tr("Replace the tour's departures: the line's trips one after the other, a break after each", "Заменить отправления выхода: рейсы линии друг за другом, перерыв после каждого")).clicked() {
                    let from = (self.fill_from.0 * 60 + self.fill_from.1) as f32;
                    let to = (self.fill_to.0 * 60 + self.fill_to.1) as f32;
                    tour.trips.clear();
                    let mut dep = from;
                    let mut k = 0;
                    while dep <= to && tour.trips.len() < 500 {
                        let t = &trips[k % trips.len()];
                        tour.trips.push(TourTrip { trip: t.clone(), profile: 0, departure: dep });
                        dep += times.get(&lower(t)).copied().unwrap_or(10.0).max(1.0) + self.layover as f32;
                        k += 1;
                    }
                    resort = true;
                }
            });

            // the departures
            ui.add_space(4.0);
            let mut remove = None;
            let count = tour.trips.len();
            if count == 0 {
                ui.label(RichText::new(tr("No departures yet: add them above.", "Отправлений пока нет: добавьте их выше.")).color(pal().text_dim));
            }
            egui::Grid::new("tt deps").num_columns(5).spacing(Vec2::new(8.0, 3.0)).show(ui, |ui| {
                for k in 0..count {
                    let tr_ = &mut tour.trips[k];
                    let mut h = (tr_.departure / 60.0).floor() as i32;
                    let mut m = (tr_.departure - h as f32 * 60.0).round() as i32;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let a = ui.add(egui::DragValue::new(&mut h).range(0..=47).custom_formatter(|v, _| format!("{:02}", v as i32)));
                        ui.label(":");
                        let b = ui.add(egui::DragValue::new(&mut m).range(-1..=60).custom_formatter(|v, _| format!("{:02}", v as i32)));
                        if a.changed() || b.changed() {
                            let mins = (h * 60 + m).clamp(0, 47 * 60 + 59);
                            tr_.departure = mins as f32;
                            changed = true;
                        }
                    });
                    egui::ComboBox::from_id_salt(("tt dep trip", k)).selected_text(tr_.trip.clone()).width(160.0).height(360.0).show_ui(ui, |ui| {
                        for t in trips {
                            changed |= ui.selectable_value(&mut tr_.trip, t.clone(), t).changed();
                        }
                        if !others.is_empty() {
                            ui.separator();
                            ui.label(RichText::new(tr("Other trips", "Другие рейсы")).small().color(pal().text_dim));
                            for t in &others {
                                changed |= ui.selectable_value(&mut tr_.trip, t.clone(), t).changed();
                            }
                        }
                    });
                    // the profile it runs to
                    let ps = profiles.get(&lower(&tr_.trip)).cloned().unwrap_or_default();
                    if ps.len() > 1 {
                        let cur = (tr_.profile.max(0) as usize).min(ps.len() - 1);
                        egui::ComboBox::from_id_salt(("tt dep profile", k)).selected_text(ps[cur].0.clone()).width(90.0).show_ui(ui, |ui| {
                            for (i, (n, m)) in ps.iter().enumerate() {
                                if ui.selectable_label(i == cur, format!("{n} · {}", mmss(*m))).clicked() && i as i32 != tr_.profile {
                                    tr_.profile = i as i32;
                                    changed = true;
                                }
                            }
                        })
                        .response
                        .on_hover_text(tr("The time profile this departure runs to", "Профиль времени этого отправления"));
                    } else {
                        ui.label("");
                    }
                    let arrive = ps.get(tr_.profile.max(0) as usize).or(ps.first()).map(|p| p.1).unwrap_or(0.0);
                    ui.label(RichText::new(format!("» {}", clock(tr_.departure + arrive))).color(pal().text_dim));
                    if small_button(ui, Icon::Delete, tr("Delete the departure", "Удалить отправление")) {
                        remove = Some(k);
                    }
                    ui.end_row();
                }
            });
            if let Some(k) = remove {
                tour.trips.remove(k);
                changed = true;
            }
        });
        if delete_tour {
            tt.lines[li].tours.remove(ti);
            self.tour = None;
            changed = true;
        }
        if resort {
            // (only when departures are added: rows being edited stay where they are)
            if let Some(t) = tt.lines[li].tours.get_mut(ti) {
                t.trips.sort_by(|a, b| a.departure.total_cmp(&b.departure));
            }
        }
        if changed || resort {
            tt.touch_line(&line);
        }
    }
}

/// The name of the depot group openMapper adds.
const DEPOT: &str = "openMapper";

/// A depot for a map without one, and the tours without buses take theirs from it. Its
/// buses get the map's own depot file, made from the lines, where there are trips (else
/// Grundorf's: its destinations are not the map's).
pub(crate) fn add_depot(doc: &mut Document) {
    let name = doc.traffic.unique_name(DEPOT);
    let mut group = crate::traffic::stock_depot(&name);
    if doc.timetable.trips.iter().any(|t| t.stations.len() >= 2) {
        group.hof = Some(crate::ui_hof::make_named(doc, ""));
    }
    doc.traffic.groups.push(group);
    doc.traffic.touch(crate::traffic::File::AiLists);
    let tt = &mut doc.timetable;
    let mut touched = Vec::new();
    for l in &mut tt.lines {
        for t in l.tours.iter_mut().filter(|t| t.ai_group.trim().is_empty()) {
            t.ai_group = name.clone();
            touched.push(l.name.clone());
        }
    }
    for l in touched {
        tt.touch_line(&l);
    }
    tt.mark();
}

/// Minutes as m:ss.
fn mmss(m: f32) -> String {
    let s = (m * 60.0).round().max(0.0) as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn time_edit(ui: &mut egui::Ui, t: &mut (u32, u32)) {
    ui.spacing_mut().item_spacing.x = 2.0;
    ui.add(egui::DragValue::new(&mut t.0).range(0..=47).custom_formatter(|v, _| format!("{:02}", v as i32)));
    ui.label(":");
    ui.add(egui::DragValue::new(&mut t.1).range(0..=59).custom_formatter(|v, _| format!("{:02}", v as i32)));
    ui.spacing_mut().item_spacing.x = 8.0;
}

#[cfg(test)]
mod station_edit_tests {
    use super::*;
    use omsi_timetable::TripProfile;

    fn profile() -> TripProfile {
        TripProfile {
            name: "standard".into(),
            factor: 20.0,
            man_dep_time: vec![(0, 1.0), (1, 7.0)],
            man_arr_time: vec![(0, 0.5), (1, 6.0), (2, 20.0)],
            other_stopping: vec![(0, 2), (1, 3), (2, 1)],
        }
    }

    fn trip(name: &str, stations: &[i64], terminus: &str) -> Trip {
        Trip { name: name.into(), stations: stations.into(), terminus: terminus.into(), profiles: vec![profile()], ..Default::default() }
    }

    fn print_trips(label: &str, trips: &[Trip]) {
        for trip in trips {
            println!("{label} {}.ttp:\n{}", trip.name, timetable::trip_text(trip));
        }
    }

    #[test]
    fn replace_station_id_updates_only_the_selected_row_and_keeps_profiles() {
        let mut trips = [trip("current", &[10, 11, 10], "Old"), trip("other", &[10, 10], "Old")];
        let before = trips.clone();
        print_trips("Before", &before);
        assert_eq!(
            replace_station_id(&mut trips, "CURRENT", 0, 10, 20, false, &["Old"], "New"),
            vec![("current".into(), vec![0])],
        );
        print_trips("After", &trips);
        assert_eq!(trips[0].stations, [20, 11, 10]);
        assert_eq!(trips[0].terminus, "Old");
        assert_eq!(trips[0].profiles, before[0].profiles);
        assert_eq!(trips[1], before[1]);
    }

    #[test]
    fn replace_station_id_updates_other_trips_but_skips_legacy_and_preserves_custom_signs() {
        let mut trips = [
            trip("current", &[10, 11, 10], " Old on map "),
            trip("other", &[10, 11, 10], "Old in timetable"),
            trip("blank", &[11, 11, 10], " "),
            trip("custom", &[11, 10, 10], "Depot"),
            trip("legacy", &[10, 11, 10], "Old on map"),
            trip("unrelated", &[21, 22, 23], "Old on map"),
        ];
        trips[4].stations_legacy.push(vec!["legacy stop".into()]);
        let before = trips.clone();
        print_trips("Before", &before);
        let edited = replace_station_id(&mut trips, "current", 2, 10, 20, true, &["Old on map", "Old in timetable"], "New");
        println!("Changed trip rows (zero-based): {edited:?}");
        print_trips("After", &trips);
        assert_eq!(edited, [("current".into(), vec![2]), ("other".into(), vec![0, 2]), ("blank".into(), vec![2]), ("custom".into(), vec![1, 2])]);
        assert_eq!(trips[0].stations, [10, 11, 20]);
        assert_eq!(trips[1].stations, [20, 11, 20]);
        assert_eq!(trips[2].stations, [11, 11, 20]);
        assert_eq!(trips[3].stations, [11, 20, 20]);
        assert_eq!(trips[..3].iter().map(|t| t.terminus.as_str()).collect::<Vec<_>>(), ["New"; 3]);
        assert_eq!(trips[3].terminus, "Depot");
        assert_eq!(trips[4..], before[4..]);
        for (old, new) in before.iter().zip(&trips) {
            assert_eq!(old.profiles, new.profiles);
        }
        assert!(replace_station_id(&mut trips, "current", 2, 10, 21, true, &[], "Invalid").is_empty());
    }

    #[test]
    fn inserting_at_the_start_middle_and_before_the_last_stop_shifts_profiles() {
        for index in 0..3 {
            // Stop 10 occurs twice with different fixed times and stopping rules.
            let mut trip = trip("loop", &[10, 11, 10], "Loop terminus");
            let mut second = profile();
            second.name = "evening".into();
            second.other_stopping = vec![(0, 1), (2, 2)];
            trip.profiles.push(second);
            print_trips(&format!("Before insertion at {index}"), std::slice::from_ref(&trip));
            let mut old_rows = vec![Some(0), Some(1), Some(2)];
            old_rows.insert(index, None);
            trip.stations.insert(index, 12);
            remap_station_profiles(&mut trip.profiles, 3, &trip.stations, &old_rows, |id| {
                assert_eq!(id, 12);
                Some(crate::stops::StopKind::Regular.serve())
            });
            print_trips(&format!("After insertion at {index}"), std::slice::from_ref(&trip));
            let shift = |old: i32| old + i32::from(old as usize >= index);
            let standard = &trip.profiles[0];
            assert_eq!(standard.man_dep_time, [(shift(0), 1.0), (shift(1), 7.0)]);
            assert_eq!(standard.man_arr_time, [(shift(0), 0.5), (shift(1), 6.0)]);
            assert_eq!(standard.factor, 20.0);
            let mut expected = vec![(shift(0), 2), (shift(1), 3), (shift(2), 1)];
            expected.insert(index, (index as i32, 1));
            assert_eq!(standard.other_stopping, expected);
            let mut expected = vec![(shift(0), 1), (index as i32, 1), (shift(2), 2)];
            expected.sort_by_key(|row| row.0);
            assert_eq!(trip.profiles[1].other_stopping, expected);
        }
    }

    fn frame(ctx: &egui::Context, ui: &mut TimetableUi, ed: &mut Editor, events: Vec<egui::Event>) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1400.0, 900.0))),
            events,
            ..Default::default()
        };
        ctx.run_ui(input, |root| ui.window(root.ctx(), ed, None, &mut Vec::new()))
    }

    fn editor_fixture() -> (Editor, TimetableUi, egui::Context) {
        let dir = std::env::temp_dir().join("openmapper-station-edit-test");
        let roots = Roots { omsi: dir.join("omsi"), content: dir.join("content") };
        let mut doc = Document::new_map(&roots, "stop-edit-test", "stop-edit-test", 1);
        let line = doc.timetable.add_line("73E");
        for mut t in [trip("current", &[10, 11, 10], "Old"), trip("other", &[10, 10], "Old")] {
            t.line = line.clone();
            doc.timetable.trips.push(t);
        }
        let mut ui = TimetableUi { open: true, map: Some(doc.dir.clone()), ..TimetableUi::new() };
        ui.select_line(&doc.timetable, Some(line));
        ui.stops = [11, 12].into_iter().map(|id| MapStop {
            tile: (0, 0), id, name: format!("Stop {id}"), pos: DVec3::ZERO, kind: crate::stops::StopKind::Regular,
        }).collect();
        let mut ed = Editor::default();
        ed.doc = Some(doc);
        let ctx = egui::Context::default();
        ctx.set_fonts(icons::fonts());
        (ed, ui, ctx)
    }

    #[test]
    fn type2_conversion_checkbox_unlocks_stop_picking_for_a_zero_stop_legacy_trip() {
        let _map = crate::doc::one_map();
        let (mut ed, mut ui, ctx) = editor_fixture();
        let tt = &mut ed.doc.as_mut().unwrap().timetable;
        let legacy = tt.trip_mut("current").unwrap();
        legacy.display_name = "tram-track".into();
        legacy.stations.clear();
        println!("Before:\n{}", timetable::trip_text(legacy));
        let text_center = |output: &egui::FullOutput, label: &str| {
            output.shapes.iter().rev().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains(label) => Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            })
        };
        let click = |ui: &mut TimetableUi, ed: &mut Editor, pos| {
            let button = |pressed| egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::default(),
            };
            frame(&ctx, ui, ed, vec![egui::Event::PointerMoved(pos), button(true)]);
            frame(&ctx, ui, ed, vec![button(false)])
        };
        frame(&ctx, &mut ui, &mut ed, Vec::new());
        let output = frame(&ctx, &mut ui, &mut ed, Vec::new());
        let exclude = tr("Exclude from Type 2 conversion", "Исключить из преобразования в тип 2");
        let pick = tr("Pick stops on the map", "Выбрать на карте");
        assert!(text_center(&output, pick).is_none());
        let button = text_center(&output, type2_conversion_label()).unwrap();
        let output = click(&mut ui, &mut ed, button);
        assert!(!ui.confirm_type2_conversion);
        let checkbox = text_center(&output, exclude).unwrap();
        click(&mut ui, &mut ed, checkbox);
        let tt = &ed.doc.as_ref().unwrap().timetable;
        assert!(!tt.excludes_type2(tt.trip("current").unwrap()));
        let output = frame(&ctx, &mut ui, &mut ed, Vec::new());
        let button = text_center(&output, type2_conversion_label()).unwrap();
        click(&mut ui, &mut ed, button);
        assert!(ui.confirm_type2_conversion);
        let output = frame(&ctx, &mut ui, &mut ed, Vec::new());
        let modal_button = text_center(&output, type2_conversion_label()).unwrap();
        assert_ne!(modal_button, button);
        click(&mut ui, &mut ed, modal_button);
        assert!(!ui.confirm_type2_conversion);
        let output = frame(&ctx, &mut ui, &mut ed, Vec::new());
        assert!(text_center(&output, type2_conversion_label()).is_none());
        assert!(text_center(&output, exclude).is_none());
        let pick_button = text_center(&output, pick).unwrap();
        click(&mut ui, &mut ed, pick_button);
        assert!(ed.pick_mode);
        ed.picked = Some((Some(Sel::Object((0, 0), 12)), None));
        frame(&ctx, &mut ui, &mut ed, Vec::new());
        let converted = ed.doc.as_ref().unwrap().timetable.trip("current").unwrap();
        println!("After conversion and picking a stop:\n{}", timetable::trip_text(converted));
        assert!(!timetable::is_type1(converted));
        assert_eq!(converted.stations, [12]);
        ed.undo();
        ed.undo();
        ed.picked = Some((Some(Sel::Object((0, 0), 11)), None));
        frame(&ctx, &mut ui, &mut ed, Vec::new());
        let restored = ed.doc.as_ref().unwrap().timetable.trip("current").unwrap();
        println!("After undo:\n{}", timetable::trip_text(restored));
        assert!(timetable::is_type1(restored));
        assert!(restored.stations.is_empty() && !ed.pick_mode);
    }

    #[test]
    fn map_insertion_and_id_replacement_each_undo_through_the_timetable_window() {
        let _map = crate::doc::one_map();
        let (mut ed, mut ui, ctx) = editor_fixture();
        let before = ed.doc.as_ref().unwrap().timetable.clone();
        ui.insert_before = Some(1);
        frame(&ctx, &mut ui, &mut ed, Vec::new());
        assert!(ed.pick_mode);
        assert!(ui.replace_id.is_none());
        ed.picked = Some((Some(Sel::Object((0, 0), 11)), None));
        frame(&ctx, &mut ui, &mut ed, Vec::new());
        assert_eq!(ui.insert_before, Some(1));
        assert!(ed.toasts.last().unwrap().error);
        assert!(ed.doc.as_ref().unwrap().timetable.same_content(&before));

        ed.picked = Some((Some(Sel::Object((0, 0), 12)), None));
        frame(&ctx, &mut ui, &mut ed, Vec::new());
        assert!(!ed.pick_mode);
        let inserted = ed.doc.as_ref().unwrap().timetable.clone();
        assert_eq!(inserted.trip("current").unwrap().stations, [10, 12, 11, 10]);
        print_trips("After map click", &inserted.trips);
        ed.undo();
        assert!(ed.doc.as_ref().unwrap().timetable.same_content(&before));
        ed.redo();
        assert!(ed.doc.as_ref().unwrap().timetable.same_content(&inserted));

        ui.replace_id = Some(ReplaceStopId { index: 3, old_id: 10, input: "12".into(), all: true });
        let output = frame(&ctx, &mut ui, &mut ed, Vec::new());
        let confirm = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == tr("Confirm", "Подтвердить") => Some(text.pos + text.galley.size() * 0.5),
            _ => None,
        }).expect("the replacement editor shows the confirmation button");
        let click = |pressed| egui::Event::PointerButton {
            pos: confirm, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::default(),
        };
        frame(&ctx, &mut ui, &mut ed, vec![egui::Event::PointerMoved(confirm), click(true)]);
        frame(&ctx, &mut ui, &mut ed, vec![click(false)]);
        let replaced = ed.doc.as_ref().unwrap().timetable.clone();
        print_trips("After Confirm", &replaced.trips);
        assert_eq!(replaced.trip("current").unwrap().stations, [10, 12, 11, 12]);
        assert_eq!(replaced.trip("other").unwrap().stations, [12, 12]);
        assert!(replaced.reroute.contains("current") && replaced.reroute.contains("other"));
        assert_eq!(ed.history.undo_label(), Some(tr("Change ID", "Заменить ID")));
        ed.undo();
        print_trips("After Undo", &ed.doc.as_ref().unwrap().timetable.trips);
        assert!(ed.doc.as_ref().unwrap().timetable.same_content(&inserted));
        ed.redo();
        assert!(ed.doc.as_ref().unwrap().timetable.same_content(&replaced));
    }

    #[test]
    fn escape_cancels_the_map_pick_and_line_selection_clears_station_edits() {
        let _map = crate::doc::one_map();
        let (mut ed, mut ui, ctx) = editor_fixture();
        let before = ed.doc.as_ref().unwrap().timetable.clone();
        ui.insert_before = Some(1);
        frame(&ctx, &mut ui, &mut ed, Vec::new());
        ed.picked = Some((Some(Sel::Object((0, 0), 12)), None));
        let escape = egui::Event::Key {
            key: egui::Key::Escape, physical_key: Some(egui::Key::Escape), pressed: true, repeat: false, modifiers: egui::Modifiers::default(),
        };
        frame(&ctx, &mut ui, &mut ed, vec![escape]);
        assert!(!ed.pick_mode && ed.picked.is_none() && ui.insert_before.is_none());
        let tt = &ed.doc.as_ref().unwrap().timetable;
        print_trips("After Escape", &tt.trips);
        assert!(tt.same_content(&before));
        ui.replace_id = Some(ReplaceStopId { index: 0, old_id: 10, input: "12".into(), all: true });
        ui.select_line(tt, None);
        assert!(ui.replace_id.is_none() && ui.insert_before.is_none() && ui.pending_insert.is_none());
    }
}


#[cfg(test)]
mod batch_route_tests {
    use super::*;
    use crate::timetable::network::{Lane, PathKey};

    fn link(from_id: i64, to_id: i64, length: f64) -> omsi_timetable::StnLink {
        omsi_timetable::StnLink { from_id, to_id, length, ..Default::default() }
    }

    /// A link with an AI path: one entry per lane driven (`lane` becomes the entry's IDCode).
    fn lane_link(from_id: i64, to_id: i64, length: f64, lanes: &[i64]) -> omsi_timetable::StnLink {
        let entries = lanes.iter().map(|lane| omsi_timetable::StnLinkEntry {
            values: [*lane as f64, 0.0, 0.0, length, -1.0, 0.0, 0.0],
        }).collect();
        omsi_timetable::StnLink {
            length,
            from_id,
            to_id,
            params: [0.0, 0.0, 0.0, 0.0, 0.0, lanes.len().saturating_sub(1) as f64],
            entries,
        }
    }

    fn names() -> HashMap<i64, String> {
        [(1, "Central"), (2, "Market"), (3, "Depot")].into_iter().map(|(id, n)| (id, n.to_string())).collect()
    }

    fn plan(name: &str, links: Vec<omsi_timetable::StnLink>) -> PlannedRoute {
        (name.to_string(), Vec::new(), links)
    }

    #[test]
    fn v2_station_link_trips_are_eligible_but_legacy_tracks_are_not() {
        let mut trip = omsi_timetable::Trip {
            stations: vec![5521173, 5521015, 5537684, 2354518],
            ..Default::default()
        };
        assert!(eligible_for_batch(&trip));
        trip.stations_legacy.push(vec!["legacy".into()]);
        assert!(!eligible_for_batch(&trip));
        trip.stations_legacy.clear();
        trip.stations.truncate(1);
        assert!(!eligible_for_batch(&trip));
    }

    #[test]
    fn batch_reroute_preserves_hand_paths_unless_overwrite_succeeds() {
        let _map = crate::doc::one_map();
        let root = std::env::temp_dir().join("openmapper-batch-hand");
        let roots = Roots { omsi: root.join("omsi"), content: root.join("content") };
        let mut doc = Document::new_map(&roots, "batch-hand", "batch-hand", 1);
        let key = PathKey { tile: doc.order[0], id: 50, path: 0 };
        let lane = Lane {
            key, pts: vec![DVec3::ZERO, DVec3::X * 100.0], along: vec![0.0, 100.0],
            file_length: 100.0, internal: 0, next: Vec::new(), side: Vec::new(),
            against: false, spline: true, rail: false,
        };
        let mut ui = TimetableUi::new();
        ui.net = Some(Network { lanes: vec![lane], by_key: HashMap::from([(key, vec![0])]), ..Default::default() });
        ui.stops = [(1, 10.0), (2, 90.0)].into_iter().map(|(id, x)| MapStop {
            tile: doc.order[0], id, name: format!("Stop {id}"), pos: DVec3::X * x,
            kind: crate::stops::StopKind::Regular,
        }).collect();
        for (name, stations) in [("hand", vec![1, 2]), ("auto", vec![1, 2]), ("broken", vec![1, 3])] {
            doc.timetable.add_trip("", name, stations);
            doc.timetable.reroute.remove(name);
        }
        let original = omsi_timetable::TrackEntry { values: vec![99.0, 0.0, 0.0, 0.0, 80.0, 0.0] };
        doc.timetable.set_by_hand("hand", Some((vec![original.clone()], vec![link(1, 2, 99.0)])));
        doc.timetable.set_by_hand("broken", Some((vec![original], vec![link(1, 3, 100.0)])));
        let track = |tt: &Timetable, name| timetable::track_text_of(tt.track(name).unwrap(), tt.by_hand(name));
        let before = track(&doc.timetable, "hand");
        let broken = track(&doc.timetable, "broken");
        let keep = ui.recalculate_all(&mut doc, false);
        println!("Original hand route:\n{before}\nPreserve mode: {keep}\n{}\n{:?}", track(&doc.timetable, "hand"), doc.timetable.links);
        assert!(doc.timetable.by_hand("hand"));
        assert_eq!(track(&doc.timetable, "hand"), before);
        assert!(doc.timetable.track("auto").is_none());
        assert!(doc.timetable.links.contains(&link(1, 2, 99.0)));

        let overwrite = ui.recalculate_all(&mut doc, true);
        println!("Overwrite mode: {overwrite}\n{}\n{}\n{:?}", track(&doc.timetable, "hand"), track(&doc.timetable, "broken"), doc.timetable.links);
        assert!(!doc.timetable.by_hand("hand"));
        assert!(doc.timetable.track("auto").is_some());
        assert_ne!(track(&doc.timetable, "hand"), before);
        assert_eq!(track(&doc.timetable, "broken"), broken);
        assert!(doc.timetable.links.contains(&link(1, 3, 100.0)));
    }

    #[test]
    fn identical_shared_links_are_safe() {
        let same = link(1, 2, 120.0);
        let plans = vec![plan("a", vec![same.clone()]), plan("b", vec![same])];
        let (conflicts, causes) = conflicting_batch_routes(&plans, &[], &HashSet::new(), &names());
        assert!(conflicts.is_empty());
        assert!(causes.is_empty());
    }

    #[test]
    fn disagreeing_shared_links_skip_both_trips() {
        let plans = vec![plan("a", vec![link(1, 2, 120.0)]), plan("b", vec![link(1, 2, 200.0)])];
        let (conflicts, causes) = conflicting_batch_routes(&plans, &[], &HashSet::new(), &names());
        assert_eq!(conflicts, HashSet::from([0, 1]));
        assert_eq!(causes.len(), 1);
        assert_eq!(causes[0].kind, ConflictKind::Disagree);
        assert_eq!(causes[0].round, 0);
    }

    #[test]
    fn two_trips_on_different_lanes_disagree_with_their_ai_paths_in_full() {
        let plans = vec![
            plan("a", vec![lane_link(1, 2, 120.0, &[100, 101])]),
            plan("b", vec![lane_link(1, 2, 120.0, &[200, 201])]),
        ];
        let (conflicts, causes) = conflicting_batch_routes(&plans, &[], &HashSet::new(), &names());
        assert_eq!(conflicts, HashSet::from([0, 1]));
        assert_eq!(causes.len(), 1, "one pair, one cause: {causes:?}");
        let cause = &causes[0];
        assert_eq!(cause.kind, ConflictKind::Disagree);
        assert_eq!(cause.round, 0, "a disagreement between trips is always a root cause");
        assert_eq!((cause.from_id, cause.to_id), (1, 2));
        assert_eq!((cause.from_name.as_str(), cause.to_name.as_str()), ("Central", "Market"));
        assert_eq!(cause.trips.len(), 2, "both trips that calculated the pair are listed");
        assert_eq!(cause.new_trips, vec![0, 1]);
        let diff = cause.diffs.iter().find(|d| d.field == "AI path sequence").expect("the AI paths differ");
        println!("AI path diff: {}", diff.detail);
        assert!(diff.detail.contains("100.000"), "the full first sequence: {}", diff.detail);
        assert!(diff.detail.contains("101.000"), "the full first sequence: {}", diff.detail);
        assert!(diff.detail.contains("200.000"), "the full second sequence: {}", diff.detail);
        assert_eq!(cause.summary(), "Central(1) → Market(2) | Disagree | 2 trips | AI path sequence");
    }

    #[test]
    fn a_conflict_protects_all_other_links_of_the_skipped_trip() {
        let plans = vec![
            plan("a", vec![link(1, 2, 120.0), link(2, 3, 150.0)]),
            plan("b", vec![link(1, 2, 200.0)]),
            plan("c", vec![link(2, 3, 150.0)]),
        ];
        // Trip a is skipped because it conflicts with b on 1->2. Trip c must
        // also be skipped, because it would change a's original 2->3 link.
        let original = vec![link(2, 3, 100.0)];
        let (conflicts, causes) = conflicting_batch_routes(&plans, &original, &HashSet::new(), &names());
        assert_eq!(conflicts, HashSet::from([0, 1, 2]));
        // (root causes first)
        let root = causes.iter().position(|c| c.round == 0).expect("a root cause");
        assert_eq!((causes[root].kind, causes[root].from_id, causes[root].to_id), (ConflictKind::Disagree, 1, 2));
        let propagated = causes.iter().find(|c| c.kind == ConflictKind::Propagated).expect("c is propagated");
        let propagated_index = causes.iter().position(|c| c.kind == ConflictKind::Propagated).unwrap();
        assert!(root < propagated_index, "root causes come first: {causes:#?}");
        assert_eq!((propagated.from_id, propagated.to_id), (2, 3));
        assert_eq!(propagated.round, 1, "2->3 becomes protected only after round 0");
        assert_eq!(propagated.new_trips, vec![2], "only c is new: a was already skipped");
        let via = propagated.via.as_ref().expect("a propagated cause names its source");
        assert_eq!(via.trip, "a");
        assert_eq!(via.pair, Some((1, 2)));
        // (the map's link is what c would have replaced)
        assert_eq!(propagated.original.as_ref().map(|l| l.length), Some(100.0));
        assert!(propagated.diffs.iter().any(|d| d.field == "length" && d.detail.contains("150.000") && d.detail.contains("100.000")));
    }

    #[test]
    fn a_failed_trip_keeps_its_existing_shared_link() {
        let original = link(1, 2, 120.0);
        let protected = HashSet::from([(1, 2)]);
        let plans = vec![plan("a", vec![link(1, 2, 130.0)])];
        let (conflicts, causes) = conflicting_batch_routes(&plans, &[original.clone()], &protected, &names());
        assert_eq!(conflicts, HashSet::from([0]));
        assert_eq!(causes.len(), 1);
        assert_eq!((causes[0].kind, causes[0].round), (ConflictKind::Protected, 0), "protected before the batch, not propagated");
        assert_eq!(causes[0].new_trips, vec![0]);
        let plans = vec![plan("a", vec![original.clone()])];
        let (conflicts, causes) = conflicting_batch_routes(&plans, &[original], &protected, &names());
        assert!(conflicts.is_empty());
        assert!(causes.is_empty());
    }

    #[test]
    fn the_conflict_report_lists_root_causes_first_with_every_value() {
        let plans = vec![
            plan("a", vec![lane_link(1, 2, 120.0, &[100]), lane_link(2, 3, 150.0, &[300])]),
            plan("b", vec![lane_link(1, 2, 120.0, &[200])]),
            plan("c", vec![lane_link(2, 3, 150.0, &[300])]),
        ];
        let original = vec![lane_link(2, 3, 100.0, &[999])];
        let (conflicts, causes) = conflicting_batch_routes(&plans, &original, &HashSet::new(), &names());
        let text = conflict_report(std::path::Path::new("/maps/test"), &plans, &conflicts, &causes);
        println!("{text}");
        let root = text.find("round 0 | Disagree").expect("the root cause is listed");
        let propagated = text.find("round 1 | Propagated").expect("the propagated cause is listed");
        assert!(root < propagated, "root causes first:\n{text}");
        assert!(text.contains("100.000/0.000/0.000/120.000/-1.000/0.000/0.000"), "the first AI path in full:\n{text}");
        assert!(text.contains("200.000/0.000/0.000/120.000/-1.000/0.000/0.000"), "the second AI path in full:\n{text}");
        assert!(text.contains("propagated from trip a on 1 -> 2"), "the source is named:\n{text}");
        assert!(!text.contains("No shared station link conflicted"));
    }

    #[test]
    fn a_batch_without_conflicts_writes_that_nothing_conflicted() {
        let plans = vec![plan("a", vec![link(1, 2, 120.0)])];
        let (conflicts, causes) = conflicting_batch_routes(&plans, &[], &HashSet::new(), &names());
        let text = conflict_report(std::path::Path::new("/maps/test"), &plans, &conflicts, &causes);
        assert!(text.contains("No shared station link conflicted"), "{text}");
        assert!(text.contains("conflicting trips: 0"), "{text}");
    }

    #[test]
    fn the_conflict_file_is_overwritten_by_the_next_batch() {
        let dir = std::env::temp_dir().join("openmapper-conflicts-overwrite");
        std::fs::create_dir_all(&dir).unwrap();
        let plans = vec![plan("a", vec![link(1, 2, 120.0)]), plan("b", vec![link(1, 2, 200.0)])];
        let (conflicts, causes) = conflicting_batch_routes(&plans, &[], &HashSet::new(), &names());
        let path = write_conflict_report(&dir, &plans, &conflicts, &causes).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.contains("Disagree"), "{first}");
        // the next batch has nothing to report: the old text must not stay behind
        let (clean, none) = conflicting_batch_routes(&plans[..1], &[], &HashSet::new(), &names());
        write_conflict_report(&dir, &plans[..1], &clean, &none).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert!(second.contains("No shared station link conflicted"), "{second}");
        assert!(!second.contains("Disagree"), "the previous batch's text is gone:\n{second}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod type1_track_preview_tests {
    use super::*;
    use crate::timetable::network::{Lane, PathKey};
    use omsi_timetable::{Track, TrackEntry};

    /// The saved Type 1 track is looked up by its explicit display_name, not by
    /// trip name, and lost paths stay as missing entries instead of being routed.
    #[test]
    fn legacy_track_preview_resolves_exact_source_and_keeps_missing_entries() {
        let trip = Trip {
            path: "line.ttp".into(),
            name: "trip-A".into(),
            display_name: "original-way".into(),
            terminus: String::new(),
            line: String::new(),
            train_reverse: false,
            stations: Vec::new(),
            stations_legacy: vec![vec!["10".into(), "0".into()]],
            profiles: Vec::new(),
        };
        let entry = |id: i64| TrackEntry { values: vec![id as f64, 0.0, 0.0, 0.0, 10.0, 0.0] };
        let mut tt = Timetable::default();
        tt.trips.push(trip.clone());
        tt.tracks.extend([
            Track { path: "original-way.ttr".into(), entries: vec![entry(10), entry(999), entry(20)] },
            Track { path: "trip-A.ttr".into(), entries: vec![entry(30)] },
        ]);
        let key = |id| PathKey { tile: (0, 0), id, path: 0 };
        let lane = |id, x: f64| Lane {
            key: key(id),
            pts: vec![DVec3::new(x, 0.0, 0.0), DVec3::new(x + 10.0, 0.0, 0.0)],
            along: vec![0.0, 10.0],
            file_length: 10.0,
            internal: 0,
            next: Vec::new(),
            side: Vec::new(),
            against: false,
            spline: true,
            rail: false,
        };
        let net = Network {
            lanes: vec![lane(10, 0.0), lane(20, 20.0), lane(30, 30.0)],
            by_key: HashMap::from([(key(10), vec![0]), (key(20), vec![1]), (key(30), vec![2])]),
            ..Default::default()
        };
        let order = [(0, 0)];
        let before = tt.clone();
        let (name, paths) = type1_track_lanes(&tt, &trip, &net, &order);
        assert_eq!(name, "original-way");
        assert_eq!(paths.unwrap(), vec![Some(0), None, Some(1)]);
        assert!(tt.same_content(&before), "preview must not modify timetable data");

        // source() resolves the Trip retained by Timetable, rather than an
        // unrelated cloned Trip value: change that retained record for each case.
        tt.trip_mut("trip-A").unwrap().display_name = "no-such-track".into();
        let missing = tt.trip("trip-A").unwrap().clone();
        let unchanged = tt.clone();
        let (name, paths) = type1_track_lanes(&tt, &missing, &net, &order);
        assert_eq!(name, "no-such-track");
        assert!(paths.is_none(), "an explicit missing name must not fall back to trip-A.ttr");
        assert!(tt.same_content(&unchanged), "a failed lookup must not modify timetable data");

        tt.trip_mut("trip-A").unwrap().display_name.clear();
        let fallback = tt.trip("trip-A").unwrap().clone();
        let unchanged = tt.clone();
        let (name, paths) = type1_track_lanes(&tt, &fallback, &net, &order);
        assert_eq!(name, "trip-A");
        assert_eq!(paths.unwrap(), vec![Some(2)]);
        assert!(tt.same_content(&unchanged), "reading a fallback track must be read-only");
        tt.trip_mut("trip-A").unwrap().display_name = trip.display_name;
        assert!(tt.same_content(&before));
    }

    /// Selecting a Type 1 trip must actually populate the same shown route used
    /// by the map overlay, without editing the trip or invoking automatic search.
    #[test]
    fn selecting_type1_trip_shows_the_saved_way_without_writing_anything() {
        let _map = crate::doc::one_map();
        let root = std::env::temp_dir().join("openmapper-type1-readonly-preview");
        let roots = Roots { omsi: root.join("omsi"), content: root.join("content") };
        let mut doc = Document::new_map(&roots, "readonly-preview", "readonly-preview", 1);
        let tile = doc.order[0];
        let stop_id = 123;
        doc.timetable.trips.push(Trip {
            path: "legacy.ttp".into(),
            name: "legacy".into(),
            display_name: "separate-track".into(),
            terminus: String::new(),
            line: String::new(),
            train_reverse: false,
            stations: Vec::new(),
            stations_legacy: vec![vec![stop_id.to_string(), "0".into()]],
            profiles: Vec::new(),
        });
        doc.timetable.tracks.push(Track {
            path: "separate-track.ttr".into(),
            entries: vec![TrackEntry { values: vec![42.0, 0.0, 0.0, 0.0, 30.0, 0.0] }],
        });
        let before = doc.timetable.clone();
        let key = PathKey { tile, id: 42, path: 0 };
        let lane = Lane {
            key,
            pts: vec![DVec3::ZERO, DVec3::X * 30.0],
            along: vec![0.0, 30.0],
            file_length: 30.0,
            internal: 0,
            next: Vec::new(),
            side: Vec::new(),
            against: false,
            spline: true,
            rail: false,
        };
        let mut ed = Editor::default();
        ed.doc = Some(doc);
        let mut ui = TimetableUi::new();
        ui.net = Some(Network {
            lanes: vec![lane],
            by_key: HashMap::from([(key, vec![0])]),
            ..Default::default()
        });
        ui.stops.push(MapStop {
            tile, id: stop_id, name: "Legacy stop".into(),
            pos: DVec3::X * 5.0, kind: crate::stops::StopKind::Regular,
        });
        ui.trip = Some("legacy".into());
        ui.update_shown(&ed);
        let shown = ui.shown.as_ref().expect("selected Type 1 track is visible in the overlay");
        assert_eq!(shown.0, "legacy");
        assert_eq!(shown.1, vec![stop_id]);
        assert_eq!(shown.3.lanes, vec![0]);
        assert!(shown.3.by_hand, "track geometry is displayed as saved, not automatically found");
        assert!(ed.doc.as_ref().unwrap().timetable.same_content(&before));
    }
}
