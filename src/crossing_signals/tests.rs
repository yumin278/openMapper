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
fn write_points_leaves_a_broken_or_unsupported_file_alone() {
    for bad in ["{not json".to_string(), json!({"version": 2, "crossings": {}}).to_string()] {
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
fn placement_fixtures() -> (Roots, std::path::PathBuf) {
    let roots = scratch("crossing-signals-place");
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
fn placed_head_z(roots: &Roots, map: &std::path::Path, assets: &Assets, rel: &str, abs: bool, rotation: f64, pos_z: f64, up: f32, steep: bool) -> f64 {
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
                t.terrain.heights[iy * TERRAIN_SAMPLES + ix] = ix as f32 * 0.1 + iy as f32 * 0.05;
            }
        }
    }
    ed.place_crossing_signals(assets, k, id).unwrap();
    let doc = ed.doc.as_ref().unwrap();
    let org = crate::doc::tile_origin(k);
    let origin_xy = DVec2::new(org.x + 10.0, org.y + 20.0);
    let junction_z = if abs { pos_z } else { doc.ground(origin_xy) + pos_z };
    // (the head is the child that carries the light number in `extra`)
    let head = crate::junction::children(doc, id).into_iter().find_map(|(hk, hid)| {
        let o = doc.object(hk, hid).unwrap();
        (o.extra.len() == 1).then(|| doc.object_world(hk, o, true).z)
    }).expect("a head was placed");
    assert!(
        (head - (junction_z + up as f64 + 2.55)).abs() < 0.01,
        "head world z {head} != junction z {junction_z} + up {up} + head height 2.55 (abs={abs}, rotation={rotation})"
    );
    head
}

#[test]
fn place_crossing_signals_sets_the_head_height_for_any_rotation_and_height_mode() {
    let (roots, map) = placement_fixtures();
    let assets = Assets::new(roots.omsi.clone(), roots.content.clone());
    for (rel, abs) in [("Sceneryobjects/Crossing/crossing_abs.sco", true), ("Sceneryobjects/Crossing/crossing_ground.sco", false)] {
        for rotation in [0.0, 17.3, 123.7, -46.2] {
            // (steep ground for the ground-relative crossing)
            placed_head_z(&roots, &map, &assets, rel, abs, rotation, 4.0, 1.23, true);
            // (and the flat case too)
            placed_head_z(&roots, &map, &assets, rel, abs, rotation, 1.5, 0.0, false);
        }
    }
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn place_crossing_signals_does_not_duplicate_on_replay() {
    let (roots, map) = placement_fixtures();
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
    let first = crate::junction::children(ed.doc.as_ref().unwrap(), id).len();
    ed.place_crossing_signals(&assets, k, id).unwrap();
    let second = crate::junction::children(ed.doc.as_ref().unwrap(), id).len();
    assert_eq!(first, second);
    assert!(first > 0);
    let _ = std::fs::remove_dir_all(&roots.content);
}
