use super::*;

fn lane(dir: Direction, width: f32, bus_only: bool) -> Strip {
    Strip::Lane {
        dir,
        width,
        bus_only,
    }
}
fn spec() -> RoadSpec {
    RoadSpec {
        left_hand: Some(false),
        strips: vec![
            Strip::Footway {
                width: 2.0,
                tex: PathBuf::new(),
            },
            Strip::BusBay { width: 3.0 },
            lane(Direction::Back, 3.0, true),
            Strip::Median {
                width: 2.0,
                raised: true,
                tex: PathBuf::new(),
            },
            lane(Direction::Fwd, 4.0, false),
            lane(Direction::Fwd, 3.5, true),
            Strip::BusBay { width: 2.5 },
            Strip::Footway {
                width: 1.0,
                tex: PathBuf::new(),
            },
        ],
        ..Default::default()
    }
}

#[test]
fn unequal_lanes_and_two_bays_emit_actual_paths_and_share_one_bay_texture() {
    let root = std::env::temp_dir().join(format!("om-layout-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let tex = root.join("asphalt.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([80, 80, 80, 255]))
        .save(&tex)
        .unwrap();
    let mut road = spec();
    road.roadway_tex = tex;
    road.name = "Layout".into();
    road.write(&root).unwrap();
    let path = root.join("Splines/openMapper/Layout.sli");
    let spline = omsi_scenery::Spline::load(&path).unwrap();
    let actual: Vec<_> = spline
        .paths
        .iter()
        .map(|p| (p.kind, p.start[0], p.width, p.direction))
        .collect();
    assert_eq!(
        actual,
        vec![
            (1, -10.25, 0.8, 2),
            (0, -7.75, 2.7, 1),
            (0, -4.75, 2.7, 1),
            (0, 0.75, 3.6, 0),
            (0, 4.5, 3.15, 0),
            (0, 7.5, 2.25, 0),
            (1, 9.25, 0.4, 2)
        ]
    );
    assert_eq!(
        spline
            .textures
            .iter()
            .filter(|t| t.file == "Layout_bay.png")
            .count(),
        1
    );
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.with_extension("road.json")).unwrap()).unwrap();
    assert_eq!(json["bus_paths"], serde_json::json!([2, 4]));
    assert!(json.get("lane_width").is_none());
    let loaded = RoadSpec::of(&path).unwrap();
    assert!(loaded.advanced);
    assert_eq!(loaded.strips, road.strips);
    assert_eq!(loaded.sli(false), road.sli(false));
    assert!(
        road.standard_markings()
            .iter()
            .any(|m| m.x == 2.75 && m.style == MarkStyle::Dashed)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn switching_to_simple_mode_rejects_lossy_settings_and_projects_representable_roads() {
    let mut road = spec();
    road.advanced = true;
    let before = road.strips.clone();
    let reasons = road.use_simple_mode().unwrap_err();
    assert!(reasons.iter().any(|r| r.contains("寬度")));
    assert!(reasons.iter().any(|r| r.contains("公車")));
    assert!(reasons.iter().any(|r| r.contains("左側")));
    assert_eq!(road.strips, before);
    assert!(road.advanced);
    let mut road = RoadSpec {
        lanes_back: 2,
        lanes_fwd: 1,
        pocket: 3.0,
        ..Default::default()
    }
    .normalised(true);
    road.advanced = true;
    road.lanes_back = 99; // stale controls must never override the strips
    road.use_simple_mode().unwrap();
    assert_eq!(road.lanes_back, 2);
    assert_eq!(road.lanes_fwd, 1);
    assert_eq!(road.pocket, 3.0);
    assert!(!road.advanced);
}

#[test]
fn invalid_layout_is_rejected_before_creating_output_files() {
    let root = std::env::temp_dir().join(format!("om-invalid-layout-{}", std::process::id()));
    let mut road = spec();
    road.strips.swap(2, 4);
    assert!(road.write(&root).is_err());
    assert!(!root.exists());
}
