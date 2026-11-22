//! A chrono scenario's own timetable (its `TTData`), edited in the Lines window as OMSI 2's
//! editor does: the map's timetable with the scenario's laid over it is put in its place for
//! as long as it lasts; afterwards the scenario keeps only what differs from the map's (a
//! line, trip or track of its own or changed, the stops and links when they differ), and a
//! line of the map taken away is taken off (`[deactivate_lines]`).

use std::path::{Path, PathBuf};

use omsi_timetable::TimetableData;

use crate::doc::Document;
use crate::timetable::{busstops_text, encode, line_text, stnlinks_text, track_name, track_text_of, trip_text, Timetable};

pub struct TtSession {
    /// The scenario's folder (in the map's own copy).
    pub dir: PathBuf,
    /// The map's own timetable, put back afterwards.
    map: Timetable,
}

/// The lines scenario `dir` takes off.
fn taken_off(dir: &Path) -> Vec<String> {
    omsi_cfg::CfgFile::read(crate::doc::safe::own_path(dir, "Chrono.cfg")).map(|f| omsi_map::ailists::parse_chrono_cfg(&f).deactivate_lines).unwrap_or_default()
}

/// The map's timetable `map` with scenario `dir`'s laid over it, as the game does: its stops
/// added, its links, trips, tracks and lines in place of those of the same name, the lines
/// it takes off gone.
pub fn merged(map: &Timetable, dir: &Path) -> Timetable {
    let mut s = TimetableData::load(dir);
    crate::timetable::reread_short_station_records(&mut s.trips);
    let mut t = map.clone();
    for b in s.bus_stops {
        if !t.stops.iter().any(|x| x.object_id == b.object_id && x.group == b.group) {
            t.stops.push(b);
        }
    }
    for l in s.stn_links {
        t.links.retain(|x| !(x.from_id == l.from_id && x.to_id == l.to_id));
        t.links.push(l);
    }
    for tr in s.trips {
        t.trips.retain(|x| !x.name.eq_ignore_ascii_case(&tr.name));
        t.trips.push(tr);
    }
    for tr in s.tracks {
        t.tracks.retain(|x| !track_name(x).eq_ignore_ascii_case(&track_name(&tr)));
        t.tracks.push(tr);
    }
    for l in s.lines {
        t.lines.retain(|x| !x.name.eq_ignore_ascii_case(&l.name));
        t.lines.push(l);
    }
    let off = taken_off(dir);
    t.lines.retain(|l| !off.iter().any(|o| o.trim().eq_ignore_ascii_case(l.name.trim())));
    t.dirty = false;
    t
}

pub fn begin(doc: &mut Document, dir: &Path) -> TtSession {
    let map = doc.timetable.clone();
    doc.timetable = merged(&map, dir);
    TtSession { dir: dir.to_path_buf(), map }
}

/// What the scenario's `TTData` and line list are to be, for an edited timetable `t` over
/// the map's `map`: (file name, text) to write, file names to take away, lines taken off.
pub fn diff(map: &Timetable, t: &Timetable) -> (Vec<(String, String)>, Vec<String>, Vec<String>) {
    let mut write: Vec<(String, String)> = Vec::new();
    let mut same: Vec<String> = Vec::new();
    let mut put = |name: String, text: String, base: Option<String>| {
        if base.as_deref() == Some(text.as_str()) {
            same.push(name);
        } else {
            write.push((name, text));
        }
    };
    for l in &t.lines {
        put(format!("{}.ttl", l.name), line_text(l), map.lines.iter().find(|x| x.name.eq_ignore_ascii_case(&l.name)).map(line_text));
    }
    for tr in &t.trips {
        put(format!("{}.ttp", tr.name), trip_text(tr), map.trips.iter().find(|x| x.name.eq_ignore_ascii_case(&tr.name)).map(trip_text));
    }
    for tr in &t.tracks {
        let name = track_name(tr);
        put(
            format!("{name}.ttr"),
            track_text_of(tr, t.by_hand(&name)),
            map.tracks
                .iter()
                .find(|x| track_name(x).eq_ignore_ascii_case(&name))
                .map(|x| track_text_of(x, map.by_hand(&track_name(x)))),
        );
    }
    put("Busstops.cfg".into(), busstops_text(&t.stops), Some(busstops_text(&map.stops)));
    put("StnLinks.cfg".into(), stnlinks_text(&t.links, &t.stops), Some(stnlinks_text(&map.links, &map.stops)));
    let off = map.lines.iter().filter(|l| !t.lines.iter().any(|x| x.name.eq_ignore_ascii_case(&l.name))).map(|l| l.name.clone()).collect();
    (write, same, off)
}

/// Put the map's timetable back and write the scenario's. How many files it has now.
pub fn end(doc: &mut Document, s: TtSession) -> anyhow::Result<usize> {
    let mut t = std::mem::replace(&mut doc.timetable, s.map);
    // (the holidays are the map's, whatever was changed meanwhile)
    if t.calendar != doc.timetable.calendar {
        doc.timetable.calendar = t.calendar.clone();
        doc.timetable.touch_calendar();
    }
    t.renumber_tiles(&doc.order);
    let mut map = doc.timetable.clone();
    map.renumber_tiles(&doc.order);
    let (write, same, off) = diff(&map, &t);
    let dir = crate::doc::safe::own_path(&s.dir, "TTData");
    std::fs::create_dir_all(&dir)?;
    for (name, text) in &write {
        crate::doc::safe::write_atomic(&crate::doc::safe::own_path(&dir, name), &encode(text))?;
    }
    for name in &same {
        let p = crate::doc::safe::own_path(&dir, name);
        if p.is_file() {
            std::fs::remove_file(p)?;
        }
    }
    // (what is no longer anywhere: its file goes too)
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let known = write.iter().any(|w| w.0.eq_ignore_ascii_case(&n));
        let ext = n.rsplit('.').next().unwrap_or("").to_lowercase();
        if !known && ["ttl", "ttp", "ttr"].contains(&ext.as_str()) {
            std::fs::remove_file(e.path())?;
        }
    }
    // the lines it takes off: the map's it no longer has
    let cfg = crate::doc::safe::own_path(&s.dir, "Chrono.cfg");
    let old = std::fs::read(&cfg).map(|b| omsi_cfg::decode_text(&b)).unwrap_or_default();
    let c = omsi_map::ailists::parse_chrono_cfg(&omsi_cfg::CfgFile::from_str(&cfg, &old));
    // (it may take off a line an earlier scenario brought: that stays taken off)
    let mut off = off;
    for l in &c.deactivate_lines {
        let of_map = map.lines.iter().any(|x| x.name.trim().eq_ignore_ascii_case(l.trim()));
        let back = t.lines.iter().any(|x| x.name.trim().eq_ignore_ascii_case(l.trim()));
        if !of_map && !back && !off.iter().any(|o| o.eq_ignore_ascii_case(l)) {
            off.push(l.clone());
        }
    }
    if c.deactivate_lines != off {
        crate::doc::safe::write_atomic(&cfg, &encode(&crate::ui_chrono::chrono_cfg_text(&old, c.start_date, c.end_date, &off)))?;
    }
    Ok(write.len())
}

/// A scenario's AI lists (its `ailists_#upd.cfg`, or a whole `ailists.cfg` of its own),
/// edited in the Traffic window: the map's groups with the scenario's vehicles added, as the
/// game reads them; afterwards the scenario keeps what the map's do not have.
pub struct ListsSession {
    pub dir: PathBuf,
    map: crate::traffic::Traffic,
}

/// The file a scenario keeps its AI lists in (the update, unless it has a whole one).
fn lists_file(dir: &Path) -> PathBuf {
    let whole = crate::doc::safe::own_path(dir, "ailists.cfg");
    if whole.is_file() {
        whole
    } else {
        crate::doc::safe::own_path(dir, "ailists_#upd.cfg")
    }
}

/// The map's groups with scenario `dir`'s laid over, as the game does: a group of the same
/// name gets the scenario's vehicles and type groups (a depot its depot file), others come in.
pub fn merged_groups(map: &[omsi_map::AiGroup], dir: &Path) -> Vec<omsi_map::AiGroup> {
    let mut groups = map.to_vec();
    if let Ok(extra) = omsi_map::AiLists::load(&lists_file(dir)) {
        for g in extra.groups {
            match groups.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&g.name) && x.is_depot == g.is_depot) {
                Some(base) => {
                    base.vehicles.extend(g.vehicles);
                    base.typgroups.extend(g.typgroups);
                    if g.hof.is_some() {
                        base.hof = g.hof;
                    }
                }
                None => groups.push(g),
            }
        }
    }
    groups
}

/// What a scenario's lists are to hold for edited groups `now` over the map's `map`: each
/// group's vehicles and type groups the map's lacks, and a depot file of its own.
pub fn groups_diff(map: &[omsi_map::AiGroup], now: &[omsi_map::AiGroup]) -> Vec<omsi_map::AiGroup> {
    let mut out = Vec::new();
    for g in now {
        match map.iter().find(|x| x.name.eq_ignore_ascii_case(&g.name) && x.is_depot == g.is_depot) {
            None => out.push(g.clone()),
            Some(m) => {
                // (as many of each as the map's lacks: an update may name a car again, to weigh it more)
                fn beyond<T: PartialEq + Clone>(now: &[T], map: &[T]) -> Vec<T> {
                    let mut left: Vec<&T> = map.iter().collect();
                    now.iter().filter(|v| match left.iter().position(|m| *m == *v) {
                        Some(i) => {
                            left.remove(i);
                            false
                        }
                        None => true,
                    }).cloned().collect()
                }
                let vehicles = beyond(&g.vehicles, &m.vehicles);
                let typgroups = beyond(&g.typgroups, &m.typgroups);
                let hof = (g.hof != m.hof).then(|| g.hof.clone()).flatten();
                if !vehicles.is_empty() || !typgroups.is_empty() || hof.is_some() {
                    out.push(omsi_map::AiGroup { name: g.name.clone(), vehicles, typgroups, hof, is_depot: g.is_depot });
                }
            }
        }
    }
    out
}

pub fn begin_lists(doc: &mut Document, dir: &Path) -> ListsSession {
    let map = doc.traffic.clone();
    doc.traffic.groups = merged_groups(&map.groups, dir);
    ListsSession { dir: dir.to_path_buf(), map }
}

/// Put the map's lists back and write the scenario's. How many groups it changes.
pub fn end_lists(doc: &mut Document, s: ListsSession) -> anyhow::Result<usize> {
    // (only the groups were the scenario's: what else was changed meanwhile is the map's)
    let mut edited = std::mem::replace(&mut doc.traffic, s.map);
    let now = std::mem::replace(&mut edited.groups, doc.traffic.groups.clone());
    doc.traffic = edited;
    let d = groups_diff(&doc.traffic.groups, &now);
    let p = lists_file(&s.dir);
    if d.is_empty() {
        if p.is_file() {
            std::fs::remove_file(&p)?;
        }
    } else {
        std::fs::create_dir_all(&s.dir)?;
        crate::doc::safe::write_atomic(&p, &encode(&crate::traffic::ailists_text(&d)))?;
    }
    Ok(d.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Berlin-Spandau's timetable with each scenario laid over it gives nothing to write
    /// when nothing was changed: the scenario keeps what it had.
    #[test]
    fn an_unchanged_scenario_timetable_writes_only_what_it_has() {
        let Some(omsi) = crate::install::find_omsi_root(&Default::default()) else { return };
        let map_dir = omsi.join("maps").join("Berlin-Spandau");
        if !map_dir.is_dir() {
            return;
        }
        let _map = crate::timetable::tests::one_map();
        let map = Timetable::load(&map_dir, &[]);
        let scen = map_dir.join("Chrono").join("0490_56 via Wiesendamm");
        let t = merged(&map, &scen);
        let (write, _, off) = diff(&map, &t);
        let names: Vec<String> = write.iter().map(|w| w.0.to_lowercase()).collect();
        // its two trips differ from the map's; nothing else of the map's lines
        assert!(names.iter().any(|n| n.ends_with(".ttp")), "{names:?}");
        assert!(!names.iter().any(|n| n.ends_with(".ttl")), "{names:?}");
        assert!(off.is_empty());
        // a line taken away is taken off
        let mut t2 = t.clone();
        let gone = t2.lines.remove(0).name;
        assert_eq!(diff(&map, &t2).2, vec![gone]);
    }

    /// Every scenario of Berlin-Spandau with AI lists, laid over the map's and taken apart
    /// again, keeps the same updates.
    #[test]
    fn scenario_lists_round_trip() {
        let Some(omsi) = crate::install::find_omsi_root(&Default::default()) else { return };
        let map_dir = omsi.join("maps").join("Berlin-Spandau");
        if !map_dir.is_dir() {
            return;
        }
        let map = crate::traffic::Traffic::load(&map_dir);
        let mut n = 0;
        for d in std::fs::read_dir(map_dir.join("Chrono")).unwrap().flatten().map(|e| e.path()).filter(|p| lists_file(p).is_file()) {
            let own = omsi_map::AiLists::load(&lists_file(&d)).unwrap().groups;
            let back = groups_diff(&map.groups, &merged_groups(&map.groups, &d));
            assert_eq!(merged_groups(&map.groups, &d), {
                let mut g = map.groups.clone();
                for x in groups_diff(&map.groups, &merged_groups(&map.groups, &d)) {
                    match g.iter_mut().find(|y| y.name.eq_ignore_ascii_case(&x.name) && y.is_depot == x.is_depot) {
                        Some(b) => {
                            b.vehicles.extend(x.vehicles);
                            b.typgroups.extend(x.typgroups);
                            if x.hof.is_some() {
                                b.hof = x.hof;
                            }
                        }
                        None => g.push(x),
                    }
                }
                g
            }, "{}", d.display());
            assert!(back.len() <= own.len(), "{}", d.display());
            n += 1;
        }
        assert!(n > 5);
    }

    /// A scenario's timetable opened and closed again without a change keeps the lines it
    /// takes off (one of them an earlier scenario's line) and what it changes.
    #[test]
    fn a_scenario_timetable_round_trips() {
        let Some(omsi) = crate::install::find_omsi_root(&Default::default()) else { return };
        let map_dir = omsi.join("maps").join("Berlin-Spandau");
        let src = map_dir.join("Chrono").join("0550_LinieEDeaktiv");
        if !src.is_dir() {
            return;
        }
        let _map = crate::timetable::tests::one_map();
        let tmp = std::env::temp_dir().join(format!("openmapper-tt-scen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        crate::doc::save::copy_missing(&src, &tmp).unwrap();
        let roots = crate::install::Roots { omsi: omsi.clone(), content: std::env::temp_dir().join("openmapper-no-content") };
        let mut doc = Document::load(&roots, &map_dir.join("global.cfg"), |_| {}).unwrap();
        let before = taken_off(&tmp);
        assert!(!before.is_empty());
        let shown = merged(&doc.timetable, &tmp);
        let s = begin(&mut doc, &tmp);
        end(&mut doc, s).unwrap();
        assert_eq!(taken_off(&tmp), before);
        let again = merged(&doc.timetable, &tmp);
        assert_eq!(again.lines, shown.lines);
        assert_eq!(again.trips, shown.trips);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
