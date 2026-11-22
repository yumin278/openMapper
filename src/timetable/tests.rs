use glam::DVec3;

use super::network::{Files, Network, PathKey};
use super::*;

fn omsi_root() -> Option<std::path::PathBuf> {
    let r = crate::install::find_omsi_root(&crate::install::Prefs::default());
    if r.is_none() {
        eprintln!("no OMSI 2 installation: skipped");
    }
    r
}

/// The map's grid is global (`omsi_map::configure_grid`): one map at a time, for every
/// test of the crate that reads or makes one.
pub(crate) fn one_map() -> crate::doc::OneMap {
    crate::doc::one_map()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("openmapper-tt-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("TTData")).unwrap();
    d
}

/// Everything written as it was read, read again with openOMSI's reader.
fn write_all(tt: &Timetable, dir: &Path) {
    let d = dir.join("TTData");
    std::fs::write(d.join("Busstops.cfg"), encode(&busstops_text(&tt.stops))).unwrap();
    std::fs::write(d.join("StnLinks.cfg"), encode(&stnlinks_text(&tt.links, &tt.stops))).unwrap();
    for t in &tt.trips {
        std::fs::write(d.join(format!("{}.ttp", t.name)), encode(&trip_text(t))).unwrap();
    }
    for t in &tt.tracks {
        std::fs::write(d.join(format!("{}.ttr", track_name(t))), encode(&track_text(t))).unwrap();
    }
    for l in &tt.lines {
        std::fs::write(d.join(format!("{}.ttl", l.name)), encode(&line_text(l))).unwrap();
    }
}

fn same(a: &Timetable, b: &TimetableData, what: &str) {
    assert_eq!(a.stops, b.bus_stops, "{what}: stops");
    assert_eq!(a.links.len(), b.stn_links.len(), "{what}: links");
    for (x, y) in a.links.iter().zip(&b.stn_links) {
        assert_eq!((x.from_id, x.to_id, x.params, &x.entries), (y.from_id, y.to_id, y.params, &y.entries), "{what}: link");
        assert!((x.length - y.length).abs() < 1e-6);
    }
    assert_eq!(a.trips.len(), b.trips.len(), "{what}: trips");
    for (x, y) in a.trips.iter().zip(&b.trips) {
        assert_eq!((&x.name, &x.display_name, &x.terminus, &x.line, x.train_reverse), (&y.name, &y.display_name, &y.terminus, &y.line, y.train_reverse), "{what}: trip");
        assert_eq!((&x.stations, &x.stations_legacy, &x.profiles), (&y.stations, &y.stations_legacy, &y.profiles), "{what}: trip {}", x.name);
    }
    assert_eq!(a.tracks.len(), b.tracks.len(), "{what}: tracks");
    for (x, y) in a.tracks.iter().zip(&b.tracks) {
        assert_eq!(x.entries, y.entries, "{what}: track {}", track_name(x));
    }
    assert_eq!(a.lines.len(), b.lines.len(), "{what}: lines");
    for (x, y) in a.lines.iter().zip(&b.lines) {
        assert_eq!((&x.name, x.user_allowed, x.priority, &x.tours), (&y.name, y.user_allowed, y.priority, &y.tours), "{what}: line");
    }
}

#[test]
fn global_type2_conversion_saves_only_the_converted_trips() {
    let out = scratch("global-type2");
    let dir = out.join("TTData");
    let trip = |track: &str, stations: &str| {
        format!("[trip]\n{track}\nTerminus\n66\n\n{stations}\n[profile]\nstandard\n18.000\n\n[profile_man_dep_time]\n2\n12.250\n[profile_otherstopping]\n1\n2\n")
    };
    let legacy = "[station]\n5521173\n5\nSouth Valley Railway Station\n271\n0.781\n41.792\n11.642\n150.000\n\n[station]\n5521015\n119\nBus Rally\n342\n1.812\n4189.310\n5.810\n150.000\n\n[station]\n5521173\n225\nSouth Valley Railway Station\n271\n0.000\n7912.901\n16.330\n0.000\n";
    let mut originals = Vec::new();
    for (file, text) in [
        ("66.ttp", trip("66", legacy)),
        ("variant.ttp", trip("66", legacy)),
        ("converted.ttp", trip("", "[station_typ2]\n42\n\n[station_typ2]\n43\n")),
        ("invalid.ttp", trip("66", &legacy.replace("5521015", "not an id"))),
        ("mixed.ttp", trip("66", &format!("[station_typ2]\n42\n\n{legacy}"))),
        ("rail.ttp", trip("66", &format!("[trainreverse]\n\n{legacy}"))),
        ("66.ttr", "[track_entry]\n1756623\n58\n271\n144\n3.4700000000\n0\n\n".into()),
        ("66.ttl", "[userallowed]\n\n[priority]\n1\n\n[newtour]\n1\nBusses\n1023\n\n[addtrip]\n66\n0\n360.000\n".into()),
        ("StnLinks.cfg", "[StnLink]\n1\n42\n43\n0\n0\n0\n1\n0\n0\n\n[StnLink_entry]\n1756623\n58\n0\n1\n-1\n0\n0\n".into()),
        ("Busstops.cfg", "[busstop]\nBus Rally\n0\n5521015\n0.0000000000\n0\n0\n\n".into()),
    ] {
        let bytes = encode(&text);
        std::fs::write(dir.join(file), &bytes).unwrap();
        if !matches!(file, "66.ttp" | "variant.ttp") {
            originals.push((file, bytes));
        }
    }
    let mut tt = Timetable::load(&out, &[]);
    let before = tt.trips.clone();
    tt.reroute.insert("66".into());
    assert_eq!(tt.convert_all_to_type2(), (2, 3));
    assert!(tt.reroute.is_empty());
    assert_eq!(tt.convert_all_to_type2(), (0, 3));
    save_to(&mut tt, &out, &[]).unwrap();
    assert!(tt.type2_converted.is_empty());
    let back = Timetable::load(&out, &[]);
    for t in before {
        let after = back.trip(&t.name).unwrap();
        if matches!(t.name.as_str(), "66" | "variant") {
            println!("{} before:\n{}\n{} after:\n{}", t.name, trip_text(&t), t.name, trip_text(after));
            assert_eq!(after, &Trip { display_name: String::new(), stations_legacy: Vec::new(), stations: vec![5521173, 5521015, 5521173], ..t });
        } else {
            assert_eq!(after, &t);
        }
    }
    for (file, bytes) in originals {
        assert_eq!(std::fs::read(dir.join(file)).unwrap(), bytes, "{file}");
    }
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn type2_exclusions_keep_legacy_files_until_the_trip_is_selected() {
    let out = scratch("type2-exclusions");
    let dir = out.join("TTData");
    let legacy = |id| format!("[station]\n{id}\n0\nStop {id}\n0\n0\n0\n0\n0\n\n");
    let trip = |track, stops: &str| format!("[trip]\n{track}\nTerminus\n66\n\n{stops}[profile]\nstandard\n18.000\n");
    let inputs = [
        ("zero.ttp", trip("tram-track", "")),
        ("one.ttp", trip("tram-track", &legacy(11))),
        ("tram.ttp", trip("tram-track", &(legacy(11) + &legacy(12)))),
        ("bus.ttp", trip("bus-track", &(legacy(21) + &legacy(22)))),
        ("draft.ttp", trip("", "")),
        ("tram-track.ttr", "[track_entry]\n1756623\n58\n0\n144\n3.4700000000\n0\n\n".into()),
        ("StnLinks.cfg", "Time Table StnLinkList File\n".into()),
    ];
    for (file, text) in &inputs {
        std::fs::write(dir.join(file), encode(text)).unwrap();
    }
    let mut tt = Timetable::load(&out, &[]);
    let baseline = tt.clone();
    for (name, expected) in [("zero", true), ("one", true), ("tram", false), ("bus", false)] {
        let trip = tt.trip(name).unwrap();
        assert!(is_type1(trip));
        assert_eq!(tt.excludes_type2(trip), expected, "{name}");
    }
    assert!(!is_type1(tt.trip("draft").unwrap()));
    tt.set_type2_exclusion(baseline.trip("tram").unwrap(), true);
    assert!(!tt.dirty && tt.is_dirty());
    let mut restored = baseline.clone();
    restored.restored(&tt);
    assert!(!restored.excludes_type2(restored.trip("tram").unwrap()));
    assert!(restored.is_dirty());
    save_to(&mut tt, &out, &[]).unwrap();
    assert!(!tt.is_dirty());
    let metadata_path = convert::exclusions_path(&out);
    let metadata_before = std::fs::read_to_string(&metadata_path).unwrap();
    println!("Exclusions before conversion:\n{metadata_before}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&metadata_before).unwrap(), serde_json::json!({
        "excluded": ["one", "tram", "zero"], "overrides": {"tram": true},
    }));
    for (file, original) in &inputs {
        assert_eq!(std::fs::read(dir.join(file)).unwrap(), encode(original), "{file}");
    }
    let mut tt = Timetable::load(&out, &[]);
    assert!(tt.excludes_type2(tt.trip("tram").unwrap()));
    assert_eq!(tt.convert_all_to_type2(), (1, 3));
    save_to(&mut tt, &out, &[]).unwrap();
    for (file, original) in &inputs {
        if *file != "bus.ttp" {
            assert_eq!(std::fs::read(dir.join(file)).unwrap(), encode(original), "{file}");
        }
    }
    for name in ["zero", "one"] {
        tt.set_type2_exclusion(baseline.trip(name).unwrap(), false);
    }
    save_to(&mut tt, &out, &[]).unwrap();
    let metadata_unchecked = std::fs::read_to_string(&metadata_path).unwrap();
    println!("Exclusions after unchecking short trips:\n{metadata_unchecked}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&metadata_unchecked).unwrap(), serde_json::json!({
        "excluded": ["tram"], "overrides": {"one": false, "tram": true, "zero": false},
    }));
    let mut tt = Timetable::load(&out, &[]);
    assert!(!tt.excludes_type2(tt.trip("zero").unwrap()));
    assert!(!tt.excludes_type2(tt.trip("one").unwrap()));
    assert_eq!(tt.convert_all_to_type2(), (2, 1));
    assert_eq!(tt.convert_all_to_type2(), (0, 1));
    save_to(&mut tt, &out, &[]).unwrap();
    let metadata_after = std::fs::read_to_string(&metadata_path).unwrap();
    println!("Exclusions after conversion:\n{metadata_after}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&metadata_after).unwrap(), serde_json::json!({
        "excluded": ["tram"], "overrides": {"tram": true},
    }));
    let reloaded = Timetable::load(&out, &[]);
    for (file, original) in &inputs {
        let bytes = std::fs::read(dir.join(file)).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        println!("{file} before:\n{original}\n{file} after:\n{text}");
        if !matches!(*file, "zero.ttp" | "one.ttp" | "bus.ttp") {
            assert_eq!(bytes, encode(original), "{file}");
        }
    }
    for (name, stations) in [("zero", vec![]), ("one", vec![11]), ("bus", vec![21, 22])] {
        let trip = reloaded.trip(name).unwrap();
        assert!(!is_type1(trip));
        assert!(trip.display_name.is_empty() && trip.stations_legacy.is_empty());
        assert_eq!(trip.stations, stations);
        assert_eq!(trip.profiles, baseline.trip(name).unwrap().profiles);
    }
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn timetable_writes_names_for_stops_attached_to_splines() {
    let _map = one_map();
    let out = scratch("attached-stop-name");
    let roots = crate::install::Roots::new(out.clone());
    let mut doc = Document::new_map(&roots, "attached-stop", "Attached stop", 1);
    let k = doc.order[0];
    let tile = &mut doc.tiles.get_mut(&k).unwrap().tile;
    let file = "Sceneryobjects\\Generic\\bus_stop.sco";
    let labels = |name: &str| vec![name.into(), "0".into(), "0".into(), "15.0".into()];
    tile.objects.push(omsi_map::MapObject { file: file.into(), id: 2544995, extra: labels("Aberdeen Tunnel"), ..Default::default() });
    tile.spline_attachments.push(omsi_map::SplineAttachment { file: file.into(), id: 2353752, strings: labels("Ocean Park"), ..Default::default() });
    let ids = [(k, 2544995), (k, 2353752)];
    let before = stop_entries(&doc, &ids[..1]);
    let after = stop_entries(&doc, &ids);
    println!("Before:\n{}\nAfter:\n{}", busstops_text(&before), busstops_text(&after));
    assert_eq!(after, [before[0].clone(), BusStopEntry { name: "Ocean Park".into(), group: 0, object_id: 2353752, params: [15.0, 0.0, 0.0] }]);
    doc.timetable.sync_stops(after);
    save_to(&mut doc.timetable, &out, &doc.order).unwrap();
    let back = Timetable::load(&out, &doc.order);
    assert_eq!(back.stop_name(2353752), Some("Ocean Park"));
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn stock_timetables_round_trip() {
    let Some(root) = omsi_root() else { return };
    for m in ["Grundorf", "Berlin-Spandau"] {
        let src = root.join("maps").join(m);
        if !src.join("TTData").is_dir() {
            continue;
        }
        let tt = Timetable::load(&src, &[]);
        assert!(!tt.lines.is_empty(), "{m}: lines");
        let out = scratch(m);
        write_all(&tt, &out);
        let back = TimetableData::load(&out);
        assert!(back.errors.is_empty(), "{:?}", back.errors);
        same(&tt, &back, m);
        eprintln!("{m}: {} lines, {} trips, {} tracks, {} links, {} stops read back the same", tt.lines.len(), tt.trips.len(), tt.tracks.len(), tt.links.len(), tt.stops.len());
        let _ = std::fs::remove_dir_all(&out);
    }
}

#[test]
fn only_what_changed_is_written() {
    let mut tt = Timetable::default();
    let out = scratch("changed");
    let line = tt.add_line("7");
    let trip = tt.add_trip(&line, "7_A-B", vec![11, 12]);
    tt.lines[0].tours.push(omsi_timetable::Tour { number: "1".into(), ai_group: "Busses".into(), extra: ALL_DAYS.to_string(), trips: vec![omsi_timetable::TourTrip { trip: trip.clone(), profile: 0, departure: 6.0 * 60.0 }] });
    save_to(&mut tt, &out, &[]).unwrap();
    let back = TimetableData::load(&out);
    assert_eq!(back.lines.len(), 1);
    assert_eq!(back.lines[0].tours[0].trips[0].trip, "7_A-B");
    assert_eq!(back.trips[0].stations, vec![11, 12]);
    assert!(!tt.dirty);
    // a rename takes the old files away
    tt.rename_trip("7_A-B", "7_B-A").unwrap();
    tt.rename_line("7", "8").unwrap();
    save_to(&mut tt, &out, &[]).unwrap();
    let back = TimetableData::load(&out);
    assert_eq!(back.trips.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["7_B-A"]);
    assert_eq!(back.lines.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["8"]);
    assert_eq!(back.lines[0].tours[0].trips[0].trip, "7_B-A");
    assert_eq!(back.trips[0].line, "8");
    tt.delete_line("8");
    save_to(&mut tt, &out, &[]).unwrap();
    let back = TimetableData::load(&out);
    assert!(back.lines.is_empty() && back.trips.is_empty());
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn open_trip_in_text_editor_uses_the_saved_station_list() {
    let source = scratch("open-trip-source");
    let working = scratch("open-trip-working");
    let trip = "73E_ba-gb";
    let mut tt = Timetable::default();
    let line = tt.add_line("73E");
    tt.add_trip(&line, trip, vec![11, 12, 13]);
    let look_up = |source_dir: Option<&Path>, fresh, name| saved_trip_path(&working, source_dir, fresh, name);

    std::fs::write(working.join("TTData").join(format!("{trip}.ttl")), "not the trip file").unwrap();
    let before = look_up(None, false, trip);
    println!("Before saving {trip} with stations [11, 12, 13]: {before:?}");
    assert_eq!(before, None);

    save_to(&mut tt, &source, &[]).unwrap();
    let source_file = source.join("TTData").join(format!("{trip}.ttp"));
    assert_eq!(look_up(Some(&source), false, trip), Some(source_file.clone()));
    assert_eq!(look_up(Some(&source), true, trip), None);

    tt.trip_mut(trip).unwrap().stations.reverse();
    tt.touch_trip(trip);
    save_to(&mut tt, &working, &[]).unwrap();
    let working_file = working.join("TTData").join(format!("{trip}.ttp"));
    assert_eq!(look_up(Some(&source), false, trip), Some(source_file.clone()));
    assert_eq!(look_up(None, false, "73e_BA-GB"), Some(working_file.clone()));
    assert_eq!(look_up(None, false, "73E_unsaved"), None);

    for (path, expected) in [(source_file, vec![11, 12, 13]), (working_file, vec![13, 12, 11])] {
        let text = std::fs::read_to_string(&path).unwrap();
        println!("Opened {}:\n{text}", path.display());
        let trip = omsi_timetable::Trip::load(&path).unwrap();
        assert_eq!(trip.stations, expected);
    }
    std::fs::remove_dir_all(source).unwrap();
    std::fs::remove_dir_all(working).unwrap();
}

/// A tour without a depot (a new map's ailists.cfg has none) keeps its days.
#[test]
fn a_tour_without_depot_reads_back() {
    let mut tt = Timetable::default();
    let line = tt.add_line("3");
    tt.lines[0].tours.push(omsi_timetable::Tour { number: "1".into(), ai_group: String::new(), extra: "799".into(), trips: vec![omsi_timetable::TourTrip { trip: "3_A".into(), profile: 0, departure: 400.0 }] });
    let back = omsi_timetable::Line::parse(&omsi_cfg::CfgFile::from_str(format!("{line}.ttl"), &line_text(&tt.lines[0])));
    assert_eq!(back.tours, tt.lines[0].tours);
}

#[test]
fn text_that_1252_cannot_hold_is_utf16() {
    assert_eq!(encode("Bf. Zoo\n"), b"Bf. Zoo\r\n".to_vec());
    assert_eq!(encode("Straße"), vec![b'S', b't', b'r', b'a', 0xDF, b'e']);
    let b = encode("Вокзал");
    assert_eq!(&b[..2], &[0xFF, 0xFE]);
    assert_eq!(omsi_cfg::decode_text(&b), "Вокзал");
}

#[test]
fn lengths_as_omsi_writes_them() {
    assert_eq!(route::sig3(19.853), 19.9);
    assert_eq!(route::sig3(271.6), 272.0);
    assert_eq!(route::sig3(2.2834), 2.28);
    assert_eq!(route::sig3(0.0), 0.0);
}

/// The times at the stops follow the distance between the fixed ones.
#[test]
fn stop_times_spread_by_distance() {
    let mut p = omsi_timetable::TripProfile { name: "standard".into(), factor: 8.0, ..Default::default() };
    let t = stop_times(3, &[100.0, 300.0], &p);
    assert_eq!(t.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(), vec![(0.0, 0.0), (2.0, 2.0), (8.0, 8.0)]);
    set_stop_time(&mut p, 4, 1, Some(5.0));
    let t = stop_times(4, &[100.0, 100.0, 100.0], &p);
    assert_eq!(t.iter().map(|x| x.1).collect::<Vec<_>>(), vec![0.0, 5.0, 6.5, 8.0]);
    assert!(t[1].2 && !t[2].2);
    assert_eq!(p.man_dep_time, vec![(1, 5.0)]);
    set_stop_time(&mut p, 4, 3, Some(9.0));
    assert_eq!(p.man_arr_time, vec![(3, 9.0)]);
    set_stop_time(&mut p, 4, 1, None);
    assert!(p.man_dep_time.is_empty());
}

/// Holidays.txt written and read back the same through openOMSI's reader; Grundorf's too.
#[test]
fn holidays_round_trip() {
    let c = omsi_map::Calendar {
        ranges: vec![omsi_map::HolidayRange { start: 20260720, end: 20260830, name: "Summer".into() }],
        holidays: vec![omsi_map::Holiday { date: 20261225, name: "Christmas".into() }, omsi_map::Holiday { date: 20270101, name: "New Year".into() }],
    };
    let back = omsi_map::Calendar::parse(&omsi_cfg::CfgFile::from_str("Holidays.txt", &holidays_text(&c)));
    assert_eq!(back, c);
    if let Some(root) = omsi_root() {
        let p = root.join("maps").join("Grundorf").join("Holidays.txt");
        if let Ok(g) = omsi_map::Calendar::load(&p) {
            let mut sorted = g.clone();
            sorted.ranges.sort_by_key(|r| r.start);
            sorted.holidays.sort_by_key(|h| h.date);
            let mut again = omsi_map::Calendar::parse(&omsi_cfg::CfgFile::from_str("Holidays.txt", &holidays_text(&g)));
            again.ranges.sort_by_key(|r| r.start);
            again.holidays.sort_by_key(|h| h.date);
            assert_eq!(again, sorted);
            assert!(g.holidays.len() > 50);
        }
    }
}

#[test]
fn clock_times() {
    assert_eq!(clock(247.0), "4:07");
    assert_eq!(clock(24.0 * 60.0 + 10.0), "24:10");
}

/// Grundorf's network as openMapper reads it names the paths of the stock tracks with
/// the same numbers OMSI wrote, and the route found through line 76's stops runs where the
/// stock one does.
#[test]
fn grundorf_routes_like_omsi() {
    let Some(root) = omsi_root() else { return };
    let cfg = root.join("maps").join("Grundorf").join("global.cfg");
    if !cfg.is_file() {
        return;
    }
    let _map = one_map();
    let roots = crate::install::Roots::new(root.clone());
    let doc = Document::load(&roots, &cfg, |_| {}).unwrap();
    let mut files = Files::new(vec![root.clone()]);
    let net = Network::build(&doc, &mut files);
    assert!(net.lanes.len() > 100, "{} lanes", net.lanes.len());
    let tt = &doc.timetable;
    for trip in ["76_BH-Kk", "76_Kk-BH"] {
        let track = tt.track(trip).unwrap();
        let mut hit = 0;
        for e in &track.entries {
            let v = &e.values;
            let key = PathKey { tile: doc.order[v[2] as usize], id: v[0] as i64, path: v[1] as usize };
            let Some(l) = net.by_key.get(&key).and_then(|l| l.first()) else { continue };
            let l = &net.lanes[*l];
            assert_eq!(l.internal as f64, v[3], "{trip}: internal number of {key:?}");
            assert!((route::sig3(l.file_length) - v[4]).abs() < 1e-9, "{trip}: length of {key:?}: {} vs {}", l.file_length, v[4]);
            hit += 1;
        }
        assert!(hit * 10 >= track.entries.len() * 9, "{trip}: {hit} of {} entries on the network", track.entries.len());
        // the route through the trip's stops
        let t = tt.trip(trip).unwrap();
        let pos: Vec<Option<DVec3>> = t.stations.iter().map(|id| stop_world(&doc, &mut files, *id)).collect();
        let r = route::find(&net, &pos, doc.global.left_hand_traffic, None);
        assert!(r.ok(), "{trip}: {:?}", r.problems);
        let ours: Vec<(f64, f64, f64)> = r.track(&net, &doc.order).iter().map(|e| (e.values[0], e.values[1], e.values[2])).collect();
        let stock: Vec<(f64, f64, f64)> = track.entries.iter().map(|e| (e.values[0], e.values[1], e.values[2])).collect();
        let common = ours.iter().filter(|k| stock.contains(k)).count();
        eprintln!("{trip}: {} lanes found, {} in the stock track of {}, {:.0} m", ours.len(), common, stock.len(), r.length());
        assert!(common * 10 >= ours.len() * 8, "{trip}: ours {ours:?}\nstock {stock:?}");
        // the station links' lengths as OMSI measured them
        let links = r.links(&net, &doc.order, &t.stations);
        for l in &links {
            if let Some(s) = tt.links.iter().find(|x| x.from_id == l.from_id && x.to_id == l.to_id) {
                eprintln!("  {} -> {}: {:.1} m (OMSI {:.1} m)", l.from_id, l.to_id, l.length, s.length);
                assert!((l.length - s.length).abs() < s.length * 0.15 + 15.0, "link {} -> {}: {} vs {}", l.from_id, l.to_id, l.length, s.length);
            }
        }
    }
}


/// A line made the way the Lines window makes one, on Grundorf: its stops routed, saved
/// next to the stock files and read back by openOMSI's reader, its track where line 76's runs.
#[test]
fn a_new_line_on_grundorf_saves_and_reads_back() {
    let Some(root) = omsi_root() else { return };
    let cfg = root.join("maps").join("Grundorf").join("global.cfg");
    if !cfg.is_file() {
        return;
    }
    let _map = one_map();
    let roots = crate::install::Roots::new(root.clone());
    let doc = Document::load(&roots, &cfg, |_| {}).unwrap();
    let mut files = Files::new(vec![root.clone()]);
    let net = Network::build(&doc, &mut files);
    let mut tt = doc.timetable.clone();
    let out = scratch("newline");
    // the stock files first, as a save of an installation map copies them
    write_all(&tt, &out);
    let stops = vec![108, 104, 129, 763];
    let line = tt.add_line("99");
    let trip = tt.add_trip(&line, "99_test", stops.clone());
    let pos: Vec<Option<DVec3>> = stops.iter().map(|id| stop_world(&doc, &mut files, *id)).collect();
    let r = route::find(&net, &pos, false, None);
    assert!(r.ok(), "{:?}", r.problems);
    let route_track = r.track(&net, &doc.order);
    // (the way found is where line 76's stock track runs: the automatic Type 2 write keeps
    // the station links, not a .ttr, so compare the calculated track with the stock one now)
    let stock = tt.track("76_BH-Kk").map(|t| t.entries.clone()).unwrap();
    let key = |e: &TrackEntry| (e.values[0], e.values[1], e.values[2], e.values[3]);
    let n = route_track.len();
    assert!(n > 5);
    assert_eq!(route_track.iter().map(key).collect::<Vec<_>>(), stock[..n].iter().map(key).collect::<Vec<_>>());
    tt.set_route(&trip, route_track, r.links(&net, &doc.order, &stops)).unwrap_or_else(|c| panic!("the new line's route was refused: {c:?}"));
    tt.lines.iter_mut().find(|l| l.name == line).unwrap().tours.push(omsi_timetable::Tour {
        number: "1".into(),
        ai_group: "Busses".into(),
        extra: ALL_DAYS.to_string(),
        trips: vec![omsi_timetable::TourTrip { trip: trip.clone(), profile: 0, departure: 8.0 * 60.0 }],
    });
    let ids: Vec<(TileKey, i64)> = net.stops.iter().map(|s| (s.0, s.1)).collect();
    tt.sync_stops(stop_entries(&doc, &ids));
    save_to(&mut tt, &out, &doc.order).unwrap();
    let back = TimetableData::load(&out);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    // the stock line is still there, the new one beside it
    assert!(back.lines.iter().any(|l| l.name == "76"));
    let l = back.lines.iter().find(|l| l.name == "99").unwrap();
    assert_eq!(l.tours[0].trips[0].trip, "99_test");
    assert_eq!(back.trip("99_test").unwrap().stations, stops);
    // the stock line's first lanes are where the new route was found
    assert!(!back.tracks.iter().any(|t| track_name(t) == "99_test"), "an automatic Type 2 trip writes no .ttr");
    // and OMSI 2 drives it by the station links
    for w in stops.windows(2) {
        assert!(back.stn_links.iter().any(|x| x.from_id == w[0] && x.to_id == w[1]), "link {w:?}");
    }
    assert!(back.bus_stops.iter().any(|s| s.object_id == 763 && s.name == "Elsterplatz"));
    let _ = std::fs::remove_dir_all(&out);
}

/// On a map made in openMapper: a straight two-lane road of two linked splines with a stop
/// on either side; the bus keeps right, so the way runs north past the east stops and south
/// past the west ones.
#[test]
fn a_new_map_routes_on_the_right() {
    let Some(root) = omsi_root() else { return };
    let road = "Splines\\Marcel\\str_2spur_8m_sedan1.sli";
    let mut files = Files::new(vec![root.clone()]);
    if files.get(road).is_none() {
        return;
    }
    let _map = one_map();
    let roots = crate::install::Roots::new(root.clone());
    let mut doc = Document::new_map(&roots, "tt-test", "tt-test", 1);
    let k = (0, 0);
    let t = &mut doc.tiles.get_mut(&k).unwrap().tile;
    let spline = |id: i64, y: f64, prev: i64, next: i64| omsi_map::MapSpline { file: road.into(), id, prev_id: prev, next_id: next, pos: [150.0, y, 0.0], heading: 0.0, length: 50.0, ..Default::default() };
    t.splines.push(spline(1, 50.0, 0, 2));
    t.splines.push(spline(2, 100.0, 1, 0));
    let stop = |id: i64, x: f64, y: f64, name: &str| omsi_map::MapObject { file: "Sceneryobjects\\Generic\\bus_stop.sco".into(), id, pos: [x, y, 0.0], flag: 7, extra: vec![name.into(), "0".into(), "0".into(), "10".into()], ..Default::default() };
    t.objects.push(stop(10, 156.0, 60.0, "South East"));
    t.objects.push(stop(11, 156.0, 140.0, "North East"));
    t.objects.push(stop(12, 144.0, 140.0, "North West"));
    t.objects.push(stop(13, 144.0, 60.0, "South West"));
    let net = Network::build(&doc, &mut files);
    assert_eq!(net.stops.len(), 4);
    let pos = |ids: &[i64], files: &mut Files| ids.iter().map(|id| stop_world(&doc, files, *id)).collect::<Vec<_>>();
    let north = route::find(&net, &pos(&[10, 11], &mut files), false, None);
    assert!(north.ok(), "{:?}", north.problems);
    let lanes: Vec<(i64, usize)> = north.lanes.iter().map(|l| (net.lanes[*l].key.id, net.lanes[*l].key.path)).collect();
    assert_eq!(lanes, vec![(1, 2), (2, 2)]);
    assert!((north.length() - 80.0).abs() < 1.0, "{}", north.length());
    let south = route::find(&net, &pos(&[12, 13], &mut files), false, None);
    assert!(south.ok(), "{:?}", south.problems);
    let lanes: Vec<(i64, usize)> = south.lanes.iter().map(|l| (net.lanes[*l].key.id, net.lanes[*l].key.path)).collect();
    assert_eq!(lanes, vec![(2, 1), (1, 1)]);
    // a stop off any road
    let mut far = pos(&[10, 11], &mut files);
    far.push(Some(DVec3::new(10.0, 10.0, 0.0)));
    assert_eq!(route::find(&net, &far, false, None).problems, vec![route::Problem::NoRoad(2)]);
    // the track names tile index 0 and OMSI's numbers of the paths: four a spline
    let tr = north.track(&net, &doc.order);
    assert_eq!(tr[0].values, vec![1.0, 2.0, 0.0, 2.0, 50.0, 0.0]);
    assert_eq!(tr[1].values, vec![2.0, 2.0, 0.0, 6.0, 50.0, 0.0]);
    let links = north.links(&net, &doc.order, &[10, 11]);
    assert_eq!(links.len(), 1);
    assert_eq!((links[0].length, links[0].params[2], links[0].params[3]), (80.0, 10.0, 40.0));
}

/// Berlin-Spandau's bus trips routed again: most find their way, about as long as OMSI's
/// own station links say.
#[test]
fn spandau_bus_trips_route() {
    let Some(root) = omsi_root() else { return };
    let cfg = root.join("maps").join("Berlin-Spandau").join("global.cfg");
    if !cfg.is_file() {
        return;
    }
    let _map = one_map();
    let roots = crate::install::Roots::new(root.clone());
    let doc = Document::load(&roots, &cfg, |_| {}).unwrap();
    let mut files = Files::new(vec![root.clone()]);
    let net = Network::build(&doc, &mut files);
    let tt = &doc.timetable;
    let (mut n, mut found, mut close) = (0, 0, 0);
    for t in tt.trips.iter().filter(|t| t.stations.len() >= 2) {
        n += 1;
        let pos: Vec<Option<DVec3>> = t.stations.iter().map(|id| stop_world(&doc, &mut files, *id)).collect();
        let r = route::find(&net, &pos, doc.global.left_hand_traffic, None);
        if !r.ok() {
            eprintln!("{}: {:?}", t.name, r.problems);
            continue;
        }
        found += 1;
        let stock: Option<f64> = t.stations.windows(2).map(|w| tt.links.iter().find(|l| l.from_id == w[0] && l.to_id == w[1]).map(|l| l.length)).sum();
        if let Some(s) = stock {
            if (r.length() - s).abs() < s * 0.1 + 50.0 {
                close += 1;
            } else {
                eprintln!("{}: {:.0} m, OMSI {:.0} m", t.name, r.length(), s);
            }
        }
    }
    eprintln!("Spandau: {found} of {n} bus trips routed, {close} within 10 % of OMSI's length");
    assert!(found * 10 >= n * 7, "{found} of {n}");
}

/// Undo and redo take the timetable back and forth; quick edits of one kind are one step,
/// and the files of what undo took away go on the next save.
#[test]
fn timetable_edits_undo_and_redo() {
    let Some(root) = omsi_root() else { return };
    let _map = one_map();
    let roots = crate::install::Roots::new(root.clone());
    let mut doc = Document::new_map(&roots, "tt-undo", "tt-undo", 1);
    let mut h = crate::history::History::default();
    let snap = |doc: &Document| crate::history::Snapshot { timetable: Some(doc.timetable.clone()), ..Default::default() };
    let before = snap(&doc);
    doc.timetable.add_line("5");
    h.push_snapshot("tt", before);
    // a second edit straight after: the same step
    let before = snap(&doc);
    doc.timetable.add_trip("5", "5_A", vec![1, 2]);
    h.push_snapshot("tt", before);
    assert!(h.can_undo());
    h.undo(&mut doc);
    assert!(doc.timetable.lines.is_empty() && doc.timetable.trips.is_empty());
    assert!(!h.can_undo());
    let out = scratch("undo");
    // what undo took away is not written; a file of it there before is taken away
    std::fs::write(out.join("TTData").join("5.ttl"), b"x").unwrap();
    save_to(&mut doc.timetable, &out, &[]).unwrap();
    assert!(!out.join("TTData").join("5.ttl").exists());
    h.redo(&mut doc);
    assert_eq!(doc.timetable.lines.len(), 1);
    assert_eq!(doc.timetable.trips[0].stations, vec![1, 2]);
    save_to(&mut doc.timetable, &out, &[]).unwrap();
    let back = TimetableData::load(&out);
    assert_eq!((back.lines.len(), back.trips.len()), (1, 1));
    let _ = std::fs::remove_dir_all(&out);
}

/// A tile taken away moves the ones after it up the `[map]` list: the stops' tile indices
/// follow on the next save, also when nothing of the timetable was edited.
#[test]
fn removing_a_tile_renumbers_an_unchanged_timetable() {
    let out = scratch("renumber");
    let mut tt = Timetable::default();
    tt.stops.push(BusStopEntry { name: "East".into(), group: 2, object_id: 5, params: [0.0; 3] });
    tt.touch_stops();
    save_to(&mut tt, &out, &[(0, 0), (1, 0), (2, 0)]).unwrap();
    assert!(!tt.dirty);
    save_to(&mut tt, &out, &[(0, 0), (2, 0)]).unwrap();
    let back = TimetableData::load(&out);
    assert_eq!(back.bus_stops[0].group, 1, "the stop's tile is now the second of the list");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_line_copied_drives_copies_of_its_trips() {
    let mut tt = Timetable::default();
    let l = tt.add_line("76");
    let a = tt.add_trip(&l, "76_A-B", vec![1, 2]);
    let b = tt.add_trip(&l, "76_B-A", vec![2, 1]);
    tt.set_route(&a, vec![Default::default()], Vec::new()).unwrap();
    let line = tt.lines.iter_mut().find(|x| x.name == l).unwrap();
    line.tours.push(omsi_timetable::Tour { number: "1".into(), trips: vec![omsi_timetable::TourTrip { trip: a.clone(), ..Default::default() }, omsi_timetable::TourTrip { trip: b.clone(), ..Default::default() }], ..Default::default() });
    let copy = tt.duplicate_line(&l).unwrap();
    assert_ne!(copy, l);
    let c = tt.lines.iter().find(|x| x.name == copy).unwrap();
    let driven: Vec<&str> = c.tours[0].trips.iter().map(|x| x.trip.as_str()).collect();
    assert_eq!(driven, vec![format!("{copy}_A-B"), format!("{copy}_B-A")]);
    assert_eq!(tt.trip(&format!("{copy}_A-B")).unwrap().stations, vec![1, 2]);
    assert_eq!(tt.trip(&format!("{copy}_A-B")).unwrap().line, copy);
    // (an automatic Type 2 trip keeps no track: the copy's station links are found again)
    assert!(tt.track(&format!("{copy}_A-B")).is_none());
    assert!(tt.reroute.contains(&format!("{copy}_A-B").to_lowercase()));
    // the original as it was
    assert_eq!(tt.lines.iter().find(|x| x.name == l).unwrap().tours[0].trips[0].trip, a);
    assert_eq!(tt.trips.len(), 4);
}

#[test]
fn the_stock_maps_trips_fit_their_roads_as_read() {
    let _map = crate::doc::one_map();
    let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
        eprintln!("no OMSI 2 installation: skipped");
        return;
    };
    let roots = crate::install::Roots { omsi: omsi.clone(), content: std::env::temp_dir().join(format!("openmapper-stale-{}", std::process::id())) };
    for name in ["Grundorf", "Berlin-Spandau"] {
        let cfg = omsi.join("maps").join(name).join("global.cfg");
        if !cfg.is_file() {
            continue;
        }
        let doc = crate::doc::Document::load(&roots, &cfg, |_| {}).unwrap();
        let mut files = Files::new(vec![roots.content.clone(), omsi.clone()]);
        let net = Network::build(&doc, &mut files);
        let on_map: std::collections::HashSet<i64> = doc.tiles.values().flat_map(|t| t.tile.splines.iter().map(|s| s.id).chain(t.tile.objects.iter().map(|o| o.id))).collect();
        let every: std::collections::HashSet<usize> = (0..doc.order.len()).collect();
        let stale = super::route::stale_trips(&doc.timetable, &net, &doc.order, &on_map, &every);
        eprintln!("{name}: {} of {} trips stale: {:?}", stale.len(), doc.timetable.trips.len(), stale.iter().take(5).collect::<Vec<_>>());
        // (Spandau lists tiles it does not ship: trips over them cannot fit)
        if name == "Grundorf" {
            assert!(stale.is_empty(), "{stale:?}");
        }
    }
}


/// A tram's stops stand at its track beside a road: its way is found on the track, a bus's
/// on the road; a track never runs on into a lane; a trip found on the tracks stays there.
#[test]
fn a_tram_trip_runs_on_its_track() {
    let tile = (0, 0);
    let mut net = Network::default();
    let north = |x: f64, y0: f64, y1: f64| vec![DVec3::new(x, y0, 0.0), DVec3::new(x, y1, 0.0)];
    // a road north in two pieces, and 8 m east of it a track north in two pieces; where the
    // first piece of track ends a car lane begins as well
    net.add(PathKey { tile, id: 1, path: 0 }, north(0.0, 0.0, 100.0), 100.0, 0, 0, (true, false));
    net.add(PathKey { tile, id: 2, path: 0 }, north(0.0, 100.0, 200.0), 100.0, 1, 0, (true, false));
    net.add(PathKey { tile, id: 3, path: 0 }, north(8.0, 0.0, 100.0), 100.0, 2, 0, (true, true));
    net.add(PathKey { tile, id: 4, path: 0 }, north(8.0, 100.0, 200.0), 100.0, 3, 0, (true, true));
    net.add(PathKey { tile, id: 5, path: 0 }, north(8.0, 100.0, 120.0), 20.0, 4, 0, (true, false));
    net.link();
    let first_track = net.by_key[&PathKey { tile, id: 3, path: 0 }][0];
    let ids: Vec<i64> = net.lanes[first_track].next.iter().map(|&l| net.lanes[l].key.id).collect();
    assert_eq!(ids, vec![4], "a track runs on into the track only");
    let at = |pts: &[(f64, f64)]| pts.iter().map(|p| Some(DVec3::new(p.0, p.1, 0.0))).collect::<Vec<_>>();
    let on = |r: &route::Route| r.lanes.iter().map(|l| net.lanes[*l].key.id).collect::<Vec<_>>();
    // the tram's stops, 3 m east of the track (and 11 m of the road)
    let tram = at(&[(11.0, 50.0), (11.0, 180.0)]);
    let r = route::find(&net, &tram, false, None);
    assert!(r.ok(), "{:?}", r.problems);
    assert_eq!((r.ways, on(&r)), (Some(route::Ways::Rail), vec![3, 4]));
    // a bus's stops at the kerb
    let bus = at(&[(2.5, 50.0), (2.5, 180.0)]);
    let r = route::find(&net, &bus, false, None);
    assert_eq!((r.ways, on(&r)), (Some(route::Ways::Road), vec![1, 2]));
    // a stop too far from the road for a bus: the tracks still serve it
    let r = route::find(&net, &at(&[(11.0, 50.0), (20.0, 180.0)]), false, None);
    assert_eq!((r.ways, on(&r)), (Some(route::Ways::Rail), vec![3, 4]));
    let r = route::find(&net, &at(&[(11.0, 50.0), (20.0, 180.0)]), false, Some(route::Ways::Road));
    assert_eq!(r.problems, vec![route::Problem::NoRoad(1)]);
    // set to the road, the tram's stops are driven past on it
    let r = route::find(&net, &tram, false, Some(route::Ways::Road));
    assert_eq!((r.ways, on(&r)), (Some(route::Ways::Road), vec![1, 2]));
    // a trip whose way was found on the tracks is found there again
    let r = route::find(&net, &tram, false, None);
    let mut tt = Timetable::default();
    let line = tt.add_line("5");
    let trip = tt.add_trip(&line, "5_1", vec![10, 11]);
    assert_eq!(route::ways_of(&tt, &trip, &net, &[tile]), None);
    tt.set_route(&trip, r.track(&net, &[tile]), r.links(&net, &[tile], &[10, 11])).unwrap();
    assert_eq!(route::ways_of(&tt, &trip, &net, &[tile]), Some(route::Ways::Rail));
}

/// A road object's lane that ends a couple of metres off the road it meets (laid a little
/// under it, as Städtedreieck's are) still runs on into the road, as openOMSI links them
/// (#120); a road on a bridge over it, 6 m up, does not.
#[test]
fn a_lane_a_little_off_the_road_in_height_runs_on_into_it() {
    let tile = (0, 0);
    let mut net = Network::default();
    let north = |y0: f64, y1: f64, z: f64| vec![DVec3::new(0.0, y0, z), DVec3::new(0.0, y1, z)];
    net.add(PathKey { tile, id: 1, path: 0 }, north(0.0, 20.0, -2.5), 20.0, 0, 0, (false, false));
    net.add(PathKey { tile, id: 2, path: 0 }, north(20.0, 120.0, 0.0), 100.0, 1, 0, (true, false));
    net.add(PathKey { tile, id: 3, path: 0 }, north(20.0, 120.0, 6.0), 100.0, 2, 0, (true, false));
    net.link();
    let first = net.by_key[&PathKey { tile, id: 1, path: 0 }][0];
    let ids: Vec<i64> = net.lanes[first].next.iter().map(|&l| net.lanes[l].key.id).collect();
    assert_eq!(ids, vec![2]);
}

/// A lane of a broken spline far out (Solnechnogorsk_2024 has some 10^22 m north) is linked
/// with the rest without the cell numbers overflowing: saving the map stopped there.
#[test]
fn a_lane_at_the_end_of_the_numbers_is_linked_without_overflowing() {
    let tile = (0, 0);
    let mut net = Network::default();
    let north = |y0: f64, y1: f64| vec![DVec3::new(0.0, y0, 0.0), DVec3::new(0.0, y1, 0.0)];
    net.add(PathKey { tile, id: 1, path: 0 }, north(1.15e22, 1.15e22 + 10.0), 10.0, 0, 0, (true, false));
    net.add(PathKey { tile, id: 2, path: 0 }, north(-1.15e22 - 10.0, -1.15e22), 10.0, 1, 0, (true, false));
    net.add(PathKey { tile, id: 3, path: 0 }, north(0.0, 20.0), 20.0, 2, 0, (true, false));
    net.link();
    assert!(net.lanes.iter().all(|l| l.next.is_empty()));
}

/// A road north: `A` to y 100, a 1 m piece `S` (a junction's connector), `B` on from y 101
/// to a junction at y 120 and a detour `D` beside it; on past the junction, and a road
/// leading into `A` from the south.
fn junction_net() -> (Network, std::collections::HashMap<i64, usize>) {
    let tile = (0, 0);
    let mut net = Network::default();
    let p = |pts: &[(f64, f64)]| pts.iter().map(|q| DVec3::new(q.0, q.1, 0.0)).collect::<Vec<_>>();
    // (one way each: direction 0, the way the points run)
    let mut add = |id: i64, pts: Vec<DVec3>, spline: bool| {
        let len: f64 = pts.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        net.add(PathKey { tile, id, path: 0 }, pts, len, id as usize, 0, (spline, false));
    };
    add(1, p(&[(0.0, 0.0), (0.0, 100.0)]), true);
    add(2, p(&[(0.0, 100.0), (0.0, 101.0)]), false);
    add(3, p(&[(0.0, 101.0), (0.0, 120.0)]), true);
    add(4, p(&[(0.0, 101.0), (2.0, 106.0), (2.0, 115.0), (0.0, 120.0)]), false);
    add(5, p(&[(0.0, 120.0), (0.0, 220.0)]), true);
    add(6, p(&[(0.0, -100.0), (0.0, 0.0)]), true);
    net.link();
    let ids = (1..=6).map(|id| (id, net.by_key[&PathKey { tile, id, path: 0 }][0])).collect();
    (net, ids)
}

/// A short piece between two paths is driven, not jumped over to the path after it: the
/// way found joins all along, as the game's bus needs it to (#122).
#[test]
fn a_short_connector_is_driven_not_jumped() {
    let (net, id) = junction_net();
    // (path 3 starts 1 m past path 1's end, within the 1.5 m a link may span)
    assert_eq!(net.lanes[id[&1]].next, vec![id[&2]]);
    let at = |pts: &[(f64, f64)]| pts.iter().map(|p| Some(DVec3::new(p.0, p.1, 0.0))).collect::<Vec<_>>();
    let r = route::find(&net, &at(&[(2.5, 50.0), (2.5, 180.0)]), false, None);
    assert!(r.ok(), "{:?}", r.problems);
    let ids: Vec<i64> = r.lanes.iter().map(|l| net.lanes[*l].key.id).collect();
    assert_eq!(ids, vec![1, 2, 3, 5]);
    assert!(way::gaps(&net, &r.lanes).is_empty());
    // the track read back names the same lanes
    let track = r.track(&net, &[(0, 0)]);
    let back: Vec<Option<usize>> = way::lanes_of_track(&net, &track, &[(0, 0)]);
    assert_eq!(back, r.lanes.iter().map(|l| Some(*l)).collect::<Vec<_>>());
    // a way that leaves the piece out breaks where it is
    let broken = vec![id[&1], id[&3], id[&5]];
    let g = way::gaps(&net, &broken);
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].0, 0);
    assert!((g[0].1 - 1.0).abs() < 1e-6);
}

/// The way picked path by path as in OMSI 2's editor: a path clicked after the end comes
/// with the paths to it, one before the start goes first, another branch at a junction
/// takes the place of the one it goes round, and a path of the way clicked is taken out.
#[test]
fn a_way_picked_by_hand() {
    let (net, id) = junction_net();
    let ids = |lanes: &[usize]| lanes.iter().map(|l| net.lanes[*l].key.id).collect::<Vec<_>>();
    let mut w = way::with_lane(&net, &[], id[&1]);
    assert_eq!(ids(&w), vec![1]);
    // after the end: the connector and the straight way filled in
    w = way::with_lane(&net, &w, id[&5]);
    assert_eq!(ids(&w), vec![1, 2, 3, 5]);
    // before the start (a way before the first stop)
    w = way::with_lane(&net, &w, id[&6]);
    assert_eq!(ids(&w), vec![6, 1, 2, 3, 5]);
    // the detour in place of the straight way
    w = way::with_lane(&net, &w, id[&4]);
    assert_eq!(ids(&w), vec![6, 1, 2, 4, 5]);
    assert!(way::gaps(&net, &w).is_empty());
    // the first taken out again
    w = way::with_lane(&net, &w, id[&6]);
    assert_eq!(ids(&w), vec![1, 2, 4, 5]);
    // the stops found along it, the way running on past the last
    let at = |pts: &[(f64, f64)]| pts.iter().map(|p| Some(DVec3::new(p.0, p.1, 0.0))).collect::<Vec<_>>();
    let r = way::along(&net, &w, &at(&[(2.5, 50.0), (2.5, 150.0)]), false);
    assert!(r.ok() && r.by_hand, "{:?}", r.problems);
    assert_eq!(r.stops.iter().map(|s| s.0).collect::<Vec<_>>(), vec![0, 3]);
    assert!((r.legs[0] - (50.0 + 1.0 + net.lanes[id[&4]].length() + 30.0)).abs() < 1e-6, "{:?}", r.legs);
    // a stop the way does not pass
    let r = way::along(&net, &w, &at(&[(2.5, 50.0), (40.0, 150.0)]), false);
    assert_eq!(r.problems, vec![route::Problem::OffWay(1)]);
}

/// A trip's paths picked by hand are kept as they are when saved and read again: its track
/// says so in a line the game passes over, and it reads back the same.
#[test]
fn a_track_picked_by_hand_reads_back() {
    let mut tt = Timetable::default();
    let out = scratch("byhand");
    let line = tt.add_line("4");
    let trip = tt.add_trip(&line, "4_A", vec![1, 2]);
    let entries = vec![TrackEntry { values: vec![10.0, 0.0, 0.0, 3.0, 50.0, 0.0] }, TrackEntry { values: vec![11.0, 1.0, 0.0, 7.0, 12.5, 0.0] }];
    tt.set_by_hand(&trip, Some((entries.clone(), Vec::new()))).unwrap();
    assert!(tt.by_hand(&trip) && tt.by_hand_track(&trip));
    save_to(&mut tt, &out, &[]).unwrap();
    let text = std::fs::read_to_string(out.join("TTData").join("4_A.ttr")).unwrap();
    assert!(text.contains(BY_HAND));
    let back = TimetableData::load(&out);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.tracks[0].entries, entries);
    let again = Timetable::load(&out, &[]);
    assert!(again.by_hand("4_a") && again.by_hand_track("4_a"));
    // renamed, it stays picked by hand; found automatically again, it is not
    tt.rename_trip(&trip, "4_B").unwrap();
    assert!(tt.by_hand("4_B") && !tt.by_hand("4_A"));
    tt.set_by_hand("4_B", None).unwrap();
    assert!(!tt.by_hand("4_B") && tt.reroute.contains("4_b"));
    let _ = std::fs::remove_dir_all(&out);
}

/// The time from stop to stop set in a profile: the stop before keeps its time, the times
/// after move along and the trip's time with them; a second profile, and the departures
/// that ran to a profile taken away run to the first (#116).
#[test]
fn the_time_from_stop_to_stop() {
    let mut p = omsi_timetable::TripProfile { name: "standard".into(), factor: 12.0, ..Default::default() };
    let legs = [100.0, 100.0, 100.0];
    // 0, 4, 8, 12: the leg into stop 2 from 4 to 6 minutes
    profile::set_leg(&mut p, 4, &legs, 2, 6.0);
    let t: Vec<f32> = stop_times(4, &legs, &p).iter().map(|x| x.0).collect();
    assert_eq!(t, vec![0.0, 4.0, 10.0, 14.0]);
    assert_eq!(p.factor, 14.0);
    // the last leg: the trip's time follows, nothing else moves
    profile::set_leg(&mut p, 4, &legs, 3, 1.5);
    let t: Vec<f32> = stop_times(4, &legs, &p).iter().map(|x| x.0).collect();
    assert_eq!(t, vec![0.0, 4.0, 10.0, 11.5]);
    assert_eq!(p.factor, 11.5);
    // the first leg: the later stops move with it
    profile::set_leg(&mut p, 4, &legs, 1, 2.0);
    let t: Vec<f32> = stop_times(4, &legs, &p).iter().map(|x| x.0).collect();
    assert_eq!(t, vec![0.0, 2.0, 8.0, 9.5]);
    profile::set_total(&mut p, 4, 20.0);
    assert_eq!(stop_times(4, &legs, &p)[3].0, 20.0);
    // a second profile; the departures follow when one is taken away
    let mut tt = Timetable::default();
    let line = tt.add_line("9");
    let trip = tt.add_trip(&line, "9_A", vec![1, 2, 3, 4]);
    tt.trip_mut(&trip).unwrap().profiles[0] = p.clone();
    assert_eq!(tt.add_profile(&trip, 0), Some(1));
    assert_eq!(tt.add_profile(&trip, 0), Some(2));
    let names: Vec<String> = tt.trip(&trip).unwrap().profiles.iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, vec!["standard", "standard 2", "standard 3"]);
    tt.lines[0].tours.push(omsi_timetable::Tour { number: "1".into(), ai_group: String::new(), extra: ALL_DAYS.to_string(), trips: (0..3).map(|k| omsi_timetable::TourTrip { trip: trip.clone(), profile: k, departure: 300.0 + k as f32 * 30.0 }).collect() });
    tt.remove_profile(&trip, 1);
    assert_eq!(tt.lines[0].tours[0].trips.iter().map(|x| x.profile).collect::<Vec<_>>(), vec![0, 0, 1]);
    // the profiles as OMSI 2 reads them
    let back = omsi_timetable::Trip::parse(&omsi_cfg::CfgFile::from_str(format!("{trip}.ttp"), &trip_text(tt.trip(&trip).unwrap())));
    assert_eq!(back.profiles, tt.trip(&trip).unwrap().profiles);
}

/// The ways OMSI 2 serves a stop, each said in its own words.
#[test]
fn every_way_a_stop_is_served_has_a_name() {
    let mut names: Vec<&str> = crate::stops::StopKind::SERVE.iter().map(|v| crate::stops::StopKind::of_serve(*v).unwrap()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 5);
    assert!(crate::stops::StopKind::SERVE.contains(&2));
}

/// The six stops a timetable test converts: ID, name (`Stop 1`…`Stop 6`), and the profile's
/// arrival and departure times at odd and even ones alike.
const SIX_STOPS: [(i64, &str); 6] = [(1000, "Stop 1"), (1001, "Stop 2"), (1002, "Stop 3"), (1003, "Stop 4"), (1004, "Stop 5"), (1005, "Stop 6")];

fn six_stop_profile() -> omsi_timetable::TripProfile {
    omsi_timetable::TripProfile {
        name: "standard".into(),
        factor: 18.0,
        man_dep_time: vec![(1, 2.0), (3, 4.0)],
        man_arr_time: vec![(5, 18.0)],
        other_stopping: vec![(2, 2)],
    }
}

fn six_stop_profile_text() -> String {
    "[profile]\nstandard\n18.000\n\n[profile_man_dep_time]\n1\n2.000\n[profile_man_dep_time]\n3\n4.000\n[profile_man_arr_time]\n5\n18.000\n[profile_otherstopping]\n2\n2\n".into()
}

/// A `[station]` record of 8 lines, as OMSI 2 writes it: object ID, track entry, name,
/// tile, then four numbers (the last its dwell time).
fn canonical_records() -> String {
    let mut o = String::new();
    for (n, (id, name)) in SIX_STOPS.iter().enumerate() {
        o.push_str(&format!("[station]\n{id}\n{}\n{name}\n0\n0.500\n{}.000\n3.000\n{}.000\n\n", n * 10, n * 100, n * 5));
    }
    o
}

/// A shorter `[station]` record (7 lines, no `dwell`) with no blank line between records, as
/// older mod maps have them.
fn short_records() -> String {
    let mut o = String::new();
    for (n, (id, name)) in SIX_STOPS.iter().enumerate() {
        o.push_str(&format!("[station]\n{id}\n{}\n{name}\n0\n0.500\n{}.000\n3.000\n", n * 10, n * 100));
    }
    o
}

/// An even shorter record (6 lines) followed by the blank line OMSI 2 usually puts between
/// blocks: seven lines per record again, and the last one takes the `[profile]` line for its
/// missing eighth field, so the profile goes with it.
fn shorter_records() -> String {
    let mut o = String::new();
    for (n, (id, name)) in SIX_STOPS.iter().enumerate() {
        o.push_str(&format!("[station]\n{id}\n{}\n{name}\n0\n0.500\n{}.000\n\n", n * 10, n * 100));
    }
    o
}

fn six_stop_trip(records: &str) -> String {
    format!("[trip]\nr66\nTerminus\n66\n\n{records}{}", six_stop_profile_text())
}

/// Every stop of a legacy trip comes through the Type 2 conversion: position, name and order
/// of all six, the even ones too, and its arrival and departure times.
fn assert_six_stops(tt: &Timetable, what: &str) {
    let trip = tt.trip("66").unwrap_or_else(|| panic!("{what}: no trip"));
    assert_eq!(trip.stations_legacy.len(), 6, "{what}: legacy records");
    let ids: Vec<i64> = trip.stations_legacy.iter().map(|s| s.first().and_then(|v| v.trim().parse().ok()).unwrap_or_default()).collect();
    let names: Vec<&str> = trip.stations_legacy.iter().map(|s| s.get(2).map(|v| v.trim()).unwrap_or_default()).collect();
    assert_eq!(ids, SIX_STOPS.iter().map(|x| x.0).collect::<Vec<_>>(), "{what}: stop IDs in order");
    assert_eq!(names, SIX_STOPS.iter().map(|x| x.1).collect::<Vec<_>>(), "{what}: stop names in order");
    assert_eq!(trip.profiles, vec![six_stop_profile()], "{what}: arrival and departure times");
}

fn assert_converted_six_stops(tt: &Timetable, what: &str) {
    let trip = tt.trip("66").unwrap_or_else(|| panic!("{what}: no trip"));
    assert!(!is_type1(trip), "{what}: converted to Type 2");
    assert!(trip.stations_legacy.is_empty() && trip.display_name.is_empty(), "{what}: legacy records cleared");
    assert_eq!(trip.stations, SIX_STOPS.iter().map(|x| x.0).collect::<Vec<_>>(), "{what}: every stop in order");
    assert_eq!(trip.profiles, vec![six_stop_profile()], "{what}: arrival and departure times kept");
}

#[test]
fn type2_conversion_keeps_every_stop_of_canonical_legacy_records() {
    let out = scratch("type2-six-canonical");
    std::fs::write(out.join("TTData/66.ttp"), encode(&six_stop_trip(&canonical_records()))).unwrap();
    let mut tt = Timetable::load(&out, &[]);
    assert_six_stops(&tt, "loaded");
    let widths: Vec<usize> = tt.trip("66").unwrap().stations_legacy.iter().map(Vec::len).collect();
    assert_eq!(widths, vec![8; 6], "loaded: fields of each record");
    assert_eq!(tt.convert_all_to_type2(), (1, 0));
    assert_converted_six_stops(&tt, "converted");
    save_to(&mut tt, &out, &[]).unwrap();
    let text = std::fs::read_to_string(out.join("TTData/66.ttp")).unwrap();
    let written: Vec<i64> = text.lines().enumerate().filter(|(_, l)| l.trim() == "[station_typ2]").map(|(i, _)| text.lines().nth(i + 1).unwrap().trim().parse().unwrap()).collect();
    assert_eq!(written, SIX_STOPS.iter().map(|x| x.0).collect::<Vec<_>>(), "written stops");
    let back = Timetable::load(&out, &[]);
    assert_converted_six_stops(&back, "saved and read again");
    let _ = std::fs::remove_dir_all(out);
}

/// Records shorter than the eight lines OMSI 2 writes used to make the trip reader take the
/// next record's `[station]` line for the missing field and skip that record: stops 2, 4 and 6
/// were gone and the conversion wrote only 1, 3 and 5.
#[test]
fn type2_conversion_keeps_every_stop_of_short_legacy_records() {
    for (what, records, width) in [("seven lines, no blank line", short_records(), 7), ("six lines and a blank line", shorter_records(), 6)] {
        let out = scratch(&format!("type2-six-short-{}", what.replace([' ', ','], "-")));
        std::fs::write(out.join("TTData/66.ttp"), encode(&six_stop_trip(&records))).unwrap();
        let mut tt = Timetable::load(&out, &[]);
        assert_six_stops(&tt, what);
        // (the "Profiles" header after the records is not taken for a field of the last one)
        let widths: Vec<usize> = tt.trip("66").unwrap().stations_legacy.iter().map(Vec::len).collect();
        assert_eq!(widths, vec![width; 6], "{what}: fields of each record");
        assert_eq!(tt.convert_all_to_type2(), (1, 0), "{what}: converted");
        assert_converted_six_stops(&tt, what);
        save_to(&mut tt, &out, &[]).unwrap();
        let text = std::fs::read_to_string(out.join("TTData/66.ttp")).unwrap();
        let written: Vec<i64> = text.lines().enumerate().filter(|(_, l)| l.trim() == "[station_typ2]").map(|(i, _)| text.lines().nth(i + 1).unwrap().trim().parse().unwrap()).collect();
        assert_eq!(written, SIX_STOPS.iter().map(|x| x.0).collect::<Vec<_>>(), "{what}: written stops");
        let back = Timetable::load(&out, &[]);
        assert_converted_six_stops(&back, what);
        let _ = std::fs::remove_dir_all(out);
    }
}

/// The station links of the whole map are keyed by their two stop IDs: a single trip's way
/// may only be written when it agrees with the other trips between the same stops.
mod shared_links {
    use super::*;

    fn link(from: i64, to: i64, length: f64) -> omsi_timetable::StnLink {
        omsi_timetable::StnLink { from_id: from, to_id: to, length, ..Default::default() }
    }

    /// A link with an AI path: one entry per lane driven (`lane` becomes the entry's IDCode).
    fn lane_link(from: i64, to: i64, length: f64, lanes: &[i64]) -> omsi_timetable::StnLink {
        let entries = lanes.iter().map(|lane| omsi_timetable::StnLinkEntry {
            values: [*lane as f64, 0.0, 0.0, length, -1.0, 0.0, 0.0],
        }).collect();
        omsi_timetable::StnLink {
            length,
            from_id: from,
            to_id: to,
            params: [0.0, 0.0, 0.0, 0.0, 0.0, lanes.len().saturating_sub(1) as f64],
            entries,
        }
    }

    fn entry(id: f64) -> TrackEntry {
        TrackEntry { values: vec![id, 0.0, 0.0, 0.0, 50.0, 0.0] }
    }

    fn two_trips() -> Timetable {
        let mut tt = Timetable::default();
        tt.add_trip("", "A", vec![100, 200]);
        tt.add_trip("", "B", vec![100, 200]);
        tt
    }

    /// A differing link on a shared pair writes nothing at all: no StnLink replaced, no
    /// `.ttr` made or changed, and the trip is not marked by hand.
    #[test]
    fn a_differing_shared_link_writes_nothing() {
        let mut tt = two_trips();
        let existing = lane_link(100, 200, 120.0, &[10, 11]);
        tt.links.push(existing.clone());
        let track_before = tt.tracks.len();
        let refusal = tt.set_by_hand("A", Some((vec![entry(1.0)], vec![lane_link(100, 200, 140.0, &[10, 11])]))).unwrap_err();
        let conflicts = refusal.conflicts;
        println!("refused: {conflicts:#?}");
        assert_eq!(tt.links, vec![existing], "the map's link is unchanged");
        assert_eq!(tt.tracks.len(), track_before, "no track was made");
        assert!(tt.track("A").is_none());
        assert!(!tt.changed.contains("a.ttr"), "nothing was queued for writing");
        assert!(!tt.by_hand("A"), "a refused write does not mark the trip by hand");
        assert_eq!(conflicts.len(), 1);
        assert_eq!((conflicts[0].from_id, conflicts[0].to_id), (100, 200));
        assert_eq!(conflicts[0].trips, vec!["B".to_string()]);
        assert_eq!(conflicts[0].existing.as_ref().map(|l| l.length), Some(120.0));
        assert_eq!(conflicts[0].new.length, 140.0);
        assert!(conflicts[0].diffs.iter().any(|d| d.field == "length" && d.detail.contains("120.000") && d.detail.contains("140.000")));
    }

    /// The same link as the map's may be written (nothing changes for the other trip).
    #[test]
    fn an_identical_shared_link_is_written() {
        let mut tt = two_trips();
        let same = lane_link(100, 200, 120.0, &[10, 11]);
        tt.links.push(same.clone());
        tt.set_route("A", vec![entry(1.0)], vec![same.clone()]).unwrap();
        assert_eq!(tt.links, vec![same.clone()]);
        assert!(tt.track("A").is_none(), "an automatic route writes no .ttr");
        assert!(!tt.changed.contains("a.ttr"));
    }

    /// A pair only this trip drives between may always be written.
    #[test]
    fn a_pair_only_this_trip_uses_is_written() {
        let mut tt = Timetable::default();
        tt.add_trip("", "A", vec![100, 200]);
        tt.add_trip("", "B", vec![200, 300]);
        tt.links.push(link(200, 300, 50.0));
        let own = lane_link(100, 200, 120.0, &[10]);
        tt.set_route("A", vec![entry(1.0)], vec![own.clone()]).unwrap();
        assert!(tt.links.contains(&own));
        assert!(tt.links.contains(&link(200, 300, 50.0)), "another pair's link is left alone");
    }

    /// Another trip on the pair and no link yet on the map: nothing to compare with, so the
    /// single write is refused rather than assumed safe.
    #[test]
    fn a_missing_link_on_a_shared_pair_is_a_conflict() {
        let mut tt = two_trips();
        let conflicts = tt.set_route("A", vec![entry(1.0)], vec![link(100, 200, 120.0)]).unwrap_err().conflicts;
        assert_eq!(tt.links.len(), 0, "nothing was written");
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].existing.is_none());
        assert!(conflicts[0].diffs.is_empty());
        assert_eq!(conflicts[0].trips, vec!["B".to_string()]);
    }

    /// A Type 1 trip's `[station]` records name the stops of its pairs too.
    #[test]
    fn a_legacy_trips_station_records_use_the_pair() {
        let mut tt = Timetable::default();
        tt.add_trip("", "A", vec![100, 200]);
        tt.trips.push(omsi_timetable::Trip {
            name: "Legacy".into(),
            stations_legacy: vec![vec!["100".into()], vec!["200".into()]],
            ..Default::default()
        });
        tt.links.push(link(100, 200, 120.0));
        let conflicts = tt.set_route("A", vec![entry(1.0)], vec![lane_link(100, 200, 140.0, &[10])]).unwrap_err().conflicts;
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].trips, vec!["Legacy".to_string()]);
        assert_eq!(tt.links.len(), 1);
    }

    /// After the user confirms: the link is replaced, and the gate reports whom it affected.
    #[test]
    fn the_user_can_overwrite_shared_links() {
        let mut tt = two_trips();
        let existing = lane_link(100, 200, 120.0, &[10]);
        tt.links.push(existing.clone());
        let new = lane_link(100, 200, 140.0, &[10, 11]);
        let affected = tt.overwrite_route("A", vec![entry(2.0)], vec![new.clone()], false).unwrap();
        assert_eq!(affected.len(), 1);
        assert_eq!(affected[0].trips, vec!["B".to_string()]);
        assert_eq!(tt.links, vec![new], "the user's link replaced the old one");
        assert!(tt.track("A").is_none(), "the automatic overwrite writes no .ttr");
        assert!(!tt.changed.contains("a.ttr"));
    }

    /// The batch writes several trips together: a pair only they use may be created even
    /// when the map has no link for it yet (`conflicting_batch_routes` compared them).
    #[test]
    fn a_batch_may_create_a_pair_shared_among_its_own_trips() {
        let mut tt = two_trips();
        let peers: std::collections::HashSet<String> = ["a".to_string(), "b".to_string()].into_iter().collect();
        let same = link(100, 200, 120.0);
        tt.set_automatic_route_among("A", vec![entry(1.0)], vec![same.clone()], &peers).unwrap();
        tt.set_automatic_route_among("B", vec![entry(2.0)], vec![same.clone()], &peers).unwrap();
        assert_eq!(tt.links, vec![same]);
        assert!(tt.track("A").is_none() && tt.track("B").is_none(), "automatic batch routes write no .ttr");
    }

    /// A scoped batch grows along every shared station pair until nothing new is reached.
    #[test]
    fn the_scope_follows_shared_station_pairs() {
        let mut tt = Timetable::default();
        tt.add_trip("", "A", vec![1, 2]);
        tt.add_trip("", "B", vec![1, 2, 3]);
        tt.add_trip("", "C", vec![2, 3, 4]);
        tt.add_trip("", "D", vec![9, 10]);
        let scope = links::sharing_trips(&tt, "A");
        let mut names: Vec<&str> = scope.iter().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["a", "b", "c"]);
        assert_eq!(links::sharing_trips(&tt, "D"), ["d".to_string()].into_iter().collect());
    }
}

/// Automatic Type 2 routes write station links only. A stale same-name `.ttr` (the map's own
/// or one an older build wrote) is repaired away, but never one another trip still names.
mod automatic_tracks {
    use super::*;
    use omsi_timetable::{BusStopEntry, StnLink, Track, TrackEntry, Trip};

    fn entry(id: f64) -> TrackEntry {
        TrackEntry { values: vec![id, 0.0, 0.0, 0.0, 50.0, 0.0] }
    }

    fn track(file: &str, ids: &[f64]) -> Track {
        Track { path: format!("{file}.ttr").into(), entries: ids.iter().map(|id| entry(*id)).collect() }
    }

    fn link(from: i64, to: i64, length: f64) -> StnLink {
        StnLink { from_id: from, to_id: to, length, ..Default::default() }
    }

    fn stops() -> Vec<BusStopEntry> {
        [1, 2, 3].into_iter().map(|id| BusStopEntry {
            name: format!("Stop {id}"), group: 0, object_id: id, ..Default::default()
        }).collect()
    }

    /// The source openOMSI's `steps_of` would read: a track named by the trimmed
    /// `display_name` when it exists, else one named like the trip; otherwise the StnLinks.
    /// Every path entry of the chosen source is printed by the caller.
    fn steps_source(tt: &Timetable, trip: &str) -> (&'static str, Vec<String>) {
        let t = tt.trip(trip).unwrap();
        let display = t.display_name.trim();
        let name = if display.is_empty() { t.name.as_str() } else { display };
        if let Some(track) = tt.tracks.iter().find(|x| track_name(x).eq_ignore_ascii_case(name)) {
            return ("track", track.entries.iter().map(|e| format!("{:?}", e.values)).collect());
        }
        let mut out = Vec::new();
        for (a, b) in links::station_pairs(t) {
            for l in tt.links.iter().filter(|l| l.from_id == a && l.to_id == b) {
                for e in &l.entries {
                    out.push(format!("{a}->{b}: {:?}", e.values));
                }
            }
        }
        ("links", out)
    }

    fn show(label: &str, tt: &Timetable, trip: &str) {
        let (source, entries) = steps_source(tt, trip);
        println!(
            "{label}: steps_of reads {source} for {trip} (display_name {:?}), {} entry(ies):\n  {}",
            tt.trip(trip).map(|t| t.display_name.clone()).unwrap_or_default(),
            entries.len(),
            entries.join("\n  ")
        );
    }

    /// A stale automatic `.ttr` beside the map is removed on the automatic write, the links
    /// are written, and a save and reload does not bring the `.ttr` back (nor does a second
    /// save).
    #[test]
    fn automatic_repair_removes_a_stale_same_name_track_and_survives_save_reload() {
        let out = scratch("auto-repair");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(Trip { name: "A".into(), stations: vec![1, 2], ..Default::default() });
        stock.tracks.push(track("A", &[77.0, 78.0]));
        stock.links.push(link(1, 2, 120.0));
        // (a Type 1 trip and its own track: nothing of the repair may touch them)
        stock.trips.push(Trip {
            name: "Legacy".into(), display_name: "Legacy".into(),
            stations_legacy: vec![vec!["1".into()], vec!["2".into()]], ..Default::default()
        });
        stock.tracks.push(track("Legacy", &[79.0]));
        write_all(&stock, &out);
        let legacy_ttp = std::fs::read(out.join("TTData").join("Legacy.ttp")).unwrap();
        let legacy_ttr = std::fs::read(out.join("TTData").join("Legacy.ttr")).unwrap();

        let mut tt = Timetable::load(&out, &[]);
        show("Before repair", &tt, "A");
        assert!(tt.track("A").is_some(), "the stale track is on the map");
        let links = vec![link(1, 2, 120.0)];
        tt.set_route("A", vec![entry(1.0)], links.clone()).unwrap();
        println!("After repair: tracks {:?}, links {:?}, removed {:?}", tt.tracks.iter().map(track_name).collect::<Vec<_>>(), tt.links, tt.removed);
        assert!(tt.track("A").is_none(), "the intercepting track was removed");
        assert!(tt.removed.contains("a.ttr"));
        assert!(tt.track("Legacy").is_some(), "the Type 1 trip's track is untouched");
        show("After repair", &tt, "A");
        assert_eq!(steps_source(&tt, "A").0, "links");

        save_to(&mut tt, &out, &[]).unwrap();
        assert!(!out.join("TTData").join("A.ttr").is_file(), "the stale .ttr file is gone");
        assert!(out.join("TTData").join("Legacy.ttr").is_file(), "the Type 1 track file is kept");
        assert_eq!(std::fs::read(out.join("TTData").join("Legacy.ttr")).unwrap(), legacy_ttr, "the Type 1 track bytes are untouched");
        assert_eq!(std::fs::read(out.join("TTData").join("Legacy.ttp")).unwrap(), legacy_ttp, "the Type 1 trip bytes are untouched");
        let back = Timetable::load(&out, &[]);
        assert!(back.track("A").is_none(), "reload does not recreate the automatic .ttr");
        assert_eq!(back.links, links);
        show("After reload", &back, "A");

        save_to(&mut Timetable::load(&out, &[]), &out, &[]).unwrap();
        assert!(!out.join("TTData").join("A.ttr").is_file(), "a second save writes nothing back");
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A track another trip still names is the other trip's way: the automatic repair is
    /// refused as a whole, and nothing of the trip, its links or its queues changes.
    #[test]
    fn automatic_repair_is_refused_when_another_trip_still_names_the_track() {
        let out = scratch("auto-shared-name");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(Trip { name: "A".into(), stations: vec![1, 2], ..Default::default() });
        stock.trips.push(Trip { name: "B".into(), display_name: "A".into(), stations: vec![1, 2], ..Default::default() });
        stock.tracks.push(track("A", &[77.0]));
        stock.links.push(link(1, 2, 120.0));
        write_all(&stock, &out);
        let mut tt = Timetable::load(&out, &[]);
        show("Before refused repair", &tt, "B");
        let tracks = tt.tracks.clone();
        let trip_before = tt.trip("A").cloned();
        let links_before = tt.links.clone();
        let refusal = tt.set_route("A", vec![entry(1.0)], vec![link(1, 2, 120.0)]).unwrap_err();
        println!("refused: {refusal:?}");
        assert!(refusal.conflicts.is_empty());
        assert!(refusal.track.as_deref().unwrap_or_default().contains("B"), "{refusal:?}");
        assert_eq!(tt.tracks, tracks, "no track is removed or changed");
        assert_eq!(tt.trip("A").cloned(), trip_before, "display_name and stations are unchanged");
        assert_eq!(tt.links, links_before, "no link is written");
        assert!(!tt.changed.contains("a.ttr") && !tt.removed.contains("a.ttr"));
        assert_eq!(steps_source(&tt, "B").0, "track", "the other trip still reads its track");
        // (a save after the refusal leaves the shared track beside the map)
        save_to(&mut tt, &out, &[]).unwrap();
        let back = Timetable::load(&out, &[]);
        assert!(back.track("A").is_some(), "the refused repair is not carried out by a save");
        assert_eq!(steps_source(&back, "B").0, "track");
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A `display_name` that names a track shared with another trip is cleared, the shared
    /// track is left for that trip, and the repaired trip falls back to its station links.
    #[test]
    fn a_shared_display_name_track_is_left_and_the_display_name_is_cleared() {
        let out = scratch("auto-shared-display");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(Trip { name: "A".into(), display_name: "Shared".into(), stations: vec![1, 2], ..Default::default() });
        stock.trips.push(Trip { name: "B".into(), display_name: "Shared".into(), stations: vec![1, 2], ..Default::default() });
        stock.tracks.push(track("Shared", &[55.0]));
        stock.links.push(link(1, 2, 120.0));
        write_all(&stock, &out);
        let mut tt = Timetable::load(&out, &[]);
        show("Before shared repair", &tt, "A");
        tt.set_route("A", vec![entry(1.0)], vec![link(1, 2, 120.0)]).unwrap();
        show("After shared repair", &tt, "A");
        assert!(tt.trip("A").unwrap().display_name.is_empty(), "A's display_name is cleared");
        assert_eq!(tt.trip("B").unwrap().display_name, "Shared", "B still names the track");
        assert!(tt.track("Shared").is_some(), "the shared track is left");
        assert_eq!(steps_source(&tt, "A").0, "links");
        assert_eq!(steps_source(&tt, "B").0, "track");

        save_to(&mut tt, &out, &[]).unwrap();
        let back = Timetable::load(&out, &[]);
        assert!(back.track("Shared").is_some());
        assert!(back.trip("A").unwrap().display_name.is_empty());
        show("After reload", &back, "A");
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A `display_name` track no other trip names is left alone (it is no longer this trip's
    /// way once the name is cleared, but it is not the repair's to delete).
    #[test]
    fn an_unshared_display_name_track_is_left_in_place() {
        let out = scratch("auto-old-display");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(Trip { name: "A".into(), display_name: "Old".into(), stations: vec![1, 2], ..Default::default() });
        stock.tracks.push(track("Old", &[44.0]));
        write_all(&stock, &out);
        let mut tt = Timetable::load(&out, &[]);
        tt.set_route("A", vec![entry(1.0)], vec![link(1, 2, 120.0)]).unwrap();
        assert!(tt.trip("A").unwrap().display_name.is_empty());
        assert!(tt.track("Old").is_some(), "the old track is not deleted");
        assert_eq!(steps_source(&tt, "A").0, "links");
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A hand-picked way writes its `.ttr`; finding the way automatically again is one write
    /// that removes that `.ttr` and writes the links, and a reload keeps it gone.
    #[test]
    fn hand_track_is_written_then_removed_by_the_automatic_route() {
        let out = scratch("hand-then-auto");
        let mut tt = Timetable::default();
        tt.stops = stops();
        tt.add_trip("", "A", vec![1, 2]);
        let hand = vec![entry(10.0), entry(11.0)];
        tt.set_by_hand("A", Some((hand.clone(), vec![link(1, 2, 99.0)]))).unwrap();
        assert!(tt.by_hand("A"));
        assert_eq!(tt.track("A").unwrap().entries, hand);
        save_to(&mut tt, &out, &[]).unwrap();
        let text = std::fs::read_to_string(out.join("TTData").join("A.ttr")).unwrap();
        assert!(text.contains(BY_HAND));
        let back = Timetable::load(&out, &[]);
        assert!(back.by_hand("A"));
        assert_eq!(steps_source(&back, "A").0, "track");
        show("Hand route after reload", &back, "A");

        let mut back = back;
        back.set_automatic_route("A", vec![entry(20.0)], vec![link(1, 2, 120.0)]).unwrap();
        assert!(!back.by_hand("A"));
        assert!(back.track("A").is_none());
        assert!(back.removed.contains("a.ttr"));
        assert_eq!(steps_source(&back, "A").0, "links");
        save_to(&mut back, &out, &[]).unwrap();
        assert!(!out.join("TTData").join("A.ttr").is_file());
        let again = Timetable::load(&out, &[]);
        assert!(!again.by_hand("A") && again.track("A").is_none());
        assert_eq!(again.links, vec![link(1, 2, 120.0)]);
        show("Automatic route after reload", &again, "A");
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A hand route becoming automatic is refused as a whole when its old track is another
    /// trip's: the hand mark, the track, the links and the queues stay as they were.
    #[test]
    fn hand_to_automatic_keeps_the_old_track_and_mode_when_it_is_shared() {
        let mut tt = Timetable::default();
        tt.stops = stops();
        tt.add_trip("", "A", vec![1, 2]);
        let hand = vec![entry(10.0)];
        tt.set_by_hand("A", Some((hand.clone(), vec![link(1, 2, 99.0)]))).unwrap();
        tt.add_trip("", "B", vec![2, 3]);
        tt.trip_mut("B").unwrap().display_name = "A".into();
        let tracks = tt.tracks.clone();
        let links = tt.links.clone();
        let reroute = tt.reroute.clone();
        let refusal = tt.set_automatic_route("A", vec![entry(20.0)], vec![link(1, 2, 120.0)]).unwrap_err();
        println!("refused: {refusal:?}");
        assert!(refusal.track.is_some());
        assert!(tt.by_hand("A"), "the trip stays picked by hand");
        assert_eq!(tt.tracks, tracks, "the old track is unchanged");
        assert_eq!(tt.links, links, "no link is written");
        assert_eq!(tt.reroute, reroute, "the queue is unchanged");
        assert_eq!(steps_source(&tt, "A").0, "track");
    }

    /// A nonempty `display_name` that names no track still makes the game look only for that
    /// name: a hand route must clear it, or `<trip>.ttr` is never read.
    #[test]
    fn a_hand_route_clears_a_display_name_that_names_no_track() {
        let mut tt = Timetable::default();
        tt.stops = stops();
        tt.add_trip("", "A", vec![1, 2]);
        tt.trip_mut("A").unwrap().display_name = "A human name".into();
        tt.set_by_hand("A", Some((vec![entry(10.0)], vec![link(1, 2, 99.0)]))).unwrap();
        assert!(tt.trip("A").unwrap().display_name.is_empty(), "the game reads A.ttr now");
        assert_eq!(steps_source(&tt, "A").0, "track");
    }

    /// An automatic Type 2 trip keeps no same-name `.ttr`, even while a `display_name` that
    /// names no track makes the game read the station links already.
    #[test]
    fn an_automatic_route_removes_a_stale_fallback_track_even_with_a_display_name() {
        let out = scratch("auto-human-display");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(Trip { name: "A".into(), display_name: "A human name".into(), stations: vec![1, 2], ..Default::default() });
        stock.tracks.push(track("A", &[66.0]));
        write_all(&stock, &out);
        let mut tt = Timetable::load(&out, &[]);
        tt.set_route("A", vec![entry(1.0)], vec![link(1, 2, 120.0)]).unwrap();
        assert_eq!(tt.trip("A").unwrap().display_name, "A human name", "the display name is kept");
        assert!(tt.track("A").is_none(), "an automatic trip keeps no same-name .ttr");
        assert!(tt.removed.contains("a.ttr"));
        assert_eq!(steps_source(&tt, "A").0, "links");
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A same-name track another trip still names is left for it while the repaired trip's
    /// `display_name` stands (it does not intercept then), and the repair still succeeds.
    #[test]
    fn a_shared_fallback_track_is_left_when_the_display_name_stands() {
        let out = scratch("auto-human-shared");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(Trip { name: "A".into(), display_name: "A human name".into(), stations: vec![1, 2], ..Default::default() });
        stock.trips.push(Trip { name: "B".into(), display_name: "A".into(), stations: vec![1, 2], ..Default::default() });
        stock.tracks.push(track("A", &[66.0]));
        stock.links.push(link(1, 2, 120.0));
        write_all(&stock, &out);
        let mut tt = Timetable::load(&out, &[]);
        tt.set_route("A", vec![entry(1.0)], vec![link(1, 2, 120.0)]).unwrap();
        assert_eq!(tt.trip("A").unwrap().display_name, "A human name");
        assert!(tt.track("A").is_some(), "the track is the other trip's way");
        assert!(tt.removed.is_empty(), "nothing is queued for removal");
        assert_eq!(steps_source(&tt, "A").0, "links");
        assert_eq!(steps_source(&tt, "B").0, "track");
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Even a confirmed shared-link overwrite does not write over another trip's track.
    #[test]
    fn a_hand_overwrite_is_refused_when_another_trip_names_the_track() {
        let mut tt = Timetable::default();
        tt.stops = stops();
        tt.add_trip("", "A", vec![1, 2]);
        tt.add_trip("", "B", vec![1, 2]);
        tt.trip_mut("B").unwrap().display_name = "A".into();
        let refusal = tt.overwrite_route("A", vec![entry(10.0)], vec![link(1, 2, 120.0)], true).unwrap_err();
        println!("refused: {refusal:?}");
        assert!(refusal.track.is_some());
        assert!(tt.track("A").is_none() && !tt.by_hand("A"));
        assert!(tt.links.is_empty());
    }

    /// A batch writes several automatic routes, each repairing its own stale track; a save
    /// and reload has no automatic `.ttr` and drives all of them by the shared StnLink.
    #[test]
    fn a_batch_automatic_write_removes_stale_tracks_and_reload_has_none() {
        let out = scratch("batch-auto-repair");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(Trip { name: "A".into(), stations: vec![1, 2], ..Default::default() });
        stock.trips.push(Trip { name: "B".into(), stations: vec![1, 2], ..Default::default() });
        stock.tracks.push(track("A", &[71.0]));
        stock.tracks.push(track("B", &[72.0]));
        stock.links.push(link(1, 2, 120.0));
        write_all(&stock, &out);
        let mut tt = Timetable::load(&out, &[]);
        let peers: std::collections::HashSet<String> = ["a".to_string(), "b".to_string()].into_iter().collect();
        let shared = link(1, 2, 120.0);
        tt.set_automatic_route_among("A", vec![entry(1.0)], vec![shared.clone()], &peers).unwrap();
        tt.set_automatic_route_among("B", vec![entry(2.0)], vec![shared.clone()], &peers).unwrap();
        assert!(tt.track("A").is_none() && tt.track("B").is_none(), "both stale tracks are repaired away");
        assert!(tt.removed.contains("a.ttr") && tt.removed.contains("b.ttr"));
        save_to(&mut tt, &out, &[]).unwrap();
        assert!(!out.join("TTData").join("A.ttr").is_file() && !out.join("TTData").join("B.ttr").is_file());
        let back = Timetable::load(&out, &[]);
        assert!(back.tracks.is_empty(), "the reload has no automatic .ttr");
        assert_eq!(back.links, vec![shared]);
        show("Batch after reload", &back, "A");
        show("Batch after reload", &back, "B");
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A group write's rollback snapshot must carry the hand markers as well as the tracks
    /// and links: an actual automatic switch drops the marker, and restoring only the `.ttr`
    /// would leave a hand-picked track that the window then treats as automatic.
    #[test]
    fn route_state_restores_the_hand_marker_with_the_hand_track() {
        let mut tt = Timetable::default();
        tt.stops = stops();
        tt.add_trip("", "A", vec![1, 2]);
        let hand = vec![entry(10.0), entry(11.0)];
        let hand_link = link(1, 2, 99.0);
        tt.set_by_hand("A", Some((hand.clone(), vec![hand_link.clone()]))).unwrap();
        assert!(tt.by_hand("A"), "the setup is a hand-picked way");
        let state = tt.route_state();
        let tracks = tt.tracks.clone();
        let links = tt.links.clone();
        let changed = tt.changed.clone();
        let removed = tt.removed.clone();
        // (the actual hand -> automatic switch takes the hand mark and the .ttr away)
        tt.set_automatic_route("A", vec![entry(20.0)], vec![link(1, 2, 120.0)]).unwrap();
        assert!(!tt.by_hand("A"), "the automatic switch dropped the hand marker");
        assert!(tt.track("A").is_none(), "the automatic switch dropped the .ttr");
        // (a later route in the same group fails: the snapshot puts the hand marker back with
        // the track, links and queues; without it the restored .ttr reads as automatic)
        tt.restore_route_state(state);
        assert!(tt.by_hand("A"), "the restored .ttr is hand-picked again");
        assert_eq!(tt.tracks, tracks);
        assert_eq!(tt.links, links);
        assert_eq!(tt.changed, changed);
        assert_eq!(tt.removed, removed);
        assert_eq!(tt.track("A").unwrap().entries, hand);
        assert_eq!(tt.links, vec![hand_link]);
    }
}

/// A single Type 1 trip becomes a Type 2 trip by format alone: its stop records and track
/// reference change, no route is found and no station link is written.
mod single_type2_conversion {
    use super::*;
    use super::convert;
    use crate::doc::Document;
    use crate::history::{History, Snapshot};
    use omsi_timetable::{BusStopEntry, Track, TrackEntry, Trip};

    fn entry(id: f64) -> TrackEntry {
        TrackEntry { values: vec![id, 0.0, 0.0, 0.0, 50.0, 0.0] }
    }

    fn track(file: &str, ids: &[f64]) -> Track {
        Track { path: format!("{file}.ttr").into(), entries: ids.iter().map(|id| entry(*id)).collect() }
    }

    fn legacy(ids: &[&str]) -> Vec<Vec<String>> {
        ids.iter().enumerate().map(|(i, id)| {
            vec![id.to_string(), (i * 10).to_string(), format!("Stop {id}"), "0".into(), "0.500".into(), format!("{}.000", i * 100), "3.000".into(), "0.000".into()]
        }).collect()
    }

    fn type1(name: &str, display: &str, ids: &[&str]) -> Trip {
        Trip {
            name: name.into(),
            display_name: display.into(),
            terminus: "Terminus".into(),
            line: "1".into(),
            stations_legacy: legacy(ids),
            ..Default::default()
        }
    }

    fn type2(name: &str, display: &str, stations: &[i64]) -> Trip {
        Trip {
            name: name.into(),
            display_name: display.into(),
            terminus: "Terminus".into(),
            line: "1".into(),
            stations: stations.to_vec(),
            ..Default::default()
        }
    }

    fn stops() -> Vec<BusStopEntry> {
        [1, 2, 3, 4].into_iter().map(|id| BusStopEntry {
            name: format!("Stop {id}"), group: 0, object_id: id, ..Default::default()
        }).collect()
    }

    /// The source openOMSI's `steps_of` reads: a track named by the trimmed `display_name`
    /// when that is nonempty, else a track named like the trip; otherwise the StnLinks.
    fn steps_source(tt: &Timetable, trip: &str) -> (&'static str, Vec<String>) {
        let t = tt.trip(trip).unwrap();
        let display = t.display_name.trim();
        let name = if display.is_empty() { t.name.as_str() } else { display };
        if let Some(track) = tt.tracks.iter().find(|x| track_name(x).eq_ignore_ascii_case(name)) {
            return ("track", track.entries.iter().map(|e| format!("{:?}", e.values)).collect());
        }
        let mut out = Vec::new();
        for (a, b) in links::station_pairs(t) {
            for l in tt.links.iter().filter(|l| l.from_id == a && l.to_id == b) {
                for e in &l.entries {
                    out.push(format!("{a}->{b}: {:?}", e.values));
                }
            }
        }
        ("links", out)
    }

    fn show(label: &str, tt: &Timetable, trip: &str) {
        let (source, entries) = steps_source(tt, trip);
        println!(
            "{label}: steps_of reads {source} for {trip} (display_name {:?}), {} entry(ies):\n  {}",
            tt.trip(trip).map(|t| t.display_name.clone()).unwrap_or_default(),
            entries.len(),
            entries.join("\n  ")
        );
    }

    fn read(dir: &std::path::Path, file: &str) -> Vec<u8> {
        std::fs::read(dir.join("TTData").join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))
    }

    /// Only the chosen trip changes; the other Type 1 trip, its track and its exclusion stay
    /// byte-for-byte as they were, and the chosen trip's unused old track is deleted on save.
    #[test]
    fn single_conversion_changes_only_the_chosen_trip_and_removes_its_unused_track() {
        let out = scratch("single-type2-only");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(type1("chosen", "chosen-way", &["1", "2", "3"]));
        stock.trips.push(type1("other", "other-way", &["1", "4"]));
        stock.tracks.push(track("chosen-way", &[71.0, 72.0]));
        stock.tracks.push(track("other-way", &[73.0]));
        stock.links.push(omsi_timetable::StnLink { from_id: 1, to_id: 2, length: 100.0, ..Default::default() });
        write_all(&stock, &out);

        let mut tt = Timetable::load(&out, &[]);
        let chosen = tt.trip("chosen").unwrap().clone();
        let other = tt.trip("other").unwrap().clone();
        tt.set_type2_exclusion(&chosen, true);
        tt.set_type2_exclusion(&other, true);
        save_to(&mut tt, &out, &[]).unwrap();
        let other_ttp = read(&out, "other.ttp");
        let other_ttr = read(&out, "other-way.ttr");
        let links_before = read(&out, "StnLinks.cfg");
        let links_data = tt.links.clone();
        let chosen_ttr_path = out.join("TTData").join("chosen-way.ttr");
        assert!(chosen_ttr_path.is_file());
        let exclusions_before = std::fs::read_to_string(convert::exclusions_path(&out)).unwrap();
        println!("Exclusions before single conversion:\n{exclusions_before}");

        let converted = tt.convert_trip_to_type2("chosen").unwrap();
        println!("Single conversion result: {converted:?}");
        assert_eq!(converted.trip, "chosen");
        assert_eq!(converted.stops, 3);
        assert_eq!(converted.removed_track.as_deref(), Some("chosen-way"));
        assert_eq!(converted.kept_shared_track, None);
        let chosen = tt.trip("chosen").unwrap();
        assert!(!is_type1(chosen), "the chosen trip is Type 2");
        assert_eq!(chosen.stations, [1, 2, 3]);
        assert!(chosen.stations_legacy.is_empty() && chosen.display_name.is_empty());
        assert!(!tt.excludes_type2(chosen), "the explicit conversion cleared its exclusion");
        assert!(!tt.type2_exclusions.contains_key("chosen"), "the chosen trip's override is removed");
        assert_eq!(tt.trip("other").unwrap(), &other, "the other trip is unchanged");
        assert!(tt.excludes_type2(tt.trip("other").unwrap()), "the other trip's exclusion is unchanged");
        assert!(tt.track("chosen-way").is_none(), "the old track left the timetable");
        assert!(tt.track("other-way").is_some(), "the other trip's track stays");
        assert!(tt.removed.contains("chosen-way.ttr") && !tt.removed.contains("other-way.ttr"));
        assert!(tt.changed.contains("chosen.ttp") && !tt.changed.contains("other.ttp"));
        assert_eq!(tt.links, links_data, "no station link is changed");

        save_to(&mut tt, &out, &[]).unwrap();
        let chosen_text = std::fs::read_to_string(out.join("TTData").join("chosen.ttp")).unwrap().replace("\r\n", "\n");
        println!("chosen.ttp after the single conversion:\n{chosen_text}");
        assert_eq!(chosen_text.matches("[station_typ2]").count(), 3);
        assert!(!chosen_text.contains("[station]\n"), "no legacy station record is left");
        assert!(!chosen_ttr_path.is_file(), "the unused old track is deleted on save");
        assert_eq!(read(&out, "other.ttp"), other_ttp, "the other trip's bytes are unchanged");
        assert_eq!(read(&out, "other-way.ttr"), other_ttr, "the other trip's track bytes are unchanged");
        assert_eq!(read(&out, "StnLinks.cfg"), links_before, "StnLinks.cfg is not written");
        let exclusions_after = std::fs::read_to_string(convert::exclusions_path(&out)).unwrap();
        println!("Exclusions after single conversion:\n{exclusions_after}");
        let json: serde_json::Value = serde_json::from_str(&exclusions_after).unwrap();
        assert_eq!(json, serde_json::json!({ "excluded": ["other"], "overrides": { "other": true } }));

        let back = Timetable::load(&out, &[]);
        show("After reload", &back, "chosen");
        assert!(!is_type1(back.trip("chosen").unwrap()));
        assert_eq!(back.trip("chosen").unwrap().stations, [1, 2, 3]);
        assert!(back.track("chosen-way").is_none());
        assert!(is_type1(back.trip("other").unwrap()));
        assert!(back.excludes_type2(back.trip("other").unwrap()));
        let _ = std::fs::remove_dir_all(out);
    }

    /// A one-stop Type 1 trip is excluded by default from the batch, but the explicit
    /// single conversion still formats it and clears any override it had.
    #[test]
    fn an_explicit_conversion_bypasses_the_default_short_trip_exclusion() {
        let out = scratch("single-type2-short");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(type1("short", "short-way", &["1"]));
        stock.tracks.push(track("short-way", &[50.0]));
        write_all(&stock, &out);
        let mut tt = Timetable::load(&out, &[]);
        assert!(tt.excludes_type2(tt.trip("short").unwrap()), "the batch would skip it by default");
        let converted = tt.convert_trip_to_type2("short").unwrap();
        assert_eq!(converted.stops, 1);
        assert!(!is_type1(tt.trip("short").unwrap()));
        assert_eq!(tt.trip("short").unwrap().stations, [1]);
        save_to(&mut tt, &out, &[]).unwrap();
        let back = Timetable::load(&out, &[]);
        assert!(!is_type1(back.trip("short").unwrap()));
        assert!(!back.excludes_type2(back.trip("short").unwrap()));
        let _ = std::fs::remove_dir_all(out);
    }

    /// The old track another trip still names is that trip's way: its bytes stay, and a
    /// pending change to it is written by the next save rather than lost.
    #[test]
    fn a_shared_old_track_keeps_its_bytes_and_pending_changes() {
        let out = scratch("single-type2-shared-bytes");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(type1("bus", "shared-way", &["1", "2"]));
        stock.trips.push(type2("holder", "shared-way", &[1, 2]));
        stock.tracks.push(track("shared-way", &[90.0]));
        write_all(&stock, &out);

        let mut tt = Timetable::load(&out, &[]);
        let original = read(&out, "shared-way.ttr");
        let converted = tt.convert_trip_to_type2("bus").unwrap();
        println!("Shared track conversion result: {converted:?}");
        assert_eq!(converted.kept_shared_track.as_deref(), Some("shared-way"));
        assert_eq!(converted.removed_track, None);
        assert!(tt.track("shared-way").is_some(), "the track is left for the other trip");
        assert!(!tt.removed.contains("shared-way.ttr"));
        assert!(tt.trip("bus").unwrap().display_name.is_empty(), "the trip's own reference is gone");
        assert_eq!(steps_source(&tt, "bus").0, "links");
        assert_eq!(steps_source(&tt, "holder").0, "track");

        save_to(&mut tt, &out, &[]).unwrap();
        assert_eq!(read(&out, "shared-way.ttr"), original, "the shared track's bytes are untouched");
        let back = Timetable::load(&out, &[]);
        assert_eq!(back.track("shared-way").unwrap().entries, vec![entry(90.0)]);
        assert!(!is_type1(back.trip("bus").unwrap()));
        assert_eq!(steps_source(&back, "bus").0, "links");

        // (a pending change to the shared track belongs to the other trip; converting bus
        // must not drop it from the changed queue or write the old entries over it. This
        // starts from a fresh copy of the original Type 1 fixture: `out` now holds the
        // already converted bus.)
        let pending = scratch("single-type2-shared-bytes-pending");
        write_all(&stock, &pending);
        let mut tt = Timetable::load(&pending, &[]);
        assert!(is_type1(tt.trip("bus").unwrap()), "the pending-write fixture starts Type 1");
        tt.tracks.iter_mut().find(|t| track_name(t) == "shared-way").unwrap().entries = vec![entry(91.0)];
        tt.changed.insert("shared-way.ttr".into());
        let converted = tt.convert_trip_to_type2("bus").unwrap();
        assert_eq!(converted.kept_shared_track.as_deref(), Some("shared-way"));
        assert!(tt.changed.contains("shared-way.ttr"), "the pending write is kept");
        assert!(!tt.removed.contains("shared-way.ttr"));
        save_to(&mut tt, &pending, &[]).unwrap();
        let text = std::fs::read_to_string(pending.join("TTData").join("shared-way.ttr")).unwrap().replace("\r\n", "\n");
        println!("Shared track with the pending change after save:\n{text}");
        assert_eq!(text, track_text(&track("shared-way", &[91.0])));
        let back = Timetable::load(&pending, &[]);
        assert_eq!(back.track("shared-way").unwrap().entries, vec![entry(91.0)]);
        let _ = std::fs::remove_dir_all(pending);
        let _ = std::fs::remove_dir_all(out);
    }

    /// A shared source whose filename equals the selected trip's name must not be read after
    /// conversion: the trip gets a persistent nonmatching first line instead of falling back
    /// to that name.
    #[test]
    fn a_shared_source_named_like_the_trip_cannot_drive_the_converted_trip() {
        let out = scratch("single-type2-shared-name");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(type1("A", "A", &["1", "2"]));
        stock.trips.push(type2("holder", "A", &[1, 2]));
        stock.tracks.push(track("A", &[77.0]));
        write_all(&stock, &out);
        // (the shared track says its paths were picked by hand: the marker belongs to the
        // track, not to the converted trip reading it by name)
        std::fs::write(out.join("TTData").join("A.ttr"), encode(&track_text_of(&track("A", &[77.0]), true))).unwrap();
        let original = read(&out, "A.ttr");

        let mut tt = Timetable::load(&out, &[]);
        assert!(tt.by_hand("A"), "the shared track is marked by hand");
        let holder_was_hand = tt.by_hand("holder");
        assert_eq!(steps_source(&tt, "holder").0, "track");
        let converted = tt.convert_trip_to_type2("A").unwrap();
        println!("Shared same-name conversion result: {converted:?}");
        assert_eq!(converted.kept_shared_track.as_deref(), Some("A"));
        assert!(!converted.display_name.is_empty(), "the fallback to A would read the shared track");
        assert!(!converted.display_name.eq_ignore_ascii_case("A"));
        assert!(tt.tracks.iter().all(|t| !track_name(t).eq_ignore_ascii_case(&converted.display_name)));
        assert_eq!(steps_source(&tt, "A").0, "links");
        assert_eq!(steps_source(&tt, "holder").0, "track");
        assert!(tt.hand.contains("a"), "the retained shared track keeps its hand marker");
        assert!(!tt.by_hand("A"), "the converted trip no longer reads the shared hand track");
        assert_eq!(tt.by_hand("holder"), holder_was_hand, "the other trip's mode is unchanged");

        save_to(&mut tt, &out, &[]).unwrap();
        assert_eq!(read(&out, "A.ttr"), original, "the shared track is not touched");
        let back = Timetable::load(&out, &[]);
        show("Shared same-name after reload", &back, "A");
        assert!(!is_type1(back.trip("A").unwrap()));
        assert!(back.hand.contains("a"), "the reloaded shared track keeps its marker");
        assert!(!back.by_hand("A"), "the reloaded trip does not inherit the shared track's hand marker");
        assert_eq!(steps_source(&back, "A").0, "links");
        assert_eq!(steps_source(&back, "holder").0, "track");
        assert_eq!(back.by_hand("holder"), holder_was_hand, "the other trip's mode is unchanged after reload");
        assert_eq!(back.trip("A").unwrap().display_name, converted.display_name, "the nonmatching first line is persisted");
        let _ = std::fs::remove_dir_all(out);
    }

    /// A track-only Type 1 trip with no `[station]` records has no Type 2 form while it must
    /// keep a nonmatching first line: it is refused rather than left looking converted.
    #[test]
    fn a_zero_stop_trip_with_a_shared_same_name_track_is_refused_without_changes() {
        let mut tt = Timetable::default();
        tt.trips.push(Trip { name: "A".into(), display_name: "A".into(), ..Default::default() });
        tt.trips.push(type2("holder", "A", &[1]));
        tt.tracks.push(track("A", &[1.0]));
        let before = tt.clone();
        let error = tt.convert_trip_to_type2("A").unwrap_err();
        println!("Refused zero-stop conversion: {error}");
        assert!(error.contains("no [station] records"), "{error}");
        assert!(tt.same_content(&before), "nothing changes");
        assert!(tt.removed.is_empty() && tt.changed.is_empty());
        assert!(is_type1(tt.trip("A").unwrap()));
    }

    /// A source stem with two tracks that differ only by ASCII case is ambiguous: the save
    /// removes by a folded name, so conversion refuses before touching anything.
    #[test]
    fn duplicate_case_source_tracks_are_refused_before_changes() {
        let mut tt = Timetable::default();
        tt.trips.push(type1("dup", "dup-way", &["1", "2"]));
        tt.tracks.push(track("dup-way", &[40.0]));
        tt.tracks.push(track("DUP-WAY", &[41.0]));
        let before = tt.clone();
        let error = tt.convert_trip_to_type2("dup").unwrap_err();
        println!("Refused duplicate source: {error}");
        assert!(error.contains("more than one") && error.contains("folded name"), "{error}");
        assert!(tt.same_content(&before), "nothing changes");
        assert!(tt.removed.is_empty() && tt.changed.is_empty());
        assert!(is_type1(tt.trip("dup").unwrap()));
    }

    /// The engine compares stems with ASCII case folding while the save planner folds the
    /// whole file name: non-ASCII case variants could be removed instead of the source, so
    /// they are refused too.
    #[test]
    fn non_ascii_case_variants_of_the_source_track_are_refused() {
        let mut tt = Timetable::default();
        tt.trips.push(type1("nonascii", "Ä", &["1", "2"]));
        tt.tracks.push(track("Ä", &[42.0]));
        tt.tracks.push(track("ä", &[43.0]));
        let before = tt.clone();
        let error = tt.convert_trip_to_type2("nonascii").unwrap_err();
        println!("Refused non-ASCII fold: {error}");
        assert!(error.contains("more than one") && error.contains("folded name"), "{error}");
        assert!(tt.same_content(&before), "nothing changes");
        assert!(tt.removed.is_empty() && tt.changed.is_empty());
        assert!(is_type1(tt.trip("nonascii").unwrap()));
    }

    /// A hand marker applies only while the game actually reads `<trip>.ttr`; a display
    /// naming another track or no track leaves the trip on that other source.
    #[test]
    fn by_hand_requires_the_game_to_read_the_trips_own_track() {
        let mut tt = Timetable::default();
        tt.trips.push(Trip { name: "A".into(), display_name: "other".into(), stations: vec![1, 2], ..Default::default() });
        tt.tracks.push(track("A", &[1.0]));
        tt.tracks.push(track("other", &[2.0]));
        tt.hand.insert("a".into());
        assert_eq!(steps_source(&tt, "A").0, "track", "the display reads the other track");
        assert!(!tt.by_hand("A"), "the game does not read A.ttr, so its hand marker does not apply");
        assert!(tt.by_hand_track("a"), "the track's own marker is still there for the file writer");
        tt.trip_mut("A").unwrap().display_name = "missing".into();
        assert_eq!(steps_source(&tt, "A").0, "links", "a missing display track reads the links");
        assert!(!tt.by_hand("A"));
        assert!(tt.by_hand_track("a"));
        tt.trip_mut("A").unwrap().display_name.clear();
        assert_eq!(steps_source(&tt, "A").0, "track", "the fallback reads A.ttr");
        assert!(tt.by_hand("A"), "a normal hand-picked trip is still by hand");
    }

    /// An unrelated same-name `.ttr` is not the selected trip's old track: it is neither
    /// deleted nor allowed to intercept the converted trip.
    #[test]
    fn an_unrelated_same_name_track_is_kept_and_does_not_intercept() {
        let out = scratch("single-type2-unrelated-name");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(type1("B", "b-way", &["1", "2"]));
        stock.tracks.push(track("b-way", &[80.0]));
        stock.tracks.push(track("B", &[81.0]));
        write_all(&stock, &out);
        // (the unrelated track is hand-marked: its marker must stay with it)
        std::fs::write(out.join("TTData").join("B.ttr"), encode(&track_text_of(&track("B", &[81.0]), true))).unwrap();
        let unrelated = read(&out, "B.ttr");

        let mut tt = Timetable::load(&out, &[]);
        assert!(tt.hand.contains("b"), "the unrelated track's marker is loaded");
        let converted = tt.convert_trip_to_type2("B").unwrap();
        println!("Unrelated same-name conversion result: {converted:?}");
        assert_eq!(converted.removed_track.as_deref(), Some("b-way"));
        assert!(converted.kept_shared_track.is_none());
        assert!(!converted.display_name.is_empty(), "the fallback to B would read the unrelated track");
        assert!(tt.track("B").is_some(), "the unrelated track is not deleted");
        assert!(tt.hand.contains("b"), "the unrelated track keeps its marker");
        assert!(!tt.removed.contains("b.ttr"));
        assert_eq!(steps_source(&tt, "B").0, "links");

        save_to(&mut tt, &out, &[]).unwrap();
        assert!(!out.join("TTData").join("b-way.ttr").is_file(), "the trip's old track is deleted");
        assert_eq!(read(&out, "B.ttr"), unrelated, "the unrelated track's bytes are untouched");
        let back = Timetable::load(&out, &[]);
        show("Unrelated same-name after reload", &back, "B");
        assert!(!is_type1(back.trip("B").unwrap()));
        assert!(back.hand.contains("b"), "the unrelated track's marker survives the reload");
        assert_eq!(steps_source(&back, "B").0, "links");
        assert!(back.track("B").is_some());
        let _ = std::fs::remove_dir_all(out);
    }

    /// No StnLinks file is needed or created: the format conversion only writes the trip.
    #[test]
    fn single_conversion_needs_no_station_links_and_reloads_as_type2() {
        let out = scratch("single-type2-no-links");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(type1("only", "only-way", &["1", "2"]));
        stock.tracks.push(track("only-way", &[70.0]));
        write_all(&stock, &out);
        std::fs::remove_file(out.join("TTData").join("StnLinks.cfg")).unwrap();

        let mut tt = Timetable::load(&out, &[]);
        assert!(tt.links.is_empty());
        tt.convert_trip_to_type2("only").unwrap();
        save_to(&mut tt, &out, &[]).unwrap();
        assert!(!out.join("TTData").join("StnLinks.cfg").is_file(), "no station links are written");
        assert!(!out.join("TTData").join("only-way.ttr").is_file(), "the old track is deleted");
        let back = Timetable::load(&out, &[]);
        show("No links after reload", &back, "only");
        assert!(!is_type1(back.trip("only").unwrap()));
        assert_eq!(back.trip("only").unwrap().stations, [1, 2]);
        assert_eq!(steps_source(&back, "only"), ("links", Vec::new()));
        let _ = std::fs::remove_dir_all(out);
    }

    /// A technical omission refuses before anything changes, and a save after the refusal
    /// writes none of the files.
    #[test]
    fn an_invalid_stop_id_leaves_the_trip_tracks_and_exclusions_untouched() {
        let out = scratch("single-type2-invalid");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(type1("bad", "bad-way", &["1", "not-an-id", "3"]));
        stock.tracks.push(track("bad-way", &[60.0]));
        write_all(&stock, &out);
        let files = ["bad.ttp", "bad-way.ttr", "Busstops.cfg", "StnLinks.cfg"];

        let mut tt = Timetable::load(&out, &[]);
        let bad = tt.trip("bad").unwrap().clone();
        tt.set_type2_exclusion(&bad, true);
        save_to(&mut tt, &out, &[]).unwrap();
        let before = tt.clone();
        let before_files: Vec<(&str, Vec<u8>)> = files.iter().map(|f| (*f, read(&out, *f))).collect();
        let exclusions_before = std::fs::read(convert::exclusions_path(&out)).unwrap();

        let error = tt.convert_trip_to_type2("bad").unwrap_err();
        println!("Refused single conversion: {error}");
        assert!(error.contains("stop 2") && error.contains("not-an-id"), "{error}");
        assert!(tt.same_content(&before), "trip, tracks and exclusions are unchanged");
        assert_eq!(tt.changed, before.changed);
        assert_eq!(tt.removed, before.removed);
        assert!(tt.excludes_type2(tt.trip("bad").unwrap()), "the exclusion stays");

        save_to(&mut tt, &out, &[]).unwrap();
        for (file, bytes) in &before_files {
            assert_eq!(&read(&out, *file), bytes, "{file} is unchanged");
        }
        assert_eq!(std::fs::read(convert::exclusions_path(&out)).unwrap(), exclusions_before);
        let _ = std::fs::remove_dir_all(out);
    }

    /// Undo puts the Type 1 trip, its old track and its exclusion back on disk; redo removes
    /// them again. The restored change/removal queues are what make the files follow.
    #[test]
    fn single_conversion_undoes_and_redoes_through_saved_files() {
        let _map = one_map();
        let out = scratch("single-type2-undo");
        let mut stock = Timetable::default();
        stock.stops = stops();
        stock.trips.push(type1("chosen", "chosen-way", &["1", "2"]));
        stock.tracks.push(track("chosen-way", &[55.0]));
        write_all(&stock, &out);

        let roots = crate::install::Roots::new(out.clone());
        let mut doc = Document::new_map(&roots, "single-type2-undo", "single-type2-undo", 1);
        doc.dir = out.clone();
        doc.source = None;
        doc.fresh = false;
        doc.timetable = Timetable::load(&out, &doc.order);
        let chosen = doc.timetable.trip("chosen").unwrap().clone();
        doc.timetable.set_type2_exclusion(&chosen, true);
        save_to(&mut doc.timetable, &out, &doc.order).unwrap();
        let track_bytes = read(&out, "chosen-way.ttr");
        let before = doc.timetable.clone();
        let mut history = History::default();

        let converted = doc.timetable.convert_trip_to_type2("chosen").unwrap();
        println!("Undo test conversion: {converted:?}");
        history.push_snapshot("Convert this trip to Type 2", Snapshot { timetable: Some(before), ..Default::default() });
        assert!(doc.timetable.removed.contains("chosen-way.ttr"));
        save_to(&mut doc.timetable, &out, &doc.order).unwrap();
        assert!(!out.join("TTData").join("chosen-way.ttr").is_file());
        assert_eq!(Timetable::load(&out, &[]).trip("chosen").unwrap().stations, [1, 2]);

        history.undo(&mut doc);
        let restored = &doc.timetable;
        assert!(is_type1(restored.trip("chosen").unwrap()), "undo brings the Type 1 records back");
        assert!(restored.excludes_type2(restored.trip("chosen").unwrap()), "undo brings the exclusion back");
        assert!(restored.changed.contains("chosen.ttp") && restored.changed.contains("chosen-way.ttr"));
        assert!(!restored.removed.contains("chosen-way.ttr"));
        save_to(&mut doc.timetable, &out, &doc.order).unwrap();
        assert_eq!(read(&out, "chosen-way.ttr"), track_bytes, "undo writes the old track bytes back");
        let back = Timetable::load(&out, &[]);
        assert!(is_type1(back.trip("chosen").unwrap()));
        assert!(back.excludes_type2(back.trip("chosen").unwrap()));

        history.redo(&mut doc);
        let redone = &doc.timetable;
        assert!(!is_type1(redone.trip("chosen").unwrap()), "redo makes it Type 2 again");
        assert!(!redone.excludes_type2(redone.trip("chosen").unwrap()));
        assert!(redone.removed.contains("chosen-way.ttr") && !redone.changed.contains("chosen-way.ttr"));
        save_to(&mut doc.timetable, &out, &doc.order).unwrap();
        assert!(!out.join("TTData").join("chosen-way.ttr").is_file(), "redo removes the old track again");
        let back = Timetable::load(&out, &[]);
        show("After redo reload", &back, "chosen");
        assert!(!is_type1(back.trip("chosen").unwrap()));
        assert_eq!(steps_source(&back, "chosen").0, "links");
        let _ = std::fs::remove_dir_all(out);
    }
}
