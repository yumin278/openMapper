use super::*;

/// Substitute only the X fields in the original text. All other data must be byte-identical.
fn translated(text: &str, dx: f32) -> String {
    let mut skip = 0;
    let mut xs = 0;
    text.lines()
        .map(|line| {
            if skip > 0 {
                skip -= 1;
            } else if xs > 0 {
                xs -= 1;
                return num(line.parse::<f32>().unwrap() + dx);
            } else {
                match line {
                    "[heightprofile]" => xs = 2,
                    "[profilepnt]" => xs = 1,
                    "[path]" => {
                        skip = 1;
                        xs = 1;
                    }
                    _ => {}
                }
            }
            line.into()
        })
        .collect::<Vec<String>>()
        .join("\n")
        + "\n"
}

#[test]
fn road_maker_x_offset_is_output_only_in_both_writers() {
    for imported_sli in [false, true] {
        let mut road = RoadSpec {
            name: "Test Road".into(),
            imported_sli,
            lane_width: 3.278,
            roadway_tex: "road.bmp".into(),
            footway_tex: "side.bmp".into(),
            ..Default::default()
        }
        .normalised(false);
        let before = road.sli(false);
        let legacy_json = serde_json::to_string_pretty(&road).unwrap();
        assert!(!legacy_json.contains("x_offset"));
        let legacy: RoadSpec = serde_json::from_str(&legacy_json).unwrap();
        assert_eq!(legacy.x_offset, 0.0);
        assert_eq!(legacy.sli(false), before);
        let layout = road.strip_positions();
        println!("Imported={imported_sli}\nOriginal JSON:\n{legacy_json}\nOriginal SLI:\n{before}");
        assert!(before.contains("[path]\n0\n-1.639\n") && before.contains("[path]\n0\n1.639\n"));
        for dx in [2.0, -2.0, 0.0] {
            road.x_offset = dx;
            let after = road.sli(false);
            println!("Offset {dx:+.3}, layout {layout:?}\nSLI:\n{after}");
            assert_eq!(road.strip_positions(), layout);
            assert_eq!(
                after,
                if dx == 0.0 {
                    before.clone()
                } else {
                    translated(&before, dx)
                }
            );
            if dx == 2.0 {
                assert!(
                    after.contains("[path]\n0\n0.361\n") && after.contains("[path]\n0\n3.639\n")
                );
            }
        }
    }
}

#[test]
fn road_maker_x_offset_can_place_a_half_width_road_on_either_side() {
    for (dx, lo, hi, path_x) in [
        (1.75, "0.000", "3.500", "1.750"),
        (-1.75, "-3.500", "0.000", "-1.750"),
    ] {
        let road = RoadSpec {
            lanes_back: 0,
            footway: 0.0,
            x_offset: dx,
            ..Default::default()
        };
        let sli = road.sli(false);
        println!("3.5 m single lane; X offset {dx:+.3}\n{sli}");
        assert!(sli.contains(&format!("[heightprofile]\n{lo}\n{hi}\n")));
        assert!(sli.contains(&format!("[path]\n0\n{path_x}\n")));
    }
}

#[test]
fn road_maker_x_offset_save_as_preserves_source_and_checks_collisions() {
    let root = std::env::temp_dir().join(format!("om-road-maker-offset-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let tex = root.join("road.bmp");
    std::fs::write(&tex, [0]).unwrap();
    let road = RoadSpec {
        name: "Original".into(),
        roadway_tex: tex,
        ..Default::default()
    };
    let rel = road.write_in_maker(&root, false, None).unwrap();
    let path = crate::doc::safe::own_path(&root, &rel);
    let json = path.with_extension("road.json");
    let before_sli = std::fs::read(&path).unwrap();
    let before_json = std::fs::read(&json).unwrap();
    let target = MakerTarget::open(rel, path.clone()).unwrap();
    let mut renamed = RoadSpec::of_with_traffic(&path, false).unwrap();
    assert_eq!(renamed.x_offset, 0.0);
    renamed.name = "Shifted road".into();
    let before_translation = renamed.sli(false);
    renamed.x_offset = 2.0;
    assert!(renamed.maker_name_changed(Some(&target)));
    let new_rel = renamed
        .write_in_maker_as_new(&root, false, &target)
        .unwrap();
    let new_path = crate::doc::safe::own_path(&root, &new_rel);
    let new_json = new_path.with_extension("road.json");
    let after_sli = std::fs::read(&new_path).unwrap();
    let after_json = std::fs::read(&new_json).unwrap();
    println!(
        "Source SLI:\n{}\nSource JSON:\n{}\nNew SLI (offset +2.000):\n{}\nNew JSON:\n{}",
        String::from_utf8_lossy(&before_sli),
        String::from_utf8_lossy(&before_json),
        String::from_utf8_lossy(&after_sli),
        String::from_utf8_lossy(&after_json)
    );
    assert_eq!(
        omsi_cfg::vfs::read_text(&new_path)
            .unwrap()
            .replace("\r\n", "\n"),
        translated(&before_translation, 2.0)
    );
    assert_eq!(
        RoadSpec::of_with_traffic(&new_path, false)
            .unwrap()
            .x_offset,
        2.0
    );
    assert!(
        renamed
            .write_in_maker_as_new(&root, false, &target)
            .is_err()
    );
    assert_eq!(std::fs::read(new_path).unwrap(), after_sli);
    assert_eq!(std::fs::read(new_json).unwrap(), after_json);
    assert_eq!(std::fs::read(path).unwrap(), before_sli);
    assert_eq!(std::fs::read(json).unwrap(), before_json);
    std::fs::remove_dir_all(&root).unwrap();
}
