//! Tests for the advanced strip settings: a footway's AI-path switch and the median and
//! footway heights. They are written here (not inside `pocket.rs` / `junction.rs`) so those
//! files stay untouched; the bus-bay and junction behaviour is exercised through their public
//! entry points.

use super::*;
use std::path::{Path, PathBuf};

fn tex(dir: &Path, name: &str, gray: u8) -> PathBuf {
    let p = dir.join(name);
    image::RgbaImage::from_pixel(4, 4, image::Rgba([gray, gray, gray, 255]))
        .save(&p)
        .unwrap();
    p
}

/// Every `[path]` of a road type's text: (kind, x, height, width, direction).
fn road_paths(text: &str) -> Vec<(f32, f32, f32, f32, f32)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(l) = lines.next() {
        if l.trim() == "[path]" {
            let mut v = [0.0f32; 5];
            for slot in &mut v {
                *slot = lines.next().unwrap().trim().parse().unwrap();
            }
            out.push((v[0], v[1], v[2], v[3], v[4]));
        }
    }
    out
}

/// The x of every vertical kerb face: a `[profile]` whose two points share an x but differ in
/// height. The free-layout writer emits exactly two `[profilepnt]` per `[profile]`.
fn faces(text: &str) -> Vec<f32> {
    let mut points = Vec::new();
    let mut lines = text.lines();
    while let Some(l) = lines.next() {
        if l.trim() == "[profilepnt]" {
            let x: f32 = lines.next().unwrap().trim().parse().unwrap();
            let h: f32 = lines.next().unwrap().trim().parse().unwrap();
            let _u: f32 = lines.next().unwrap().trim().parse().unwrap();
            let _v: f32 = lines.next().unwrap().trim().parse().unwrap();
            points.push((x, h));
        }
    }
    points
        .chunks_exact(2)
        .filter_map(|p| {
            let (x0, h0) = p[0];
            let (x1, h1) = p[1];
            ((x0 - x1).abs() < 1e-6 && (h0 - h1).abs() > 1e-6).then_some(x0)
        })
        .collect()
}

fn close(x: f32, y: f32) -> bool {
    (x - y).abs() < 1e-3
}

fn contains(paths: &[Vec<f32>], p: &[f32]) -> bool {
    paths.iter().any(|q| q.len() == p.len() && q.iter().zip(p).all(|(a, b)| (*a - *b).abs() < 1e-4))
}

fn same_set(a: &[Vec<f32>], b: &[Vec<f32>]) -> bool {
    a.len() == b.len() && a.iter().all(|p| contains(b, p)) && b.iter().all(|p| contains(a, p))
}

/// A footway, a median and lanes in the order the editor allows (footways at the ends, the
/// median inside the roadway).
fn raised_spec(name: &str, left_path: bool, median_height: Option<f32>) -> RoadSpec {
    RoadSpec {
        name: name.into(),
        left_hand: Some(false),
        strips: vec![
            Strip::Footway {
                width: 2.0,
                tex: PathBuf::new(),
                path: left_path,
                height: Some(0.30),
            },
            Strip::Lane {
                dir: Direction::Back,
                width: 3.0,
                bus_only: false,
            },
            Strip::Median {
                width: 2.0,
                raised: true,
                tex: PathBuf::new(),
                height: median_height,
            },
            Strip::Lane {
                dir: Direction::Fwd,
                width: 3.0,
                bus_only: false,
            },
            Strip::Footway {
                width: 2.0,
                tex: PathBuf::new(),
                path: true,
                height: Some(0.30),
            },
        ],
        ..Default::default()
    }
}

#[test]
fn footway_without_an_ai_path_omits_its_path_and_renumbers_bus_and_bay_paths() {
    let dir = std::env::temp_dir().join(format!("om-advanced-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let asphalt = tex(&dir, "asph.png", 80);
    let side = tex(&dir, "side.png", 150);
    let foot = |p: bool| Strip::Footway {
        width: 2.0,
        tex: PathBuf::new(),
        path: p,
        height: None,
    };
    let spec = |name: &str, left_path: bool| RoadSpec {
        name: name.into(),
        left_hand: Some(false),
        strips: vec![
            foot(left_path),
            Strip::BusBay { width: 3.0 },
            Strip::Lane {
                dir: Direction::Back,
                width: 3.0,
                bus_only: true,
            },
            Strip::Median {
                width: 2.0,
                raised: true,
                tex: PathBuf::new(),
                height: None,
            },
            Strip::Lane {
                dir: Direction::Fwd,
                width: 4.0,
                bus_only: false,
            },
            Strip::Lane {
                dir: Direction::Fwd,
                width: 3.5,
                bus_only: true,
            },
            Strip::BusBay { width: 2.5 },
            foot(true),
        ],
        roadway_tex: asphalt.clone(),
        footway_tex: side.clone(),
        ..Default::default()
    };
    let with = spec("With", true);
    let without = spec("Without", false);

    // The path numbering skips the footway whose AI path is off, so every bus/bay index is one
    // less than it is with the footway on.
    let with_bus = with.bus_path_indices();
    let with_bay = with.bay_path_indices();
    let without_bus = without.bus_path_indices();
    let without_bay = without.bay_path_indices();
    assert_eq!(with_bus, vec![2, 4]);
    assert_eq!(with_bay, vec![1, 5]);
    assert_eq!(
        without_bus,
        with_bus.iter().map(|i| i - 1).collect::<Vec<_>>()
    );
    assert_eq!(
        without_bay,
        with_bay.iter().map(|i| i - 1).collect::<Vec<_>>()
    );

    let content = dir.join("content");
    with.write(&content).unwrap();
    without.write(&content).unwrap();
    for (spec, bus, bay, footways) in [
        (&with, &with_bus, &with_bay, 2usize),
        (&without, &without_bus, &without_bay, 1usize),
    ] {
        let path = content
            .join("Splines/openMapper")
            .join(format!("{}.sli", spec.file_stem()));
        let spline = omsi_scenery::Spline::load(&path).unwrap();
        // The indices point at the paths actually written: bus and bay lanes are street paths.
        for &i in bus {
            assert_eq!(spline.paths[i].kind, 0, "{name} bus {i}", name = spec.name);
        }
        for &i in bay {
            assert_eq!(spline.paths[i].kind, 0, "{name} bay {i}", name = spec.name);
        }
        // Only the footways that keep their AI path write a walk path.
        assert_eq!(
            spline.paths.iter().filter(|p| p.kind == 1).count(),
            footways,
            "{name}",
            name = spec.name
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn specified_heights_reach_the_height_profiles_profiles_paths_and_kerb_faces() {
    let dir = std::env::temp_dir().join(format!("om-advanced-height-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut spec = raised_spec("Raised", true, Some(0.25));
    spec.roadway_tex = tex(&dir, "asph.png", 80);
    spec.footway_tex = tex(&dir, "side.png", 150);
    let text = spec.sli(false);

    // Positions: footway -6..-4, lane -4..-1, median -1..1, lane 1..4, footway 4..6.
    assert!(text.contains("[heightprofile]\n-6.000\n-4.000\n0.300\n0.300"));
    assert!(text.contains("[heightprofile]\n-1.000\n1.000\n0.250\n0.250"));
    assert!(text.contains("[heightprofile]\n1.000\n4.000\n0.100\n0.100"));
    // Footway paths carry their height; lane paths sit on the roadway.
    assert!(text.contains("[path]\n1\n-5.000\n0.300\n0.800\n2"));
    assert!(text.contains("[path]\n0\n-2.500\n0.100\n2.700\n1"));
    assert!(text.contains("[path]\n1\n5.000\n0.300\n0.800\n2"));

    // Kerb faces at the footway inner edges (0.10 -> 0.30) and both median edges (0.10 -> 0.25).
    let fs = faces(&text);
    for x in [-4.0, -1.0, 1.0, 4.0] {
        assert!(
            fs.iter().any(|&f| close(f, x)),
            "no kerb face at {x}: {fs:?}\n{text}"
        );
    }
    assert_eq!(fs.len(), 4, "faces {fs:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_median_level_with_the_roadway_has_no_kerb_faces() {
    let dir = std::env::temp_dir().join(format!("om-advanced-level-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut spec = raised_spec("Level", true, Some(0.10));
    spec.roadway_tex = tex(&dir, "asph.png", 80);
    spec.footway_tex = tex(&dir, "side.png", 150);
    let text = spec.sli(false);
    assert!(text.contains("[heightprofile]\n-1.000\n1.000\n0.100\n0.100"));
    // Only the footways' inner edges rise (0.10 -> 0.30); the median's do not.
    let fs = faces(&text);
    assert_eq!(fs.len(), 2, "faces {fs:?}\n{text}");
    for x in [-4.0, 4.0] {
        assert!(fs.iter().any(|&f| close(f, x)), "missing footway face at {x}");
    }
    assert!(!fs.iter().any(|&f| close(f, -1.0) || close(f, 1.0)));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_strip_below_the_roadway_is_refused_and_a_level_footway_warns() {
    let dir = std::env::temp_dir().join(format!("om-advanced-low-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let asphalt = tex(&dir, "asph.png", 80);
    let side = tex(&dir, "side.png", 150);
    let foot = |h: f32| Strip::Footway {
        width: 2.0,
        tex: PathBuf::new(),
        path: true,
        height: Some(h),
    };
    let lanes = vec![
        Strip::Lane {
            dir: Direction::Back,
            width: 3.0,
            bus_only: false,
        },
        Strip::Lane {
            dir: Direction::Fwd,
            width: 3.0,
            bus_only: false,
        },
    ];

    let low = RoadSpec {
        name: "Low".into(),
        left_hand: Some(false),
        strips: vec![foot(0.05), lanes[0].clone(), lanes[1].clone(), foot(0.30)],
        roadway_tex: asphalt.clone(),
        footway_tex: side.clone(),
        ..Default::default()
    };
    let expected = crate::i18n::tr(
        "The median and footway must not be lower than the roadway",
        "Разделительная полоса и тротуар не могут быть ниже проезжей части",
    );
    assert_eq!(low.validate().unwrap_err(), expected);
    assert!(low.sli(false).is_empty());
    let content = dir.join("content");
    assert!(low.write(&content).is_err());
    assert!(!content.join("Splines").exists());

    let level = RoadSpec {
        name: "Level".into(),
        left_hand: Some(false),
        strips: vec![foot(0.10), lanes[0].clone(), lanes[1].clone(), foot(0.30)],
        roadway_tex: asphalt,
        footway_tex: side,
        ..Default::default()
    };
    assert!(level.validate().is_ok());
    assert!(level.warnings().iter().any(|w| w.as_str() == crate::i18n::tr(
        "This footway is level with the roadway: the bus bay tool opens the bay outside it",
        "Этот тротуар на одном уровне с проезжей частью: инструмент автобусного кармана сделает карман снаружи него",
    )));
    assert!(!level.sli(false).is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bus_bays_open_between_the_roadway_and_a_raised_disabled_path_footway() {
    let dir = std::env::temp_dir().join(format!("om-advanced-bay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let content = dir.join("content");
    let mut spec = raised_spec("Raised", false, Some(0.25));
    spec.roadway_tex = tex(&dir, "asph.png", 80);
    spec.footway_tex = tex(&dir, "side.png", 150);
    let rel = spec.write(&content).unwrap();

    let read = |rel: &str| {
        omsi_cfg::vfs::read_text(&omsi_cfg::resolve_path(&content, rel))
            .unwrap()
            .replace("\r\n", "\n")
    };
    let original = read(&rel);
    let original_streets: Vec<_> = road_paths(&original)
        .into_iter()
        .filter(|p| p.0 == 0.0)
        .map(|p| (p.1, p.2, p.3, p.4))
        .collect();
    // two through lanes, and the disabled left footway wrote no walk path
    assert_eq!(original_streets.len(), 2);
    assert_eq!(
        road_paths(&original).into_iter().filter(|p| p.0 == 1.0).count(),
        1
    );

    let mut assets = crate::assets::Assets::new(dir.join("omsi"), content.clone());
    let bay = crate::pocket::bay_type(&mut assets, &content, &rel, false, false, 3.0, false)
        .unwrap();
    let text = read(&bay);
    let streets: Vec<_> = road_paths(&text)
        .into_iter()
        .filter(|p| p.0 == 0.0)
        .map(|p| (p.1, p.2, p.3, p.4))
        .collect();
    // The original lane paths are untouched; the bay lane (5.5 m, road height, 0.9 x width,
    // the way of the lane beside it) is added between the roadway (edge at 4 m) and the footway.
    assert_eq!(&streets[..original_streets.len()], original_streets.as_slice());
    assert_eq!(&streets[original_streets.len()..], &[(5.5, 0.100, 2.7, 0.0)]);

    // The bay stands at the roadway's height; the footway (and its height) is pushed out.
    assert!(text.contains("[heightprofile]\n4.000\n7.000\n0.100\n0.100"));
    assert!(text.contains("[heightprofile]\n7.000\n9.000\n0.300\n0.300"));
    // The right footway's walk path moves with it and keeps its 0.30 m height.
    assert!(text.contains("[path]\n1\n8.000\n0.300\n0.800\n2"));

    // The bus-bay tool's own index range names exactly the added lane.
    let main = assets.spline_wait(&rel, 5000).unwrap();
    let bay_model = assets.spline_wait(&bay, 5000).unwrap();
    let range = crate::pocket::bay_path_indices(&main, &bay_model);
    assert_eq!(range, main.def.paths.len()..bay_model.def.paths.len());
    assert_eq!(bay_model.def.paths[range.clone()].len(), 1);
    assert_eq!(bay_model.def.paths[range.start].kind, 0);

    // Both sides: two bay lanes, both footways pushed out, the left one still without a path.
    let both = crate::pocket::bay_type(&mut assets, &content, &rel, false, false, 3.0, true)
        .unwrap();
    let both_text = read(&both);
    let both_streets: Vec<_> = road_paths(&both_text)
        .into_iter()
        .filter(|p| p.0 == 0.0)
        .map(|p| (p.1, p.2, p.3, p.4))
        .collect();
    assert_eq!(both_streets.len(), original_streets.len() + 2);
    assert_eq!(
        road_paths(&both_text).into_iter().filter(|p| p.0 == 1.0).count(),
        1
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn junctions_build_with_raised_strips_and_only_from_footways_that_have_paths() {
    let roots = crate::doc::tests::scratch("junction-advanced");
    let content = roots.content.clone();
    std::fs::create_dir_all(&content).unwrap();
    let asphalt = tex(&content, "asph.png", 80);
    let side = tex(&content, "side.png", 150);
    let write = |name: &str, left_path: bool| {
        let mut spec = raised_spec(name, left_path, Some(0.25));
        spec.roadway_tex = asphalt.clone();
        spec.footway_tex = side.clone();
        spec.write(&content).unwrap()
    };
    // The same road with the left footway present (path on), present but path off, and gone.
    let both = write("Both", true);
    let mixed = write("Mixed", false);
    let mut single = raised_spec("Single", false, Some(0.25));
    single.strips.remove(0); // no left footway strip at all
    single.roadway_tex = asphalt.clone();
    single.footway_tex = side.clone();
    let single = single.write(&content).unwrap();
    let mut assets = crate::assets::Assets::new(roots.omsi.clone(), content.clone());
    for rel in [&both, &mixed, &single] {
        assert!(assets.spline_wait(rel, 5000).is_some());
    }

    // The source roads differ only by the disabled footway's walk path.
    let footways = |rel: &str| {
        let text = omsi_cfg::vfs::read_text(&omsi_cfg::resolve_path(&content, rel))
            .unwrap()
            .replace("\r\n", "\n");
        road_paths(&text).into_iter().filter(|p| p.0 == 1.0).count()
    };
    assert_eq!(footways(&both), 2);
    assert_eq!(footways(&mixed), 1);
    assert_eq!(footways(&single), 1);

    let mut build = |file: &str| -> Vec<Vec<f32>> {
        let mut doc = crate::selection::tests::flat_map(1);
        doc.dir = content.join("maps").join("Advanced");
        let centre = glam::DVec3::new(150.0, 150.0, 0.0);
        let mut arms = Vec::new();
        for (i, heading) in [0.0f64, 90.0, 180.0, 270.0].into_iter().enumerate() {
            let p = centre.truncate() + crate::roads::dir(heading) * 18.0;
            let id = 1000 + i as i64 + 1;
            doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.push(omsi_map::MapSpline {
                file: file.to_string(),
                id,
                pos: [p.x, p.y, 0.0],
                heading,
                length: 50.0,
                ..Default::default()
            });
            arms.push(crate::junction::Arm { key: (0, 0), id, at_end: false });
        }
        let id = crate::junction::make_with(
            &mut doc,
            &mut assets,
            &content,
            None,
            centre,
            &arms,
            &crate::junction::Options::default(),
            crate::junction::MakeEdit::default(),
        )
        .unwrap();
        let obj = doc.tiles[&(0, 0)].tile.objects.iter().find(|o| o.id == id).unwrap();
        let text = omsi_cfg::vfs::read_text(&omsi_cfg::resolve_path(&content, &obj.file))
            .unwrap()
            .replace("\r\n", "\n");
        // Each junction path has 12 numbers; kind is the 9th (index 8).
        let mut walks = Vec::new();
        let mut lines = text.lines();
        while let Some(l) = lines.next() {
            if l.trim() == "[path]" {
                let mut v = Vec::new();
                for _ in 0..12 {
                    v.push(lines.next().unwrap().trim().parse::<f32>().unwrap());
                }
                if v[8] == 1.0 {
                    walks.push(v);
                }
            }
        }
        walks
    };

    let all = build(&both);
    let one = build(&mixed);
    let none = build(&single);
    assert!(!all.is_empty(), "the both-footway junction has corner walks");
    // The footway whose AI path is off contributes no corner walk: the junction is exactly
    // the same as if that footway strip were absent.
    assert!(same_set(&one, &none), "mixed {:?} vs single {:?}", one, none);
    // The footway does change the junction when its path is on.
    assert!(!same_set(&all, &one), "both and mixed should differ");
    std::fs::remove_dir_all(&content).unwrap();
}

#[test]
fn old_road_json_without_the_new_fields_saves_back_unchanged() {
    let dir = std::env::temp_dir().join(format!("om-advanced-old-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let spec = RoadSpec {
        name: "Old".into(),
        lanes_fwd: 2,
        lanes_back: 2,
        lane_width: 3.25,
        footway: 2.0,
        roadway_tex: tex(&dir, "asph.png", 80),
        footway_tex: tex(&dir, "side.png", 150),
        ..Default::default()
    };
    let content = dir.join("content");
    spec.write(&content).unwrap();
    let sli_path = content.join("Splines/openMapper/Old.sli");
    let json_path = sli_path.with_extension("road.json");
    let sli_before = std::fs::read_to_string(&sli_path).unwrap();
    let json_before = std::fs::read(&json_path).unwrap();

    // The freshly written road.json has none of the new per-strip keys.
    let value: serde_json::Value = serde_json::from_slice(&json_before).unwrap();
    let strips = value["strips"].as_array().unwrap();
    assert!(!strips.is_empty());
    for strip in strips {
        for (_, inner) in strip.as_object().unwrap() {
            let obj = inner.as_object().unwrap();
            assert!(!obj.contains_key("path"));
            assert!(!obj.contains_key("height"));
        }
    }

    // Opening and saving it again changes nothing, text or metadata.
    let back = RoadSpec::of(&sli_path).unwrap();
    back.write_with_traffic(&content, false).unwrap();
    assert_eq!(std::fs::read_to_string(&sli_path).unwrap(), sli_before);
    assert_eq!(std::fs::read(&json_path).unwrap(), json_before);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The x of every `[path]`, exactly as the `.sli` text has it.
fn path_x_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "[path]" {
            let _kind = lines.next();
            if let Some(x) = lines.next() {
                out.push(x.trim().to_string());
            }
            for _ in 0..3 {
                let _ = lines.next();
            }
        }
    }
    out
}

/// Both writers take every path's x from `RoadSpec::path_x`, in its three decimals: negative,
/// positive, and the zero of a one-way road centred on the spline.
#[test]
fn road_maker_writers_take_every_path_x_from_the_shared_calculation() {
    let simple = |name: &str, lanes_fwd: u8, lanes_back: u8, footway: f32| {
        RoadSpec {
            name: name.into(),
            lanes_fwd,
            lanes_back,
            lane_width: 3.5,
            footway,
            footway_right: Some(footway),
            ..Default::default()
        }
        .normalised(false)
    };
    let centered = simple("Centred", 1, 0, 0.0);
    let two_way = simple("Two way", 1, 1, 2.0);
    // A fixed height keeps this one out of the simple controls: it goes to the free layout.
    let free = raised_spec("Free layout", true, Some(0.25)).normalised(false);
    // Each goes through the writer its layout calls for.
    assert_eq!(centered.sli(false), centered.projection().compatible_sli(false));
    assert_eq!(two_way.sli(false), two_way.projection().compatible_sli(false));
    assert_eq!(free.sli(false), super::free_layout::emit(&free));

    for spec in [&centered, &two_way, &free] {
        let expected: Vec<String> = spec
            .strips
            .iter()
            .zip(spec.strip_positions())
            .filter(|(strip, _)| strip.has_ai_path())
            .map(|(_, (lo, hi))| format!("{:.3}", spec.path_x(lo, hi)))
            .collect();
        let written = path_x_lines(&spec.sli(false));
        assert_eq!(written.len(), expected.len(), "{}", spec.name);
        assert_eq!(written, expected, "{}", spec.name);
        println!("{} path x: {written:?}", spec.name);
    }
    // The named values the calculation gives, sign and all.
    assert_eq!(path_x_lines(&centered.sli(false)), vec!["0.000".to_string()]);
    assert!(path_x_lines(&two_way.sli(false)).contains(&"-1.750".to_string()));
    assert!(path_x_lines(&two_way.sli(false)).contains(&"1.750".to_string()));
}
