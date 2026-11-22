//! The station links (`StnLinks.cfg`) are keyed only by the two stop IDs, so every trip
//! between the same stops drives the one link: a single trip's way may only be written when
//! it agrees with what the other trips on that pair have. This is the local check a single
//! trip's recalculation uses; the batch recalculation keeps its own propagation
//! (`conflicting_batch_routes` in `ui_timetable`), which this module shares only the
//! comparison of two links with.
//!
//! Nothing is written here: [`link_conflicts`] answers what would conflict, and the write
//! gate in [`super::Timetable::set_route`] refuses the whole write when anything does.

use std::collections::HashSet;

use glam::DVec3;
use omsi_timetable::{StnLink, TrackEntry, Trip};

use super::route::Problem;
use super::Timetable;

/// One field two links disagree on.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldDiff {
    /// Short name for the summary line ("AI path sequence", "length", ...).
    pub field: &'static str,
    /// Both sides' values, for the log and the report file.
    pub detail: String,
}

/// An AI path sequence in full: every entry's seven numbers (`id/path/tile/length/-1/0/0`).
pub fn entries_text(entries: &[omsi_timetable::StnLinkEntry]) -> String {
    let items: Vec<String> = entries.iter().map(|e| e.values.iter().map(|v| format!("{v:.3}")).collect::<Vec<_>>().join("/")).collect();
    format!("[{}]", items.join(", "))
}

/// The fields two links disagree on (the station pair is the same by construction).
pub fn link_diffs(a: &str, first: &StnLink, b: &str, other: &StnLink) -> Vec<FieldDiff> {
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

/// A link in full, the same text the batch diagnostics use for a candidate.
pub fn describe_link(name: &str, link: &StnLink) -> String {
    let p = link.params;
    format!(
        "{}: length {:.3}, stop A off {:.3} / along {:.3}, stop B off {:.3} / along {:.3}, \
         flag {:.0}, last entry {:.0}, AI path {}",
        name, link.length, p[0], p[2], p[1], p[3], p[4], p[5], entries_text(&link.entries)
    )
}

/// The stops a trip drives between, in order: its Type 2 stop IDs, or the object IDs of its
/// legacy `[station]` records (a trip with both is a Type 1 trip, read by those records).
pub fn station_pairs(trip: &Trip) -> Vec<(i64, i64)> {
    if trip.stations_legacy.is_empty() {
        return trip.stations.windows(2).map(|w| (w[0], w[1])).collect();
    }
    let mut out = Vec::new();
    let mut previous: Option<i64> = None;
    for record in &trip.stations_legacy {
        let id = record.first().and_then(|s| s.trim().parse::<i64>().ok());
        if let (Some(a), Some(b)) = (previous, id) {
            out.push((a, b));
        }
        previous = id;
    }
    out
}

/// Every trip of the map that shares a station pair with `trip`, itself included (lower-case
/// names): the scope of a batch recalculation started from one trip.
pub fn sharing_trips(tt: &Timetable, trip: &str) -> HashSet<String> {
    let mut scope = HashSet::new();
    let Some(start) = tt.trip(trip) else { return scope };
    scope.insert(start.name.to_lowercase());
    loop {
        let pairs: HashSet<(i64, i64)> = tt.trips.iter().filter(|t| scope.contains(&t.name.to_lowercase())).flat_map(station_pairs).collect();
        let mut added = false;
        for t in &tt.trips {
            let key = t.name.to_lowercase();
            if !scope.contains(&key) && station_pairs(t).iter().any(|p| pairs.contains(p)) {
                scope.insert(key);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    scope
}

/// Why a route was not written: station links other trips drive on would change, or the
/// trip's own `.ttr` cannot be written or taken away without taking another trip's track.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RouteRefusal {
    /// Shared station links that would have changed (empty when only the track refused).
    pub conflicts: Vec<SharedLinkConflict>,
    /// Why the trip's track could not be repaired or written.
    pub track: Option<String>,
}

impl RouteRefusal {
    pub fn links(conflicts: Vec<SharedLinkConflict>) -> Self {
        RouteRefusal { conflicts, track: None }
    }

    pub fn track(why: String) -> Self {
        RouteRefusal { conflicts: Vec::new(), track: Some(why) }
    }

    /// Write the whole refusal to the log: every conflicting link in full and the track's
    /// reason, so the input and the output of a refused write are both in the log.
    pub fn log(&self, trip: &str) {
        if !self.conflicts.is_empty() {
            log::warn!("timetable: the route of {trip} was not written: {} shared station link(s) would change", self.conflicts.len());
            for c in &self.conflicts {
                log::warn!("timetable: {trip}: {}", c.log_text());
            }
        }
        if let Some(why) = &self.track {
            log::warn!("timetable: the route of {trip} was not written: {why}");
        }
    }

    /// One-line reason for a caller's own message (file imports, MCP).
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.conflicts.is_empty() {
            parts.push(format!(
                "shared station links would change: {}",
                self.conflicts.iter().map(SharedLinkConflict::pair).collect::<Vec<_>>().join(", ")
            ));
        }
        if let Some(why) = &self.track {
            parts.push(why.clone());
        }
        parts.join("; ")
    }
}

/// Why a shared station link was not written.
#[derive(Clone, Debug, PartialEq)]
pub struct SharedLinkConflict {
    pub from_id: i64,
    pub to_id: i64,
    pub from_name: String,
    pub to_name: String,
    /// The other trips between these stops (the trip being written is not listed).
    pub trips: Vec<String>,
    /// The map's link for the pair, when it has one to compare with.
    pub existing: Option<StnLink>,
    /// The link that was to be written.
    pub new: StnLink,
    /// What differs, field by field (`existing` first against `new`); empty when the map
    /// has no link for the pair yet.
    pub diffs: Vec<FieldDiff>,
}

impl SharedLinkConflict {
    /// `Central(1) → Market(2)`, for a one-line message.
    pub fn pair(&self) -> String {
        format!("{}({}) → {}({})", self.from_name, self.from_id, self.to_name, self.to_id)
    }

    /// The other trips between the stops, comma-separated.
    pub fn users(&self) -> String {
        self.trips.join(", ")
    }

    /// One log line with both links in full and what differs.
    pub fn log_text(&self) -> String {
        let mut o = format!("shared station link {} | used by: {}", self.pair(), self.users());
        match &self.existing {
            Some(existing) => o.push_str(&format!(" | existing: {}", describe_link("existing", existing))),
            None => o.push_str(" | no StnLink for this pair on the map yet"),
        }
        o.push_str(&format!(" | new: {}", describe_link("new", &self.new)));
        for d in &self.diffs {
            o.push_str(&format!(" | {} [{}]", d.field, d.detail));
        }
        o
    }
}

/// The local check of a single trip's way: nothing when each link may be written, else the
/// shared pairs that would change under the other trips on them. With another trip on a
/// pair and no link yet on the map there is nothing to compare: that is a conflict too, not
/// a reason to assume it is safe.
pub fn link_conflicts(tt: &Timetable, trip: &str, new_links: &[StnLink]) -> Vec<SharedLinkConflict> {
    conflicts_with(tt, trip, new_links, &HashSet::new())
}

/// The same check for a batch: `peers` (lower-case names) are the trips being written in the
/// same operation, whose links `conflicting_batch_routes` already compared with each other.
/// A pair shared only among them may be created; a pair used by anyone else is checked
/// against the map's link, and a missing one is a conflict there too.
pub(crate) fn link_conflicts_among(tt: &Timetable, trip: &str, new_links: &[StnLink], peers: &HashSet<String>) -> Vec<SharedLinkConflict> {
    conflicts_with(tt, trip, new_links, peers)
}

fn conflicts_with(tt: &Timetable, trip: &str, new_links: &[StnLink], peers: &HashSet<String>) -> Vec<SharedLinkConflict> {
    let mut out: Vec<SharedLinkConflict> = Vec::new();
    for new in new_links {
        let pair = (new.from_id, new.to_id);
        let mut trips = Vec::new();
        let mut outside = false;
        for other in &tt.trips {
            if other.name.eq_ignore_ascii_case(trip) || !station_pairs(other).contains(&pair) {
                continue;
            }
            trips.push(other.name.clone());
            if !peers.contains(&other.name.to_lowercase()) {
                outside = true;
            }
        }
        if trips.is_empty() {
            continue;
        }
        // (the trips written together were compared by the batch before it wrote them)
        if !outside {
            continue;
        }
        let existing: Vec<&StnLink> = tt.links.iter().filter(|l| (l.from_id, l.to_id) == pair).collect();
        if existing.iter().any(|l| *l == new) {
            continue;
        }
        let link = existing.first().map(|l| (*l).clone());
        // (another trip drives this pair but the map has no link to compare: not safe)
        // (a loop trip may name the same pair twice: one entry per candidate link is enough)
        if out.iter().any(|c| (c.from_id, c.to_id) == pair && c.new == *new) {
            continue;
        }
        let diffs = match &link {
            Some(existing) => link_diffs("existing", existing, "new", new),
            None => Vec::new(),
        };
        out.push(SharedLinkConflict {
            from_id: pair.0,
            to_id: pair.1,
            from_name: stop_label(tt, pair.0),
            to_name: stop_label(tt, pair.1),
            trips,
            existing: link,
            new: new.clone(),
            diffs,
        });
    }
    out
}

fn stop_label(tt: &Timetable, id: i64) -> String {
    tt.stop_name(id).map(str::to_string).unwrap_or_else(|| format!("#{id}"))
}

/// The complete input and output of a route write, for the log (not only a summary).
pub fn log_route(trip: &str, stations: &[i64], positions: &[Option<DVec3>], track: &[TrackEntry], links: &[StnLink], problems: &[Problem]) {
    log::info!("timetable: route of {trip}: stations {stations:?} at {positions:?}");
    let entries: Vec<String> = track.iter().map(|e| e.values.iter().map(|v| format!("{v:.3}")).collect::<Vec<_>>().join("/")).collect();
    log::info!("timetable: route of {trip}: track entries [{}]", entries.join(", "));
    for l in links {
        log::info!("timetable: route of {trip}: StnLink {}", describe_link("new", l));
    }
    log::info!("timetable: route of {trip}: problems {problems:?}");
}
