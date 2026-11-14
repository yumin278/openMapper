use super::*;
use crate::doc::tests::scratch;
use crate::install::Roots;
use serde_json::{Value, json};

fn config_path(map: &Path) -> std::path::PathBuf {
    map.join("openMapper").join(FILE_NAME)
}

fn config(default: &str, sets: &[Value]) -> Value {
    json!({"version": 1, "default": default, "sets": sets})
}

fn fixture(id: &str) -> Value {
    json!({
        "id": id, "name": "Fixture lamp",
        "head": "Sceneryobjects/Fixtures/Head.sco",
        "pole": r"Sceneryobjects\Fixtures\Pole.sco",
        "head_height": 2.55, "pole_offset": [0.0, -0.47, -2.55]
    })
}

fn write_config(map: &Path, config: &Value) {
    let path = config_path(map);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, config.to_string()).unwrap();
}

fn fixtures(roots: &Roots) {
    let dir = roots.content.join("Sceneryobjects").join("Fixtures");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Head.sco"), "[absheight]\n").unwrap();
    std::fs::write(dir.join("Pole.sco"), "[friendlyname]\nPole\n").unwrap();
}

fn report<'a>(sets: &'a SignalSets, request: &str) -> &'a str {
    let (set, why) = sets.resolve(request);
    println!("requested={request:?} -> used={set:?}, reason={why}");
    &set.id
}

#[test]
fn junction_signal_sets_fall_back_for_missing_invalid_or_unsupported_configs() {
    let roots = scratch("junction-signal-fallback");
    let map = roots.content.join("maps").join("map");
    let path = config_path(&map);
    let mut sets = SignalSets::default();
    for bytes in [
        None,
        Some("{invalid JSON"),
        Some(r#"{"version":2,"default":"other","sets":[]}"#),
    ] {
        println!("Input at {}: {bytes:?}", path.display());
        if let Some(bytes) = bytes {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
        }
        sets.reload(&map, &roots.content);
        assert_eq!(report(&sets, ""), "mc_small");
        assert_eq!(report(&sets, "unknown"), "mc_small");
        assert_eq!(std::fs::read_to_string(&path).ok().as_deref(), bytes);
    }
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn junction_signal_sets_normalize_paths_and_skip_bad_entries() {
    let roots = scratch("junction-signal-valid");
    fixtures(&roots);
    let map = roots.content.join("maps").join("map");
    let mut missing = fixture("missing");
    missing["pole"] = json!("Sceneryobjects/Fixtures/Missing.sco");
    let input = config(
        "small-1_a",
        &[
            fixture("bad id"),
            json!(42),
            missing.clone(),
            fixture("small-1_a"),
        ],
    );
    write_config(&map, &input);
    println!("Input at {}:\n{input:#}", config_path(&map).display());
    let mut sets = SignalSets::default();
    sets.reload(&map, &roots.content);
    let (set, _) = sets.resolve("");
    assert_eq!(sets.sets.len(), 1);
    assert_eq!(report(&sets, ""), "small-1_a");
    assert_eq!(set.head, r"Sceneryobjects\Fixtures\Head.sco");
    assert_eq!(set.pole, r"Sceneryobjects\Fixtures\Pole.sco");
    assert!(set.mirror_x_lht);
    assert_eq!(report(&sets, "bad id"), "small-1_a");
    assert_eq!(report(&sets, "missing"), "small-1_a");
    for (input, explicit, expected) in [
        (config("small-1_a", &[missing]), "", "mc_small"),
        (config("gone", &[fixture("small-1_a")]), "", "mc_small"),
        (config("gone", &[fixture("small-1_a")]), "small-1_a", "small-1_a"),
    ] {
        println!("Reload input: {input}");
        write_config(&map, &input);
        sets.reload(&map, &roots.content);
        assert_eq!(report(&sets, explicit), expected);
    }
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn junction_signal_sets_old_sco_uses_the_open_maps_default() {
    let roots = scratch("junction-signal-map-switch");
    fixtures(&roots);
    let first = roots.content.join("maps").join("first");
    let second = roots.content.join("maps").join("second");
    write_config(&first, &config("first", &[fixture("first")]));
    write_config(&second, &config("second", &[fixture("second")]));
    let old = "openMapper junction: lights=1 main=30 side=20 crossings= helpers=omsi";
    let opts = crate::junction::Options::parse(old);
    println!("Old SCO:\n{old}\nParsed: {opts:?}");
    assert!(opts.signal_set.is_empty());

    let mut ed = crate::editor::Editor::default();
    ed.content_dir = Some(roots.content.clone());
    for (dir, expected) in [(&first, "first"), (&second, "second")] {
        let mut doc = crate::selection::tests::flat_map(1);
        doc.dir = dir.clone();
        ed.open(doc);
        let sets = &ed.doc.as_ref().unwrap().signal_sets;
        assert_eq!(report(sets, &opts.signal_set), expected);
        assert_eq!(report(sets, "first"), expected);
    }
    // Reopening the map replaces the current configuration even when its file was removed.
    std::fs::remove_file(config_path(&second)).unwrap();
    let doc = ed.doc.take().unwrap();
    ed.open(doc);
    assert_eq!(
        report(&ed.doc.as_ref().unwrap().signal_sets, &opts.signal_set),
        "mc_small"
    );
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn junction_signal_pole_is_behind_the_head_at_omsi_headings() {
    let sets = SignalSets::default();
    let mut set = sets.builtin;
    let head = DVec3::new(20.0, 30.0, 14.55);
    let mut old = set.clone();
    old.pole_offset[1] = 0.47;
    for (heading, expected_xy) in [
        (0.0, [0.0, -0.47]),
        (90.0, [-0.47, 0.0]),
        (180.0, [0.0, 0.47]),
        (270.0, [0.47, 0.0]),
    ] {
        let pole = set.pole_position(head, heading, false);
        let offset = pole - head;
        let renderer_offset =
            omsi_geometry::object_rotation(omsi_geometry::map_rotation([heading, 0.0, 0.0]))
                .transform_vector3(DVec3::from_array(set.pole_offset).as_vec3());
        let expected = DVec2::from_array(expected_xy).extend(-2.55);
        println!(
            "heading={heading} head={head:?} old local={:?} old pole={:?} new local={:?} new pole={pole:?} expected={:?} renderer offset={renderer_offset:?}",
            old.pole_offset,
            old.pole_position(head, heading, false),
            set.pole_offset,
            head + expected,
        );
        assert!((offset - expected).length() < 1e-9);
        assert!((offset - renderer_offset.as_dvec3()).length() < 1e-6);
    }
    set.pole_offset[0] = 0.31;
    let right = set.pole_position(head, 90.0, false);
    let left = set.pole_position(head, 90.0, true);
    println!(
        "heading=90 head={head:?} local={:?}, RHT pole={right:?}, LHT pole={left:?}",
        set.pole_offset
    );
    assert!((right - head - DVec3::new(-0.47, -0.31, -2.55)).length() < 1e-9);
    assert!((left - head - DVec3::new(-0.47, 0.31, -2.55)).length() < 1e-9);
    set.mirror_x_lht = false;
    assert_eq!(set.pole_position(head, 90.0, true), right);
}

#[test]
fn junction_signal_pair_uses_two_tiles_and_undoes_together() {
    let roots = scratch("junction-signal-tile");
    fixtures(&roots);
    let map = roots.content.join("maps").join("map");
    write_config(&map, &config("custom", &[fixture("custom")]));
    let assets = Assets::new(roots.omsi.clone(), roots.content.clone());
    let sets = SignalSets::load(&map, &roots.content);
    let (set, _) = sets.resolve("");
    let height_modes = set.height_modes(&assets);
    assert_eq!(height_modes, [true, false]);
    let mut doc = crate::selection::tests::flat_map(1);
    let head = DVec3::new(crate::doc::tile_size() - 0.2, 50.0, 14.55);
    let at = || Placement {
        junction: 123,
        light: 1,
        heading: 270.0,
        head,
    };
    let old_next_id = doc.global.next_id_code;
    set.place(&mut doc, None, at(), height_modes);
    println!(
        "Missing pole tile: head={head:?}, children={:?}",
        crate::junction::children(&doc, 123)
    );
    assert!(crate::junction::children(&doc, 123).is_empty());
    assert_eq!(doc.global.next_id_code, old_next_id);

    doc.add_tile((1, 0));
    doc.tiles.get_mut(&(1, 0)).unwrap().terrain.heights.fill(7.0);
    let mut history = History::default();
    history.begin("place signal");
    history.touch_global(&doc);
    set.place(&mut doc, Some(&mut history), at(), height_modes);
    history.commit();
    let h = &doc.tiles[&(0, 0)].tile.objects[0];
    let p = &doc.tiles[&(1, 0)].tile.objects[0];
    let pole = head + DVec3::new(0.47, 0.0, -2.55);
    println!("Head (0, 0) {h:?} world={head:?}; pole (1, 0) {p:?} world={pole:?}");
    assert!((doc.object_world((0, 0), h, true) - head).length() < 1e-9);
    assert!((doc.object_world((1, 0), p, false) - pole).length() < 1e-9);
    assert_eq!(
        (h.flag, h.extra.as_slice(), h.var_parent),
        (1, &["1".to_string()][..], Some(123))
    );
    assert_eq!((p.flag, p.extra.len(), p.var_parent), (0, 0, Some(123)));
    assert_eq!(h.rot, [270.0, 0.0, 0.0]);
    assert_eq!(h.rot, p.rot);
    history.undo(&mut doc);
    assert!(crate::junction::children(&doc, 123).is_empty());
    history.redo(&mut doc);
    assert_eq!(crate::junction::children(&doc, 123).len(), 2);
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn junction_signal_configuration_is_untouched_when_saving_a_copy() {
    let roots = scratch("junction-signal-save");
    let from = roots.omsi.join("maps").join("original");
    let to = roots.content.join("maps").join("copied");
    write_config(&from, &config("other", &[fixture("other")]));
    let before = std::fs::read(config_path(&from)).unwrap();
    let mut doc = crate::doc::tests::sample_map(&roots);
    doc.dir = to.clone();
    doc.source = Some(from.clone());
    crate::doc::save(&mut doc).unwrap();
    println!(
        "Source {}: {}; copy {} exists: {}",
        config_path(&from).display(),
        String::from_utf8_lossy(&before),
        config_path(&to).display(),
        config_path(&to).exists()
    );
    assert!(!config_path(&to).exists());
    assert_eq!(std::fs::read(config_path(&from)).unwrap(), before);
    write_config(&to, &config("custom", &[fixture("custom")]));
    let before = std::fs::read(config_path(&to)).unwrap();
    crate::doc::save(&mut doc).unwrap();
    assert_eq!(std::fs::read(config_path(&to)).unwrap(), before);
    let _ = std::fs::remove_dir_all(&roots.content);
}
