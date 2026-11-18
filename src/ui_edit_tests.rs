//! Headless tests of what the object panel offers from an object's own settings: the `.sco` a
//! file names (content folder first, the installation after it), the counts read beside it, and
//! an object the editor has just written being there without a rescan or a restart.
//!
//! The panels are not drawn: what they decide on is checked here, which is where the bugs were
//! (a settings file turned into another settings file, a catalogue that never hears of the
//! editor's own write).

use super::*;

use omsi_map::MapObject;

use crate::assets::Assets;
use crate::crossing_config::{self, CrossingConfig, SignalPoint, TemplateRule};
use crate::install::Roots;

/// A plain crossing object: two `[path]`s and no light program of its own.
const PLAIN_SCO: &str = "[path]\n0\n0\n0\n0\n0\n10\n0\n0\n0\n3\n0\n0\n\n[path]\n0\n0\n0\n0\n0\n10\n0\n0\n0\n3\n0\n0\n";

/// A crossing object that runs a light program: what the catalogue calls `light_program`.
const LIT_SCO: &str = "[traffic_lights_group]\n0\n\n[traffic_light]\nMain\n\n[path]\n0\n0\n0\n0\n0\n10\n0\n0\n0\n3\n0\n0\n\n[path]\n0\n0\n0\n0\n0\n10\n0\n0\n0\n3\n0\n0\n";

fn point(light: usize) -> SignalPoint {
    SignalPoint { light, set: String::new(), x: 0.0, up: 0.0, fwd: 0.0, heading: 0.0 }
}

fn rule(kind: &str) -> TemplateRule {
    TemplateRule { path_index: 0, kind: kind.into(), value: 0.0, extra: 0.0 }
}

/// The `.sco` of `file` under `root`, written with `text`.
///
/// Like every writer of content, it tells the content caches the folder changed
/// ([`crate::hot::own_change`]): `omsi_cfg`'s per-folder name listings are cleared then, so a
/// folder made here is found by the next lookup (a listing taken before it existed would not
/// hold it).
fn write_sco(root: &std::path::Path, file: &str, text: &str) -> std::path::PathBuf {
    let abs = omsi_cfg::resolve_path(root, file);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    std::fs::write(&abs, text).unwrap();
    crate::hot::own_change();
    abs
}

/// An editor holding a map with one object, `file`'s `.sco` in the content folder.
fn editor_with(roots: &Roots, file: &str) -> Editor {
    let mut doc = crate::selection::tests::flat_map(1);
    doc.dir = roots.content.join("maps").join("Ui");
    doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects.push(MapObject { file: file.into(), id: 1, pos: [0.0, 0.0, 0.0], ..Default::default() });
    let mut ed = Editor::default();
    ed.content_dir = Some(roots.content.clone());
    ed.install_dir = Some(roots.omsi.clone());
    ed.open(doc);
    ed.select(Some(Sel::Object((0, 0), 1)));
    ed
}

#[test]
fn the_counts_come_from_the_settings_beside_the_sco() {
    let roots = crate::doc::tests::scratch("ui-edit-counts");
    let file = "Sceneryobjects\\Test\\Kreuzung.sco";
    let abs = write_sco(&roots.content, file, PLAIN_SCO);
    let mut ed = editor_with(&roots, file);
    // (nothing beside it yet: a panel is offered nothing)
    assert_eq!(settings_counts(&ed, file), Some((0, 0)));
    assert!(!any_template(&ed));
    assert!(!sco_light_program(&ed, &Library::default(), file), "no [traffic_light] in the .sco");
    // (a .sco that runs a light program is read the same way when the catalogue has not seen it)
    let lit = "Sceneryobjects\\Test\\Lit.sco";
    write_sco(&roots.content, lit, LIT_SCO);
    assert!(sco_light_program(&ed, &Library::default(), lit), "its own [traffic_light] is read");
    assert!(sco_light_program(&ed, &Library::default(), lit), "and cached by the file's size and time");

    crossing_config::write(&abs, &CrossingConfig::of(None, vec![point(0), point(1)], vec![rule("bus")]), 2).unwrap();
    assert_eq!(settings_counts(&ed, file), Some((2, 1)));
    assert!(any_template(&ed), "a template beside the object is offered");

    // (the same file, read as the catalogue would have it: the light program comes from there)
    let mut lib = Library::default();
    lib.assets.push(crate::library::Asset {
        kind: crate::library::Kind::Object,
        rel: file.into(),
        name: "Kreuzung".into(),
        dir: "Test".into(),
        category: crate::library::Category::Junctions,
        haystack: String::new(),
        from_content: true,
        is_bus_stop: false,
        is_entry_point: false,
        is_car_park: false,
        abs_height: false,
        lamp: false,
        light_head: false,
        light_program: true,
        helpers: Vec::new(),
        paths: 2,
        half_width: 0.0,
        lanes: (0, 0, 0),
    });
    lib.reindex_for_tests();
    assert!(sco_light_program(&ed, &lib, file), "the catalogue's own answer is taken first");

    ed.select(None);
    assert!(!any_template(&ed), "nothing selected, nothing offered");
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn the_sco_is_looked_for_in_the_content_folder_and_then_the_installation() {
    let roots = crate::doc::tests::scratch("ui-edit-install");
    let file = "Sceneryobjects\\Test\\Both.sco";
    let in_content = write_sco(&roots.content, file, PLAIN_SCO);
    let in_install = write_sco(&roots.omsi, file, PLAIN_SCO);
    let ed = editor_with(&roots, file);
    // (both roots have it: the content folder's copy is the one the game takes)
    assert_eq!(sco_path(&ed, file).as_deref(), Some(in_content.as_path()), "the content folder's copy first");
    crossing_config::write(&in_content, &CrossingConfig::of(None, vec![point(0)], Vec::new()), 2).unwrap();
    crossing_config::write(&in_install, &CrossingConfig::of(None, vec![point(0), point(1)], Vec::new()), 2).unwrap();
    assert_eq!(settings_counts(&ed, file), Some((1, 0)), "the content copy's settings are the object's");

    // (an object only the installation has is found there, its settings beside it)
    std::fs::remove_file(&in_content).unwrap();
    assert_eq!(sco_path(&ed, file).as_deref(), Some(in_install.as_path()), "the installation after it");
    assert_eq!(settings_counts(&ed, file), Some((2, 0)));
    let _ = std::fs::remove_dir_all(&roots.content);
    let _ = std::fs::remove_dir_all(&roots.omsi);
}

/// 兩代副本各自顯示自己的設定（檔案層級）：改指回舊副本時顯示舊的，不是最新或原始。
/// 真正的「連續兩次另存」走 `CrossingPaths::save`，見
/// `ui_crossing::tests::two_generations_of_copies_show_the_newest_settings`。
#[test]
fn a_copy_of_a_copy_shows_the_newest_one() {
    let roots = crate::doc::tests::scratch("ui-edit-copies");
    let source = "Sceneryobjects\\Test\\Kreuzung.sco";
    let first = "Sceneryobjects\\Test\\Kreuzung_aaaaaa.sco";
    let second = "Sceneryobjects\\Test\\Kreuzung_bbbbbb.sco";
    let source_abs = write_sco(&roots.content, source, PLAIN_SCO);
    let first_abs = write_sco(&roots.content, first, PLAIN_SCO);
    let second_abs = write_sco(&roots.content, second, PLAIN_SCO);
    crossing_config::write(&source_abs, &CrossingConfig::of(None, vec![point(0)], Vec::new()), 2).unwrap();
    crossing_config::write(&first_abs, &CrossingConfig::of(Some(source.into()), vec![point(0), point(1)], Vec::new()), 2).unwrap();
    crossing_config::write(&second_abs, &CrossingConfig::of(Some(first.into()), vec![point(0), point(1), point(2)], vec![rule("bus")]), 2).unwrap();

    // (the map object points at the newest copy, as saving a copy again does)
    let mut ed = editor_with(&roots, second);
    assert_eq!(settings_counts(&ed, second), Some((3, 1)), "the newest copy's own settings");
    assert!(any_template(&ed));
    assert_eq!(crossing_config::CrossingConfig::load(&second_abs, 2).unwrap().copy_from.as_deref(), Some(first), "its own source, not the original");

    // (pointed back at the first copy, the panel shows that one's settings, not the newest)
    ed.doc.as_mut().unwrap().tiles.get_mut(&(0, 0)).unwrap().tile.objects[0].file = first.into();
    assert_eq!(settings_counts(&ed, first), Some((2, 0)));
    assert!(!any_template(&ed), "the first copy has no template");
    let _ = std::fs::remove_dir_all(&roots.content);
}

/// Four roads of a plain type in `content`, loaded into `assets`, and a map holding them.
fn four_arm_junction(tag: &str) -> (Roots, Assets, crate::doc::Document, DVec3, Vec<crate::junction::Arm>) {
    let roots = crate::doc::tests::scratch(tag);
    let content = roots.content.clone();
    std::fs::create_dir_all(content.join("Sceneryobjects")).unwrap();
    let tex = content.join("asph.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([80, 80, 80, 255])).save(&tex).unwrap();
    let side = content.join("side.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([150, 150, 150, 255])).save(&side).unwrap();
    let road = crate::road_maker::RoadSpec { name: "Plain".into(), roadway_tex: tex, footway_tex: side, ..Default::default() }.write(&content).unwrap();
    let mut assets = Assets::new(roots.omsi.clone(), content.clone());
    assert!(assets.spline_wait(&road, 5000).is_some(), "{road} loads");
    let mut doc = crate::selection::tests::flat_map(1);
    doc.dir = content.join("maps").join("Auto");
    std::fs::create_dir_all(&doc.dir).unwrap();
    let centre = DVec3::new(100.0, 100.0, 0.0);
    let mut arms = Vec::new();
    for (i, out) in [0.0_f64, 90.0, 180.0, 270.0].into_iter().enumerate() {
        let at = centre.truncate() + crate::roads::dir(out) * 10.0;
        let id = 300 + i as i64;
        crate::selection::tests::add_spline(&mut doc, (0, 0), id, at.x, at.y, out, 20.0, 0, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.last_mut().unwrap().file = road.clone();
        arms.push(crate::junction::Arm { key: (0, 0), id, at_end: false });
    }
    (roots, assets, doc, centre, arms)
}

#[test]
fn an_auto_junction_with_lights_shows_at_once_without_a_restart() {
    let (roots, mut assets, mut doc, centre, arms) = four_arm_junction("ui-edit-auto-lights");
    let content = roots.content.clone();
    let opts = crate::junction::Options { lights: true, crossings: vec![0.0, 180.0], zebra_texture: false, ..Default::default() };
    let id = crate::junction::make_with(&mut doc, &mut assets, &content, None, centre, &arms, &opts, crate::junction::MakeEdit::default()).unwrap();
    let object = doc.tiles[&(0, 0)].tile.objects.iter().find(|o| o.id == id).unwrap().clone();
    let mut ed = Editor::default();
    ed.content_dir = Some(content.clone());
    ed.install_dir = Some(roots.omsi.clone());
    ed.open(doc);
    ed.select(Some(Sel::Object((0, 0), id)));

    // (the catalogue has not read it yet: a restart or a rescan used to be the only way)
    let mut lib = Library::default();
    assert!(lib.find(&object.file).is_none(), "{} is not in the catalogue yet", object.file);
    assert!(sco_light_program(&ed, &lib, &object.file), "the .sco itself says it has a light program");
    let (signals, _) = settings_counts(&ed, &object.file).expect("the object is there");
    assert!(signals > 0, "the generated junction's signal positions are beside its .sco");

    // (what the application does each frame: the written object is read into the catalogue)
    lib.refresh_pending(&roots);
    let asset = lib.find(&object.file).expect("the written junction is in the catalogue");
    assert!(asset.light_program, "the catalogue sees the light program now");
    assert!(asset.paths > 0, "its paths are read too");
    assert!(sco_light_program(&ed, &lib, &object.file), "the panel offers the signals");
    let _ = std::fs::remove_dir_all(&roots.content);
    let _ = std::fs::remove_dir_all(&roots.omsi);
}

#[test]
fn what_was_written_under_another_content_folder_is_not_read_here() {
    let first = crate::doc::tests::scratch("ui-edit-queue-a");
    let second = crate::doc::tests::scratch("ui-edit-queue-b");
    let rel = "Sceneryobjects\\Test\\Shared.sco";
    // (the same path in two content folders, with different objects: a queue that kept only
    // the path could read the wrong one, or swallow the other folder's)
    write_sco(&first.content, rel, LIT_SCO);
    write_sco(&second.content, rel, PLAIN_SCO);
    crate::library::note_written(&first.content, rel);

    let mut lib = Library::default();
    assert_eq!(lib.refresh_pending(&second), 0, "nothing was written under the second folder");
    assert!(lib.find(rel).is_none(), "the first folder's object is not read into the second's catalogue");

    assert_eq!(lib.refresh_pending(&first), 1, "it was kept for the folder it was written under");
    let asset = lib.find(rel).expect("read from its own content folder");
    assert!(asset.light_program, "the first folder's copy, not the second's");
    let _ = std::fs::remove_dir_all(&first.content);
    let _ = std::fs::remove_dir_all(&second.content);
}

#[test]
fn an_object_persisted_into_the_installation_is_read_there() {
    let roots = crate::doc::tests::scratch("ui-edit-persist");
    let rel = "Sceneryobjects\\openMapper-Persisted\\Map\\Junction-7.sco";
    let abs = write_sco(&roots.omsi, rel, LIT_SCO);
    crossing_config::write(&abs, &CrossingConfig::of(None, vec![point(0)], Vec::new()), 2).unwrap();
    // (persisting writes into the installation, and the map object points at that copy)
    let ed = editor_with(&roots, rel);
    assert_eq!(settings_counts(&ed, rel), Some((1, 0)), "the settings beside the installation's copy");
    assert!(sco_light_program(&ed, &Library::default(), rel), "its .sco is read from the installation");

    // (what the application does when a junction is persisted)
    let mut lib = Library::default();
    lib.refresh_crossing(&roots, rel);
    assert!(lib.find(rel).is_some_and(|a| a.light_program), "the catalogue reads it from the installation too");
    let _ = std::fs::remove_dir_all(&roots.content);
    let _ = std::fs::remove_dir_all(&roots.omsi);
}
