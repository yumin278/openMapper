use super::*;
use serde_json::{json, Value};

use crate::assets::Assets;
use crate::doc::tests::scratch;
use crate::install::Roots;
use glam::DVec2;
use omsi_map::{MapObject, TERRAIN_SAMPLES};

fn temp_map(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("openmapper-crossing-signals-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn path_of(map: &std::path::Path) -> std::path::PathBuf {
    map.join("openMapper").join(FILE_NAME)
}

fn read(map: &std::path::Path) -> Option<Value> {
    std::fs::read(path_of(map)).ok().map(|b| serde_json::from_slice(&b).unwrap())
}

#[test]
fn key_normalizes_slashes_and_case() {
    assert_eq!(key("SceneryObjects/X/Y.SCO"), "sceneryobjects\\x\\y.sco");
    assert_eq!(key(r"SceneryObjects\X\Y.sco"), "sceneryobjects\\x\\y.sco");
}

#[test]
fn load_skips_invalid_entries_and_normalizes_keys() {
    let map = temp_map("load");
    let raw = json!({
        "version": 1,
        "crossings": {
            "SceneryObjects/A/B.sco": [
                {"light": 0, "set": "", "x": 1.0, "up": 2.0, "fwd": 3.0, "heading": 4.0},
                {"light": 0, "set": "x", "x": 1.0, "up": "bad", "fwd": 3.0, "heading": 4.0},
                {"light": -1, "set": "", "x": 1.0, "up": 2.0, "fwd": 3.0, "heading": 4.0}
            ]
        }
    });
    std::fs::create_dir_all(map.join("openMapper")).unwrap();
    std::fs::write(path_of(&map), raw.to_string()).unwrap();

    let mut signals = CrossingSignals::default();
    signals.reload(&map);
    // (the two bad entries are dropped; the key is normalised)
    let points = signals.points(r"SceneryObjects\a\b.sco").unwrap();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].x, 1.0);
    assert_eq!(signals.points("SceneryObjects/A/B.sco").unwrap().len(), 1);
    assert!(signals.points("other").is_none());
    let _ = std::fs::remove_dir_all(&map);
}

#[test]
fn write_points_replaces_only_one_key_and_keeps_the_rest() {
    let map = temp_map("write");
    std::fs::create_dir_all(map.join("openMapper")).unwrap();
    // (B has an entry this editor does not understand: it must survive a save of A untouched)
    let before = json!({
        "version": 1,
        "crossings": {
            "a.sco": [{"light": 0, "set": "", "x": 0.0, "up": 0.0, "fwd": 0.0, "heading": 0.0}],
            "b.sco": [{"light": 0, "set": "", "x": 9.0, "up": 9.0, "fwd": 9.0, "heading": 9.0, "unknown": "kept"}]
        }
    });
    std::fs::write(path_of(&map), before.to_string()).unwrap();

    let new_a = vec![SignalPoint { light: 1, set: "s".into(), x: 1.0, up: 2.0, fwd: 3.0, heading: 4.0 }];
    write_points(&map, "A.SCO", &new_a).unwrap();

    let after = read(&map).unwrap();
    let crossings = after["crossings"].as_object().unwrap();
    // (A was rewritten under its normalised key)
    let a = crossings["a.sco"].as_array().unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["light"], 1);
    assert_eq!(a[0]["set"], "s");
    // (B is byte-for-byte the value it had, unknown field included)
    assert_eq!(crossings["b.sco"], before["crossings"]["b.sco"]);
    let _ = std::fs::remove_dir_all(&map);
}

#[test]
fn empty_crossing_entries_survive_reload_and_disappearing_files_clear_the_cache() {
    let map = temp_map("empty-reload");
    write_points(&map, "SceneryObjects/Empty.SCO", &[]).unwrap();
    let mut signals = CrossingSignals::default();
    signals.reload(&map);
    assert_eq!(signals.points(r"sceneryobjects\empty.sco"), Some(&[][..]));
    std::fs::remove_file(path_of(&map)).unwrap();
    signals.reload(&map);
    assert!(signals.points(r"sceneryobjects\empty.sco").is_none());
    let _ = std::fs::remove_dir_all(&map);
}

#[test]
fn malformed_crossing_value_does_not_discard_other_crossings() {
    let map = temp_map("invalid-crossing");
    std::fs::create_dir_all(map.join("openMapper")).unwrap();
    let raw = json!({"version": 1, "crossings": {
        "bad.sco": "not an array",
        "good.sco": [{"light":0,"set":"","x":1,"up":2,"fwd":3,"heading":4}]
    }});
    std::fs::write(path_of(&map), raw.to_string()).unwrap();
    let mut signals = CrossingSignals::default();
    signals.reload(&map);
    assert_eq!(signals.points("good.sco").unwrap().len(), 1);
    assert!(signals.points("bad.sco").is_none());
    write_points(&map, "good.sco", &[]).unwrap();
    assert_eq!(read(&map).unwrap()["crossings"]["bad.sco"], raw["crossings"]["bad.sco"]);
    let _ = std::fs::remove_dir_all(&map);
}

#[test]
fn saving_one_crossing_removes_only_its_normalized_aliases() {
    let map = temp_map("aliases");
    std::fs::create_dir_all(map.join("openMapper")).unwrap();
    let point = json!({"light":0,"set":"","x":1,"up":2,"fwd":3,"heading":4});
    let raw = json!({"version":1,"unknown":{"preserve":true},"crossings":{
        "Sceneryobjects/A.sco":[point.clone()],
        "sceneryobjects\\a.sco":[point],
        "Other/B.SCO":{"unrecognised":"keep"}
    }});
    std::fs::write(path_of(&map), raw.to_string()).unwrap();
    write_points(&map, "SCENERYOBJECTS/A.SCO", &[SignalPoint::default()]).unwrap();
    let saved = read(&map).unwrap();
    let crossings = saved["crossings"].as_object().unwrap();
    assert_eq!(crossings.len(), 2);
    assert_eq!(crossings[r"sceneryobjects\a.sco"].as_array().unwrap().len(), 1);
    assert_eq!(crossings["Other/B.SCO"], raw["crossings"]["Other/B.SCO"]);
    assert_eq!(saved["unknown"], raw["unknown"]);
    let _ = std::fs::remove_dir_all(&map);
}

#[test]
fn write_points_leaves_a_broken_or_unsupported_file_alone() {
    for bad in [
        "{not json".to_string(),
        json!({"version": 2, "crossings": {}}).to_string(),
        json!({"version": "1", "crossings": {}}).to_string(),
        json!({"crossings": {}}).to_string(),
        json!({"version": 1, "crossings": []}).to_string(),
    ] {
        let map = temp_map("broken");
        std::fs::create_dir_all(map.join("openMapper")).unwrap();
        std::fs::write(path_of(&map), &bad).unwrap();
        let points = vec![SignalPoint::default()];
        assert!(write_points(&map, "a.sco", &points).is_err());
        assert_eq!(std::fs::read_to_string(path_of(&map)).unwrap(), bad);
        let _ = std::fs::remove_dir_all(&map);
    }
}

/// A content folder with a signal set (head with `[absheight]`, pole without) and two crossing
/// `.sco`s - one with `[absheight]`, one without - each with a single-light program.
fn placement_fixtures(name: &str) -> (Roots, std::path::PathBuf) {
    let roots = scratch(&format!("crossing-signals-{name}"));
    let fix = roots.content.join("Sceneryobjects").join("Fixtures");
    std::fs::create_dir_all(&fix).unwrap();
    std::fs::write(fix.join("Head.sco"), "[absheight]\n").unwrap();
    std::fs::write(fix.join("Pole.sco"), "[friendlyname]\nPole\n").unwrap();
    let cross = roots.content.join("Sceneryobjects").join("Crossing");
    std::fs::create_dir_all(&cross).unwrap();
    std::fs::write(cross.join("crossing_abs.sco"), "[absheight]\n[traffic_light]\nMain\n[phase]\n0\n30\n").unwrap();
    std::fs::write(cross.join("crossing_ground.sco"), "[traffic_light]\nMain\n[phase]\n0\n30\n").unwrap();
    let map = roots.content.join("maps").join("map");
    std::fs::create_dir_all(map.join("openMapper")).unwrap();
    let sets = json!({"version":1, "default":"fixture", "sets":[{"id":"fixture", "name":"Fixture", "head":"Sceneryobjects/Fixtures/Head.sco", "pole":"Sceneryobjects/Fixtures/Pole.sco", "head_height":2.55, "pole_offset":[0.0, -0.47, -2.55]}]});
    std::fs::write(map.join("openMapper").join(crate::signal_sets::FILE_NAME), sets.to_string()).unwrap();
    (roots, map)
}

/// Place one hand-authored signal and give the world z of the head that was made.
fn placed_head_z(roots: &Roots, map: &std::path::Path, assets: &Assets, rel: &str, abs: bool, head_abs: bool, pole_abs: bool, rotation: f64, pos_z: f64, up: f32, steep: bool) -> f64 {
    let mut ed = crate::editor::Editor::default();
    ed.content_dir = Some(roots.content.clone());
    // (the positions are read when the map opens)
    let mut signals = serde_json::Map::new();
    signals.insert("version".into(), json!(1));
    let mut crossings = serde_json::Map::new();
    crossings.insert(key(rel), json!([{"light":0, "set":"", "x":3.0, "up": up, "fwd":-2.0, "heading":40.0}]));
    signals.insert("crossings".into(), json!(crossings));
    std::fs::write(map.join("openMapper").join(FILE_NAME), serde_json::to_string(&signals).unwrap()).unwrap();
    let mut doc = crate::selection::tests::flat_map(1);
    doc.dir = map.to_path_buf();
    ed.open(doc);
    let k = (0, 0);
    let id = 42i64;
    ed.doc.as_mut().unwrap().tiles.get_mut(&k).unwrap().tile.objects.push(MapObject {
        file: rel.to_string(),
        id,
        pos: [10.0, 20.0, pos_z],
        rot: [rotation, 0.0, 0.0],
        ..Default::default()
    });
    if steep {
        // (a slope so the ground at the crossing's origin is not the flat 0)
        let t = ed.doc.as_mut().unwrap().tiles.get_mut(&k).unwrap();
        for iy in 0..TERRAIN_SAMPLES {
            for ix in 0..TERRAIN_SAMPLES {
                t.terrain.heights[iy * TERRAIN_SAMPLES + ix] = ix as f32 * 8.0 + iy as f32 * 4.0;
            }
        }
    }
    ed.place_crossing_signals(assets, k, id).unwrap();
    let doc = ed.doc.as_ref().unwrap();
    let org = crate::doc::tile_origin(k);
    let origin_xy = DVec2::new(org.x + 10.0, org.y + 20.0);
    let junction_z = if abs { pos_z } else { doc.ground(origin_xy) + pos_z };
    // (the head is the child that carries the light number in `extra`)
    let children = crate::junction::children(doc, id);
    assert_eq!(children.len(), 2);
    let head = children.iter().find_map(|&(hk, hid)| {
        let o = doc.object(hk, hid).unwrap();
        (o.extra.len() == 1).then(|| {
            let actual = doc.object_world(hk, o, head_abs);
            let expected_xy = origin_xy + crate::selection::turn_point(DVec2::new(3.0, -2.0), DVec2::ZERO, rotation);
            assert!((actual.truncate() - expected_xy).length() < 0.001);
            assert!((o.rot[0] - (rotation + 40.0)).abs() < 0.001);
            actual.z
        })
    }).expect("a head was placed");
    let pole_z = children.iter().find_map(|&(pk, pid)| {
        let o = doc.object(pk, pid).unwrap();
        o.extra.is_empty().then(|| doc.object_world(pk, o, pole_abs).z)
    }).unwrap();
    assert!((pole_z - (head - 2.55)).abs() < 0.01);
    assert!(
        (head - (junction_z + up as f64 + 2.55)).abs() < 0.01,
        "head world z {head} != junction z {junction_z} + up {up} + head height 2.55 (abs={abs}, rotation={rotation})"
    );
    head
}

#[test]
fn place_crossing_signals_sets_the_head_height_for_any_rotation_and_height_mode() {
    let (roots, map) = placement_fixtures("height");
    let assets = Assets::new(roots.omsi.clone(), roots.content.clone());
    for head_abs in [false, true] {
        for pole_abs in [false, true] {
            let fix = roots.content.join("Sceneryobjects").join("Fixtures");
            std::fs::write(fix.join("Head.sco"), if head_abs { "[absheight]\n" } else { "[friendlyname]\nHead\n" }).unwrap();
            std::fs::write(fix.join("Pole.sco"), if pole_abs { "[absheight]\n" } else { "[friendlyname]\nPole\n" }).unwrap();
            for (rel, abs) in [("Sceneryobjects/Crossing/crossing_abs.sco", true), ("Sceneryobjects/Crossing/crossing_ground.sco", false)] {
                for rotation in [0.0, 17.3, 123.7, -46.2] {
                    placed_head_z(&roots, &map, &assets, rel, abs, head_abs, pole_abs, rotation, 4.0, 1.23, true);
                    placed_head_z(&roots, &map, &assets, rel, abs, head_abs, pole_abs, rotation, 1.5, -0.75, false);
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn place_crossing_signals_does_not_duplicate_on_replay() {
    let (roots, map) = placement_fixtures("replay");
    let assets = Assets::new(roots.omsi.clone(), roots.content.clone());
    let rel = "Sceneryobjects/Crossing/crossing_abs.sco";
    let mut ed = crate::editor::Editor::default();
    ed.content_dir = Some(roots.content.clone());
    let mut signals = serde_json::Map::new();
    signals.insert("version".into(), json!(1));
    let mut crossings = serde_json::Map::new();
    crossings.insert(key(rel), json!([{"light":0, "set":"", "x":3.0, "up":1.0, "fwd":-2.0, "heading":0.0}]));
    signals.insert("crossings".into(), json!(crossings));
    std::fs::write(map.join("openMapper").join(FILE_NAME), serde_json::to_string(&signals).unwrap()).unwrap();
    let mut doc = crate::selection::tests::flat_map(1);
    doc.dir = map.clone();
    ed.open(doc);
    let (k, id) = ((0, 0), 42i64);
    ed.doc.as_mut().unwrap().tiles.get_mut(&k).unwrap().tile.objects.push(MapObject {
        file: rel.to_string(),
        id,
        pos: [10.0, 20.0, 0.0],
        rot: [0.0, 0.0, 0.0],
        ..Default::default()
    });
    ed.place_crossing_signals(&assets, k, id).unwrap();
    let first_objects: Vec<_> = crate::junction::children(ed.doc.as_ref().unwrap(), id)
        .into_iter().map(|(k, id)| (k, ed.doc.as_ref().unwrap().object(k, id).unwrap().clone())).collect();
    let first = first_objects.len();
    ed.place_crossing_signals(&assets, k, id).unwrap();
    let second = crate::junction::children(ed.doc.as_ref().unwrap(), id).len();
    assert_eq!(first, second);
    assert_eq!(first, 2);
    let doc = ed.doc.as_mut().unwrap();
    ed.history.undo(doc);
    for (k, object) in &first_objects {
        assert_eq!(doc.object(*k, object.id), Some(object));
    }
    ed.history.redo(doc);
    assert_eq!(crate::junction::children(doc, id).len(), 2);
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn refreshing_a_saved_crossing_updates_its_light_program_without_a_full_scan() {
    let (roots, _) = placement_fixtures("catalogue");
    let rel = r"Sceneryobjects\Crossing\crossing_abs.sco";
    let mut lib = crate::library::Library::default();
    lib.refresh_crossing(&roots, rel);
    assert!(lib.find(rel).unwrap().light_program);
    std::fs::write(roots.content.join("Sceneryobjects/Crossing/crossing_abs.sco"), "[absheight]\n").unwrap();
    lib.refresh_crossing(&roots, rel);
    assert!(!lib.find(rel).unwrap().light_program);
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn clearing_configured_positions_removes_all_direct_children_and_undo_restores_them() {
    let (roots, map) = placement_fixtures("clear");
    let assets = Assets::new(roots.omsi.clone(), roots.content.clone());
    let rel = r"Sceneryobjects\Crossing\crossing_abs.sco";
    let mut doc = crate::selection::tests::flat_map(2);
    doc.dir = map.clone();
    let (k, id) = ((0, 0), 42);
    doc.tiles.get_mut(&k).unwrap().tile.objects.push(MapObject {
        file: rel.into(), id, pos: [10.0, 20.0, 4.0], ..Default::default()
    });
    for (child, parent) in [(51, id), (52, 777)] {
        doc.tiles.get_mut(&(1, 0)).unwrap().tile.objects.push(MapObject {
            id: child, var_parent: Some(parent), ..Default::default()
        });
    }
    doc.tiles.get_mut(&(1, 0)).unwrap().tile.attach_objects.push(MapObject {
        id: 53, var_parent: Some(id), parent_id: Some(52), ..Default::default()
    });
    doc.tiles.get_mut(&(1, 0)).unwrap().tile.spline_attachments.push(omsi_map::SplineAttachment {
        id: 54, var_parent: Some(id), ..Default::default()
    });
    let before = doc.tiles[&(1, 0)].tile.clone();
    write_points(&map, rel, &[]).unwrap();
    let mut ed = crate::editor::Editor::default();
    ed.open(doc);
    ed.place_crossing_signals(&assets, k, id).unwrap();
    let doc = ed.doc.as_mut().unwrap();
    assert_eq!(doc.tiles[&(1, 0)].tile.objects.len(), 1);
    assert_eq!(doc.tiles[&(1, 0)].tile.objects[0].id, 52);
    assert!(doc.tiles[&(1, 0)].tile.attach_objects.is_empty());
    assert!(doc.tiles[&(1, 0)].tile.spline_attachments.is_empty());
    assert!(doc.object(k, id).is_some());
    ed.history.undo(doc);
    assert_eq!(doc.tiles[&(1, 0)].tile, before);
    ed.history.redo(doc);
    assert_eq!(doc.tiles[&(1, 0)].tile.objects.len(), 1);
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn placement_report_counts_missing_tiles_and_invalid_light_indices() {
    let (roots, map) = placement_fixtures("report");
    let assets = Assets::new(roots.omsi.clone(), roots.content.clone());
    let rel = r"Sceneryobjects\Crossing\crossing_abs.sco";
    write_points(&map, rel, &[
        SignalPoint { x: (crate::doc::tile_size() - 10.2) as f32, fwd: 30.0, heading: 270.0, ..Default::default() },
        SignalPoint { light: 99, ..Default::default() },
    ]).unwrap();
    let mut doc = crate::selection::tests::flat_map(1);
    doc.dir = map.clone();
    let (k, id) = ((0, 0), 42);
    doc.tiles.get_mut(&k).unwrap().tile.objects.push(MapObject {
        file: rel.into(), id, pos: [10.0, 20.0, 4.0], ..Default::default()
    });
    let mut ed = crate::editor::Editor::default();
    ed.content_dir = Some(roots.content.clone());
    ed.open(doc);
    let report = ed.place_crossing_signals(&assets, k, id).unwrap();
    assert_eq!((report.requested, report.placed, report.skipped), (2, 0, 2));
    assert!(crate::junction::children(ed.doc.as_ref().unwrap(), id).is_empty());
    ed.doc.as_mut().unwrap().add_tile((1, 0));
    let before_next_id = ed.doc.as_ref().unwrap().global.next_id_code;
    let report = ed.place_crossing_signals(&assets, k, id).unwrap();
    assert_eq!((report.requested, report.placed, report.skipped), (2, 1, 1));
    let doc = ed.doc.as_mut().unwrap();
    assert_eq!(crate::junction::children(doc, id).len(), 2);
    ed.history.undo(doc);
    // History deliberately does not recycle issued IDs (history::apply): redo and
    // generated filenames may still refer to them.
    assert_eq!(doc.global.next_id_code, before_next_id + 2);
    assert!(crate::junction::children(doc, id).is_empty());
    let _ = std::fs::remove_dir_all(&roots.content);
}
