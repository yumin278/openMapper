use super::*;
use crate::road_maker::{RoadSpec, Strip};
use crate::selection::tests::flat_map;

struct Fixture {
    root: std::path::PathBuf,
    bus: String,
    plain: String,
    assets: Assets,
    editor: Editor,
}
impl Fixture {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("om-bus-rules-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let tex = root.join("asphalt.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([80, 80, 80, 255]))
            .save(&tex)
            .unwrap();
        let plain_spec = RoadSpec {
            name: "Plain road".into(),
            roadway_tex: tex,
            ..Default::default()
        };
        let plain = plain_spec.write(&root).unwrap();
        let mut spec = plain_spec.normalised(false);
        spec.name = "Bus road".into();
        if let Strip::Lane { bus_only, .. } = &mut spec.strips[2] {
            *bus_only = true;
        }
        spec.strips.insert(3, Strip::BusBay { width: 3.0 });
        let bus = spec.write(&root).unwrap();
        let mut doc = flat_map(1);
        doc.dir = root.join("maps/Rules");
        let mut editor = Editor::default();
        editor.content_dir = Some(root.clone());
        editor.open(doc);
        editor.auto_junctions = false;
        Self {
            assets: Assets::new(root.join("omsi"), root.clone()),
            root,
            bus,
            plain,
            editor,
        }
    }
    fn lay(&mut self) -> Vec<(TileKey, i64)> {
        let start = Anchor {
            pos: DVec3::new(100.0, 50.0, 0.0),
            ..Default::default()
        };
        let pieces = vec![Seg {
            start: start.pos.truncate(),
            heading: 0.0,
            length: 80.0,
            radius: 0.0,
        }];
        self.editor.history.begin("test road");
        let doc = self.editor.doc.as_ref().unwrap();
        self.editor.history.touch_tile(doc, (0, 0));
        self.editor.history.touch_global(doc);
        let made = self
            .editor
            .lay_road(
                &mut self.assets,
                &self.bus,
                &pieces,
                0.0,
                0.0,
                start,
                None,
                5.0,
            )
            .unwrap();
        self.editor.history.commit();
        made
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn assert_rules(doc: &Document, instances: &[(TileKey, i64)]) {
    for &(key, id) in instances {
        let s = doc.spline(key, id).unwrap();
        for index in [2, 3] {
            for kind in ["bus", "no_cars"] {
                assert_eq!(
                    s.rules
                        .iter()
                        .filter(|r| r.path_index == index && r.kind == kind && !r.kill)
                        .count(),
                    1,
                    "{id}: {:?}",
                    s.rules
                );
            }
        }
    }
}

#[test]
fn laying_a_made_road_applies_bus_lane_and_bus_bay_rules_and_undoes_them() {
    let mut f = Fixture::new("lay");
    let made = f.lay();
    assert_rules(f.editor.doc.as_ref().unwrap(), &made);
    let path = omsi_cfg::resolve_path(&f.root, &f.bus);
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.with_extension("road.json")).unwrap()).unwrap();
    assert_eq!(json["bus_paths"], serde_json::json!([2]));
    assert_eq!(json["bay_paths"], serde_json::json!([3]));
    f.editor.undo();
    assert!(made.iter().all(|&(k, id)| {
        f.editor
            .doc
            .as_ref()
            .unwrap()
            .spline(k, id)
            .is_none_or(|s| s.deleted)
    }));
}

#[test]
fn split_paste_parallel_and_reshape_fill_missing_rules_without_duplicates() {
    let mut f = Fixture::new("copies");
    let made = f.lay();
    let doc = f.editor.doc.as_mut().unwrap();
    doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0]
        .rules
        .clear();
    let new = doc.split_spline(made[0].0, made[0].1, 30.0).unwrap();
    assert_rules(doc, &[((0, 0), new)]);
    let copied = crate::parallel::copy(doc, &made, 10.0).unwrap();
    assert_rules(doc, &copied);
    let clip = crate::clipboard::copy(doc, &[Sel::Spline(made[0].0, made[0].1)], |_| false);
    let pasted = crate::clipboard::paste(doc, &clip, DVec3::new(140.0, 50.0, 0.0));
    let pasted: Vec<_> = pasted
        .into_iter()
        .filter_map(|s| {
            if let Sel::Spline(k, id) = s {
                Some((k, id))
            } else {
                None
            }
        })
        .collect();
    assert_rules(doc, &pasted);
    let plan = reshape::Plan {
        pieces: vec![reshape::Piece {
            key: made[0].0,
            id: made[0].1,
            segs: vec![Seg {
                start: DVec2::new(100.0, 50.0),
                heading: 0.0,
                length: 120.0,
                radius: 0.0,
            }],
            z0: 0.0,
            z1: 0.0,
            g0: None,
            g1: None,
        }],
    };
    reshape::apply(doc, &plan);
    let made: Vec<_> = doc.tiles[&(0, 0)]
        .tile
        .splines
        .iter()
        .filter(|s| !s.deleted)
        .map(|s| ((0, 0), s.id))
        .collect();
    assert_rules(doc, &made);
}

#[test]
fn retyping_bus_to_plain_and_back_preserves_manual_speed_limits() {
    let mut f = Fixture::new("retype");
    let made = f.lay();
    let speed = omsi_map::MapRule {
        path_index: 2,
        kind: "speedlimit".into(),
        value: 30.0,
        ..Default::default()
    };
    for &(k, id) in &made {
        f.editor
            .doc
            .as_mut()
            .unwrap()
            .tiles
            .get_mut(&k)
            .unwrap()
            .tile
            .splines
            .iter_mut()
            .find(|s| s.id == id)
            .unwrap()
            .rules
            .push(speed.clone());
    }
    f.editor
        .set_selection(made.iter().map(|&(k, id)| Sel::Spline(k, id)).collect());
    f.editor.retype_splines(&f.plain);
    for &(k, id) in &made {
        let s = f.editor.doc.as_ref().unwrap().spline(k, id).unwrap();
        assert_eq!(s.file, f.plain);
        assert_eq!(s.rules, [speed.clone()]);
    }
    assert_eq!(
        f.editor.replace_everywhere(&f.plain, &f.bus, true),
        made.len()
    );
    assert_rules(f.editor.doc.as_ref().unwrap(), &made);
    for &(k, id) in &made {
        let s = f.editor.doc.as_ref().unwrap().spline(k, id).unwrap();
        assert_eq!(
            s.rules
                .iter()
                .filter(|r| r.kind == "speedlimit")
                .collect::<Vec<_>>(),
            vec![&speed]
        );
        assert_eq!(s.rules.len(), 5);
    }
    f.editor.undo();
    for &(k, id) in &made {
        assert_eq!(
            f.editor.doc.as_ref().unwrap().spline(k, id).unwrap().rules,
            [speed.clone()]
        );
    }
    f.editor.undo();
    assert_rules(f.editor.doc.as_ref().unwrap(), &made);
}

/// Exercise the actual Roads-tool flow, including cutting, type generation,
/// bay rule insertion and both taper junctions. The old pocket_type is untouched.
#[test]
fn pocket_tool_preserves_maker_bus_paths_and_bays_on_one_and_both_sides() {
    for left_bay in [true, false] {
        for both in [false, true] {
            let mut f = Fixture::new(&format!("pocket-{left_bay}-{both}"));
            if left_bay {
                let path = omsi_cfg::resolve_path(&f.root, &f.plain);
                let mut road = RoadSpec::of(&path).unwrap();
                road.name = "Left shoulder".into();
                road.strips.insert(1, Strip::BusBay { width: 3.0 });
                f.bus = road.write(&f.root).unwrap();
            }
            let source_path = omsi_cfg::resolve_path(&f.root, &f.bus);
            let source = RoadSpec::of(&source_path).unwrap();
            let mut original_indices = source.bus_paths.clone();
            original_indices.extend(&source.bay_paths);
            original_indices.sort_unstable();
            let main = f.assets.spline_wait(&f.bus, 5000).unwrap();
            f.lay();
            f.editor.pocket_both_sides = both;
            f.editor.pocket_width = 3.0;
            f.editor.pocket_len = 12.0;
            f.editor.cursor = Some(DVec3::new(100.0, 90.0, 0.0));
            f.editor.pocket_click(&mut f.assets);
            let doc = f.editor.doc.as_ref().unwrap();
            let bays: Vec<_> = doc
                .tiles
                .values()
                .flat_map(|t| &t.tile.splines)
                .filter(|s| !s.deleted && s.file.to_lowercase().contains("_bay_"))
                .collect();
            assert!(
                !bays.is_empty(),
                "left_bay={left_bay}, both={both}: tool produced no bay"
            );
            assert_eq!(
                doc.tiles
                    .values()
                    .flat_map(|t| &t.tile.objects)
                    .filter(|o| o.file.contains("junction_"))
                    .count(),
                2,
                "both taper junctions are produced"
            );
            for spline in bays {
                let bay = f.assets.spline_wait(&spline.file, 5000).unwrap();
                let added = crate::pocket::bay_path_indices(&main, &bay);
                let first = main.def.paths.len();
                assert_eq!(added, first..first + if both { 2 } else { 1 });
                for (i, (before, after)) in main.def.paths.iter().zip(&bay.def.paths).enumerate() {
                    assert_eq!(before.kind, after.kind, "original path {i}");
                    // Widening moves the outside footways. Existing vehicle paths
                    // retain their original position as well as their index.
                    if before.kind == 0 { assert_eq!(before.start, after.start, "original vehicle path {i}"); }
                    assert_eq!(before.direction, after.direction, "original path {i}");
                    assert_eq!(before.width, after.width, "original path {i}");
                }
                for i in added.clone() {
                    assert_eq!(bay.def.paths[i].kind, 0);
                }
                let expected: Vec<_> = original_indices.iter().copied().chain(added).collect();
                for &index in &expected {
                    assert_eq!(bay.def.paths[index].kind, 0);
                    for kind in ["bus", "no_cars"] {
                        assert_eq!(
                            spline
                                .rules
                                .iter()
                                .filter(|r| r.path_index == index as i32
                                    && r.kind == kind
                                    && !r.kill)
                                .count(),
                            1,
                            "left_bay={left_bay} both={both} path={index} kind={kind} rules={:?}",
                            spline.rules
                        );
                    }
                }
                assert_eq!(
                    spline.rules.len(),
                    expected.len() * 2,
                    "no rules drift to other paths"
                );
                println!(
                    "pocket compatibility: left_bay={left_bay}, both={both}, original_bus_paths={:?}, original_bay_paths={:?}, new_bay_paths={first}..{}, rules={:?}",
                    source.bus_paths,
                    source.bay_paths,
                    bay.def.paths.len(),
                    spline.rules
                );
            }
        }
    }
}
