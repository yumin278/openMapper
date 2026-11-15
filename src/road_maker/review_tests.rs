use super::*;

fn fixture(tag: &str) -> (PathBuf, RoadSpec) {
    let root = std::env::temp_dir().join(format!("om-review-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let tex = root.join("asphalt.png");
    image::RgbaImage::from_pixel(4, 4, image::Rgba([80, 80, 80, 255]))
        .save(&tex)
        .unwrap();
    (
        root,
        RoadSpec {
            name: "Review road".into(),
            roadway_tex: tex,
            ..Default::default()
        }
        .normalised(false),
    )
}

#[test]
fn saving_preserves_custom_bay_pixels_and_sidecars_until_explicit_reset() {
    let (root, mut road) = fixture("pixels");
    road.strips.insert(3, Strip::BusBay { width: 3.0 });
    std::fs::write(root.join("asphalt.png.cfg"), b"original cfg").unwrap();
    std::fs::write(root.join("asphalt.surf"), b"original surf").unwrap();
    road.write(&root).unwrap();
    let folder = root.join("Splines/openMapper/texture");
    let bay = folder.join("Review_road_bay.png");
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 127, 255]))
        .save(&bay)
        .unwrap();
    std::fs::write(folder.join("Review_road_bay.png.cfg"), b"custom cfg").unwrap();
    std::fs::write(folder.join("Review_road_bay.surf"), b"custom surf").unwrap();
    road.write(&root).unwrap();
    assert_eq!(
        image::open(&bay).unwrap().to_rgba8().get_pixel(0, 0).0,
        [255, 0, 127, 255]
    );
    assert_eq!(
        std::fs::read(folder.join("Review_road_bay.png.cfg")).unwrap(),
        b"custom cfg"
    );
    assert_eq!(
        std::fs::read(folder.join("Review_road_bay.surf")).unwrap(),
        b"custom surf"
    );
    road.reset_bay_texture(&root).unwrap();
    assert_eq!(
        image::open(&bay).unwrap().to_rgba8().get_pixel(0, 0).0,
        [80, 80, 80, 255]
    );
    assert_eq!(
        std::fs::read(folder.join("Review_road_bay.png.cfg")).unwrap(),
        b"original cfg"
    );
    assert_eq!(
        std::fs::read(folder.join("Review_road_bay.surf")).unwrap(),
        b"original surf"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bay_edge_markings_follow_the_actual_side() {
    for (left, right) in [(true, false), (false, true), (true, true), (false, false)] {
        let mut road = RoadSpec::default().normalised(false);
        if right {
            road.strips.insert(3, Strip::BusBay { width: 3.0 });
        }
        if left {
            road.strips.insert(1, Strip::BusBay { width: 3.0 });
        }
        let marks = road.standard_markings();
        assert_eq!(
            marks[0].style,
            if left {
                MarkStyle::Dashed
            } else {
                MarkStyle::Solid
            }
        );
        assert_eq!(
            marks[1].style,
            if right {
                MarkStyle::Dashed
            } else {
                MarkStyle::Solid
            }
        );
    }
}

#[test]
fn blank_footway_textures_inherit_the_default_or_omit_visual_profiles() {
    let (root, mut road) = fixture("footways");
    let side = root.join("side.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([150, 150, 150, 255]))
        .save(&side)
        .unwrap();
    // Exercise both the compatible writer and the general strip writer.
    for advanced in [false, true] {
        if let Strip::Lane { bus_only, .. } = &mut road.strips[2] {
            *bus_only = advanced;
        }
        for with_texture in [false, true] {
            road.footway_tex = if with_texture {
                side.clone()
            } else {
                PathBuf::new()
            };
            let rel = road.write(&root).unwrap();
            let model = omsi_scenery::Spline::load(&omsi_cfg::resolve_path(&root, &rel)).unwrap();
            assert_eq!(model.paths.iter().filter(|p| p.kind == 1).count(), 2);
            assert!(road.sli(false).matches("[heightprofile]").count() >= 3);
            assert_eq!(
                model.profiles.len(),
                match (advanced, with_texture) {
                    (false, false) => 1,
                    (false, true) => 5,
                    (true, false) => 2,
                    (true, true) => 6,
                }
            );
            if !with_texture {
                assert!(
                    model
                        .profiles
                        .iter()
                        .all(|p| p.points.iter().all(|q| (q.z - 0.10).abs() < 1e-4))
                );
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn metadata_prefers_explicit_content_root_over_map_location_and_falls_back() {
    let (root, mut road) = fixture("root");
    let preferred = root.join("preferred");
    let fallback = root.join("fallback");
    if let Strip::Lane { bus_only, .. } = &mut road.strips[1] {
        *bus_only = true;
    }
    let rel = road.write(&preferred).unwrap();
    if let Strip::Lane { bus_only, .. } = &mut road.strips[1] {
        *bus_only = false;
    }
    if let Strip::Lane { bus_only, .. } = &mut road.strips[2] {
        *bus_only = true;
    }
    road.write(&fallback).unwrap();
    let mut doc = crate::selection::tests::flat_map(1);
    doc.dir = fallback.join("maps/Test");
    assert_eq!(bus_rules::paths_for(&doc, &rel, Some(&preferred)), [1]);
    doc.content_root = Some(preferred.clone());
    assert_eq!(bus_rules::paths_for(&doc, &rel, None), [1]);
    assert_eq!(
        bus_rules::paths_for(&doc, &rel, Some(&root.join("missing"))),
        [2]
    );
    std::fs::remove_dir_all(root).unwrap();
}
