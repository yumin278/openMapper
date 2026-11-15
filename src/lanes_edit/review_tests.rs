use super::*;
use crate::road_maker::{RoadSpec, Strip};

#[test]
fn detaching_transfers_all_live_rule_kinds_to_each_resulting_ai_path() {
    let root = std::env::temp_dir().join(format!("om-detach-rules-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let tex = root.join("asphalt.png");
    image::RgbaImage::from_pixel(4, 4, image::Rgba([80, 80, 80, 255]))
        .save(&tex)
        .unwrap();
    let mut road = RoadSpec {
        name: "Detachable road".into(),
        roadway_tex: tex,
        ..Default::default()
    }
    .normalised(false);
    if let Strip::Lane { bus_only, .. } = &mut road.strips[2] {
        *bus_only = true;
    }
    road.strips.insert(3, Strip::BusBay { width: 3.0 });
    let file = road.write(&root).unwrap();
    // An additional bidirectional vehicle path must split into two AI splines.
    let path = omsi_cfg::resolve_path(&root, &file);
    let mut text = omsi_cfg::vfs::read_text(&path).unwrap();
    text.push_str("\n[path]\n0\n0\n0.1\n3\n2\n\n");
    std::fs::write(&path, omsi_cfg::encode_text(&text)).unwrap();
    for (file, kind) in [("car.sli", 0), ("walk.sli", 1)] {
        std::fs::write(root.join(file), format!("[path]\n{kind}\n0\n0\n3\n0\n\n")).unwrap();
    }
    let mut assets = Assets::new(root.join("omsi"), root.clone());
    let source = assets.spline_wait(&file, 5000).unwrap();
    assets.spline_wait("car.sli", 5000).unwrap();
    assets.spline_wait("walk.sli", 5000).unwrap();
    let mut doc = crate::selection::tests::flat_map(1);
    doc.global.next_id_code = 10;
    let mut rules = Vec::new();
    for (p, _) in source.def.paths.iter().enumerate() {
        for (i, kind) in [
            "bus",
            "no_cars",
            "speedlimit",
            "trafficdensity",
            "priority",
            "trucks",
        ]
        .iter()
        .enumerate()
        {
            rules.push(omsi_map::MapRule {
                path_index: p as i32,
                kind: (*kind).into(),
                value: (p * 10 + i) as f64,
                extra: 7.0,
                ..Default::default()
            });
        }
        rules.push(omsi_map::MapRule {
            path_index: p as i32,
            kind: "killed".into(),
            kill: true,
            ..Default::default()
        });
    }
    doc.tiles
        .get_mut(&(0, 0))
        .unwrap()
        .tile
        .splines
        .push(MapSpline {
            file: file.clone(),
            id: 1,
            pos: [100.0, 100.0, 0.0],
            length: 60.0,
            rules: rules.clone(),
            ..Default::default()
        });
    let free = std::collections::HashMap::from([(file.to_lowercase(), "without_paths.sli".into())]);
    assert_eq!(
        detach(
            &mut doc,
            &mut assets,
            &[((0, 0), 1)],
            &free,
            "car.sli",
            "walk.sli"
        ),
        1
    );
    assert!(doc.spline((0, 0), 1).unwrap().rules.is_empty());
    for (p, path) in source.def.paths.iter().enumerate() {
        let expected: Vec<_> = rules
            .iter()
            .filter(|r| r.path_index == p as i32 && !r.kill)
            .map(|r| omsi_map::MapRule {
                path_index: 0,
                ..r.clone()
            })
            .collect();
        let lanes: Vec<_> = doc.tiles[&(0, 0)]
            .tile
            .splines
            .iter()
            .filter(|s| s.id != 1 && (s.pos[0] - 100.0 - f64::from(path.start[0])).abs() < 1e-3)
            .collect();
        assert_eq!(
            lanes.len(),
            if path.direction == 2 { 2 } else { 1 },
            "source path {p}"
        );
        for lane in lanes {
            assert_eq!(
                lane.rules, expected,
                "source path {p}, new spline {}",
                lane.id
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
