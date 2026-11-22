//! Which `.ttr` a trip would use, and how an automatic Type 2 route must leave the map's
//! tracks alone. The game reads a track named by the trip's `display_name` when that is set,
//! otherwise one named like the trip; an automatic Type 2 trip must have neither, or it is
//! driven by the old track instead of its station links.
//!
//! A track another trip still names is never removed or overwritten: that trip would lose its
//! way. A shared track is left for the other trip, and only a track that would become the
//! repaired trip's fallback after `display_name` is cleared can block the repair. A track
//! whose file says it was picked by hand is only ever removed when it is this trip's own.

use omsi_timetable::{Track, Trip};

use super::{track_name, Timetable};

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The track name the game would look for: the trimmed `display_name` when set, else the
/// trip's own name. (ASCII case does not matter to the game.)
pub(crate) fn referenced_name(trip: &Trip) -> String {
    let explicit = trip.display_name.trim();
    if explicit.is_empty() {
        trip.name.clone()
    } else {
        explicit.to_string()
    }
}

fn tracks_named(tt: &Timetable, name: &str) -> Vec<&Track> {
    tt.tracks.iter().filter(|t| same(&track_name(t), name)).collect()
}

/// Another retained trip that would still look for `name` after this repair (the trip being
/// repaired is not one of them).
pub(crate) fn other_trip_naming<'a>(tt: &'a Timetable, trip: &str, name: &str) -> Option<&'a Trip> {
    tt.trips.iter().find(|other| !same(&other.name, trip) && same(&referenced_name(other), name))
}

/// What an automatic route changes in the track files and the trip's `display_name`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct AutomaticPlan {
    /// The stems of the tracks to take away (the ones that would intercept the links).
    pub remove: Vec<String>,
    /// Clear `display_name` (it named an existing track, so the game would read that one).
    pub clear_display: bool,
    /// Tracks left for another trip, which still names them.
    pub left_shared: Vec<String>,
    /// Other tracks the old `display_name` named: left alone, they no longer intercept.
    pub left: Vec<String>,
}

/// The tracks an automatic route of `trip` must remove, or why it cannot be made safe.
/// The plan is checked before any link or track is written.
pub(crate) fn automatic_plan(tt: &Timetable, trip: &str) -> Result<AutomaticPlan, String> {
    let Some(me) = tt.trip(trip) else {
        return Err(format!("trip {trip} is not in the timetable"));
    };
    let fallback = me.name.clone();
    let explicit = me.display_name.trim().to_string();
    let mut plan = AutomaticPlan::default();

    // (a display_name that names an existing track is what the game reads now: it must be
    // cleared, or the trip stays on that track rather than its station links)
    if !explicit.is_empty() && !tracks_named(tt, &explicit).is_empty() {
        plan.clear_display = true;
        if !same(&explicit, &fallback) {
            if other_trip_naming(tt, &me.name, &explicit).is_some() {
                plan.left_shared.push(explicit.clone());
            } else {
                // (not shared: it is no longer this trip's way once display_name is cleared,
                // but it is not ours to delete either)
                plan.left.push(explicit.clone());
            }
        }
    }

    // (an automatic Type 2 trip keeps no same-name .ttr: the game falls back to the trip's
    // name when display_name is empty or cleared, and a track of that name would then drive
    // it. A track another trip still names is left for that trip; it only blocks the repair
    // when it would actually intercept this trip.)
    let fallback_tracks = tracks_named(tt, &fallback);
    if !fallback_tracks.is_empty() {
        let intercepts = explicit.is_empty() || plan.clear_display;
        if let Some(other) = other_trip_naming(tt, &me.name, &fallback) {
            if intercepts {
                return Err(format!(
                    "the track {fallback}.ttr is still used by trip {} after display_name is cleared",
                    other.name
                ));
            }
            plan.left_shared.push(fallback);
        } else {
            plan.remove.push(fallback);
        }
    }
    plan.remove.sort();
    plan.remove.dedup();
    plan.left_shared.sort();
    plan.left.sort();
    Ok(plan)
}

/// What a hand-picked route changes in the trip's `display_name`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct HandPlan {
    /// Clear `display_name`: a nonempty one makes the game look only for that name, so it
    /// must be cleared for `<trip>.ttr` to be read (unless it is already the trip's name).
    pub clear_display: bool,
}

/// Whether an explicit hand route may write `<trip>.ttr`: no other retained trip may name
/// that track.
pub(crate) fn hand_plan(tt: &Timetable, trip: &str) -> Result<HandPlan, String> {
    let Some(me) = tt.trip(trip) else {
        return Err(format!("trip {trip} is not in the timetable"));
    };
    let name = me.name.clone();
    if let Some(other) = other_trip_naming(tt, &me.name, &name) {
        return Err(format!("the track {name}.ttr is still used by trip {}", other.name));
    }
    let explicit = me.display_name.trim().to_string();
    // (a nonempty display_name makes the game look only for that name; even when no track of
    // it exists it does not fall back to `<trip>.ttr`, so the hand route needs it cleared
    // unless the name is already the trip's own)
    let clear_display = !explicit.is_empty() && !same(&explicit, &name);
    Ok(HandPlan { clear_display })
}

/// The track the game would read for `trip` now, and whether it is the trip's own name
/// (`display_name` nonempty and naming an existing track counts as a track source).
pub(crate) fn source<'a>(tt: &'a Timetable, trip: &str) -> (String, Option<&'a Track>) {
    let Some(t) = tt.trip(trip) else {
        return (String::new(), None);
    };
    let name = referenced_name(t);
    let track = tt.tracks.iter().find(|track| same(&track_name(track), &name));
    (name, track)
}
