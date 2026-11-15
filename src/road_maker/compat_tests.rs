use super::legacy_reference::LegacyRoadSpec;
use super::*;

fn legacy(spec: &RoadSpec) -> LegacyRoadSpec {
    serde_json::from_value(serde_json::to_value(spec).unwrap()).unwrap()
}

fn assert_compatible(old: &str, new: &str, bay_name: Option<&str>) {
    if let Some(name) = bay_name {
        let texture = format!("[texture]\n{name}\n\n");
        assert_eq!(new.matches(&texture).count(), 1);
        let mut stripped = new.replacen(&texture, "", 1);
        let old_start = old.rfind("[profile]\n").unwrap();
        let old_end = old.find("[path]\n").unwrap();
        let new_start = stripped.rfind("[profile]\n").unwrap();
        let new_end = stripped.find("[path]\n").unwrap();
        let profile = &stripped[new_start..new_end];
        let texture_index = old.matches("[texture]\n").count();
        assert!(profile.starts_with(&format!("[profile]\n{texture_index}\n\n")));
        let points: Vec<_> = profile.split("[profilepnt]\n").skip(1).collect();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].lines().nth(2), Some("0.000"));
        assert_eq!(points[1].lines().nth(2), Some("1.000"));
        // Only the bay material and u coordinates may differ: its x/y/v also match.
        let old_points: Vec<_> = old[old_start..old_end]
            .split("[profilepnt]\n")
            .skip(1)
            .collect();
        for (a, b) in points.iter().zip(old_points) {
            for field in [0, 1, 3] {
                assert_eq!(a.lines().nth(field), b.lines().nth(field));
            }
        }
        stripped.replace_range(new_start..new_end, &old[old_start..old_end]);
        assert_eq!(stripped, old);
    } else {
        assert_eq!(new, old);
    }
}

#[test]
fn all_presets_match_the_frozen_integration_writer_in_both_traffic_sides() {
    let mut cases: Vec<_> = presets().into_iter().map(|(_, s)| s).collect();
    cases.extend([
        RoadSpec {
            lanes_fwd: 2,
            lanes_back: 2,
            lane_width: 3.25,
            median: 4.0,
            median_raised: true,
            footway: 0.0,
            footway_right: Some(2.0),
            ..Default::default()
        },
        RoadSpec {
            lanes_fwd: 2,
            lanes_back: 3,
            lane_width: 3.0,
            median: 2.0,
            median_raised: true,
            footway: 2.0,
            ..Default::default()
        },
        RoadSpec {
            lanes_fwd: 2,
            lanes_back: 2,
            lane_width: 3.25,
            median: 1.0,
            footway: 2.5,
            ..Default::default()
        },
        RoadSpec {
            lanes_fwd: 0,
            lanes_back: 3,
            footway_right: Some(0.0),
            ..Default::default()
        },
        RoadSpec {
            pocket: 3.0,
            footway: 2.0,
            ..Default::default()
        },
    ]);
    for (i, mut spec) in cases.into_iter().enumerate() {
        spec.name = format!("case_{i}");
        for textured in [false, true] {
            spec.roadway_tex = "asphalt.png".into();
            spec.footway_tex = if textured {
                "side.png".into()
            } else {
                PathBuf::new()
            };
            spec.median_tex = if textured {
                "grass.png".into()
            } else {
                PathBuf::new()
            };
            for marked in [false, true] {
                spec.marked = marked;
                spec.markings = if marked {
                    legacy(&spec).standard_markings()
                } else {
                    Vec::new()
                };
                for left_hand in [false, true] {
                    let old = legacy(&spec).sli(left_hand);
                    let new = spec.sli(left_hand);
                    let bay_name = spec.bay_tex_name();
                    assert_compatible(&old, &new, (spec.pocket > 0.0).then_some(bay_name.as_str()));
                }
            }
        }
    }
}

#[test]
fn old_roads_keep_their_traffic_side_when_opened_in_the_opposite_map() {
    let root = std::env::temp_dir().join(format!("om-strips-migrate-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let texture = root.join("asphalt.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([80, 80, 80, 255]))
        .save(&texture)
        .unwrap();
    for left_hand in [false, true] {
        for (back, fwd) in [(1, 2), (0, 2), (2, 0)] {
            let spec = RoadSpec {
                name: format!("old_{left_hand}_{back}_{fwd}"),
                lanes_back: back,
                lanes_fwd: fwd,
                roadway_tex: texture.clone(),
                ..Default::default()
            };
            let old = legacy(&spec);
            let original = old.sli(left_hand);
            let sli = root.join(format!("{}.sli", spec.file_stem()));
            std::fs::write(&sli, &original).unwrap();
            std::fs::write(
                sli.with_extension("road.json"),
                serde_json::to_vec(&old).unwrap(),
            )
            .unwrap();
            let migrated = RoadSpec::of_with_traffic(&sli, !left_hand).unwrap();
            assert_eq!(migrated.left_hand, Some(left_hand));
            assert_eq!(migrated.sli(!left_hand), original);
            migrated.write_with_traffic(&root, !left_hand).unwrap();
            let saved = root
                .join("Splines/openMapper")
                .join(sli.file_name().unwrap());
            let text = omsi_cfg::vfs::read_text(&saved)
                .unwrap()
                .replace("\r\n", "\n");
            assert_eq!(text, original);
            let json: serde_json::Value =
                serde_json::from_slice(&std::fs::read(saved.with_extension("road.json")).unwrap())
                    .unwrap();
            assert_eq!(json["left_hand"], left_hand);
            assert!(json["strips"].is_array());
            assert_eq!(
                RoadSpec::of_with_traffic(&saved, !left_hand)
                    .unwrap()
                    .sli(!left_hand),
                original
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn traffic_inference_prefers_json_and_falls_back_only_without_usable_paths() {
    let root = std::env::temp_dir().join(format!("om-strips-infer-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("old.sli");
    let spec = RoadSpec::default();
    assert!(migration::infer_traffic(&spec, &path, None, true));
    assert!(!migration::infer_traffic(&spec, &path, Some(false), true));
    std::fs::write(&path, legacy(&spec).sli(false)).unwrap();
    assert!(migration::infer_traffic(&spec, &path, Some(true), false));
    std::fs::write(&path, "[path]\n0\n0\n0.1\n3\n2\n").unwrap();
    assert!(migration::infer_traffic(&spec, &path, None, true));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bay_texture_copies_original_asphalt_and_sidecars_when_marked() {
    let root = std::env::temp_dir().join(format!("om-strips-bay-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let texture = root.join("asphalt.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([80, 80, 80, 255]))
        .save(&texture)
        .unwrap();
    std::fs::write(root.join("asphalt.png.cfg"), b"texture settings").unwrap();
    std::fs::write(root.join("asphalt.surf"), b"surface settings").unwrap();
    let mut spec = RoadSpec {
        name: "Bus road".into(),
        pocket: 3.0,
        roadway_tex: texture.clone(),
        marked: true,
        ..Default::default()
    };
    spec.markings = spec.standard_markings();
    spec.write(&root).unwrap();
    let folder = root.join("Splines/openMapper/texture");
    assert_eq!(
        std::fs::read(folder.join("Bus_road_bay.png")).unwrap(),
        std::fs::read(&texture).unwrap()
    );
    assert_eq!(
        std::fs::read(folder.join("Bus_road_bay.png.cfg")).unwrap(),
        b"texture settings"
    );
    assert_eq!(
        std::fs::read(folder.join("Bus_road_bay.surf")).unwrap(),
        b"surface settings"
    );
    let sli = omsi_scenery::Spline::load(&root.join("Splines/openMapper/Bus_road.sli")).unwrap();
    assert_eq!(sli.textures[0].file, "Bus_road_lines.bmp");
    assert_eq!(sli.textures.last().unwrap().file, "Bus_road_bay.png");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_pocket_files_migrate_resave_and_match_explicit_bus_bay_blocks() {
    let root = std::env::temp_dir().join(format!("om-bay-legacy-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let texture = root.join("asphalt.png");
    image::RgbaImage::from_pixel(4, 4, image::Rgba([80, 80, 80, 255]))
        .save(&texture)
        .unwrap();
    for left_hand in [false, true] {
        let spec = RoadSpec {
            name: format!("Legacy pocket {left_hand}"),
            lanes_back: 2,
            lanes_fwd: 1,
            pocket: 3.0,
            roadway_tex: texture.clone(),
            ..Default::default()
        };
        let old = legacy(&spec);
        let sli = root.join(format!("{}.sli", spec.file_stem()));
        std::fs::write(&sli, old.sli(left_hand)).unwrap();
        std::fs::write(
            sli.with_extension("road.json"),
            serde_json::to_vec(&old).unwrap(),
        )
        .unwrap();
        let migrated = RoadSpec::of_with_traffic(&sli, !left_hand).unwrap();
        assert_eq!(migrated.bay_paths, [4]);
        assert!(migrated.bus_paths.is_empty());
        assert_eq!(
            migrated.sli(!left_hand),
            spec.normalised(left_hand).sli(!left_hand)
        );
        let rel = migrated.write_with_traffic(&root, !left_hand).unwrap();
        let output = omsi_cfg::vfs::read_text(&omsi_cfg::resolve_path(&root, &rel))
            .unwrap()
            .replace("\r\n", "\n");
        assert_compatible(&old.sli(left_hand), &output, Some(&spec.bay_tex_name()));
        let read =
            RoadSpec::of_with_traffic(&omsi_cfg::resolve_path(&root, &rel), !left_hand).unwrap();
        assert_eq!(read.left_hand, Some(left_hand));
        assert_eq!(read.bay_paths, [4]);
    }
    std::fs::remove_dir_all(root).unwrap();
}
