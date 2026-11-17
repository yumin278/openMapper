// Taper tool (complex junction editor): the plate between a road's end and another road type,
// built with real road types written by the road maker. Included in `editor::tests`.

/// Road types for the taper tests, written into the fixture's content folder: two lanes, three
/// lanes, and another two-way road as wide as the first with its lanes set out otherwise.
fn taper_fixture(tag: &str) -> (Assets, std::path::PathBuf, [String; 4]) {
    use crate::road_maker::{Direction, RoadSpec, Strip};
    let (mut assets, dir, _, _) = pocket_fixture(tag);
    let content = dir.join("content");
    let lane = |dir, width| Strip::Lane { dir, width, bus_only: false };
    let spec = |name: &str, strips: Vec<Strip>| RoadSpec {
        name: name.into(), left_hand: Some(false),
        roadway_tex: dir.join("asph.png"), footway_tex: dir.join("side.png"),
        strips, ..Default::default()
    };
    let two = spec("Taper two", vec![lane(Direction::Back, 3.5), lane(Direction::Fwd, 3.5)]).write(&content).unwrap();
    let three = spec("Taper three", vec![lane(Direction::Back, 3.5), lane(Direction::Fwd, 3.5), lane(Direction::Fwd, 3.5)]).write(&content).unwrap();
    let same = spec("Taper same width", vec![lane(Direction::Back, 2.0), lane(Direction::Fwd, 2.5), lane(Direction::Fwd, 2.5)]).write(&content).unwrap();
    let one_way = spec("Taper one way", vec![lane(Direction::Fwd, 3.5), lane(Direction::Fwd, 3.5)]).write(&content).unwrap();
    for rel in [&two, &three, &same, &one_way] {
        assert!(assets.spline_wait(rel, 5000).is_some(), "{rel} loads");
    }
    (assets, dir, [two, three, same, one_way])
}

/// A taper from the end of a road running north to (0, 0): P at the origin, heading north.
fn north_taper(from: &str, to: &str, length: f64, offset: f64) -> crate::cjunction::Taper {
    crate::cjunction::Taper { x: 0.0, y: 0.0, out: 0.0, from_file: from.into(), from_at_end: true, to_file: to.into(), length, offset }
}

/// Where the vehicle lanes of a plate arm's road end at the arm, with each lane's x in its `.sli`.
fn vehicle_lane_ends(assets: &mut Assets, arm: &crate::junction::PlateArm) -> Vec<(DVec2, f64)> {
    let model = assets.spline_wait(&arm.file, 5000).unwrap();
    let along = if arm.at_end { crate::roads::norm_deg(arm.out + 180.0) } else { arm.out };
    model.def.paths.iter().filter(|p| p.kind == 0).map(|p| {
        let x = p.start[0] as f64;
        (arm.pos + crate::roads::right(along) * x, x)
    }).collect()
}

fn print_pairs(what: &str, plate: &crate::junction::Plate) {
    println!("{what}: {} paths", plate.lanes.len());
    for (seg, kind, dir) in &plate.lanes {
        println!("  kind {kind} dir {dir}: ({:.3}, {:.3}) -> ({:.3}, {:.3}), heading {:.1}°, radius {:.2}, length {:.2}", seg.start.x, seg.start.y, seg.end().x, seg.end().y, seg.heading, seg.radius, seg.length);
    }
}

/// Every vehicle lane of both arms has a path of the plate ending on it.
fn assert_every_lane_joined(assets: &mut Assets, t: &crate::cjunction::Taper, plate: &crate::junction::Plate) {
    for arm in t.plate_arms() {
        for (e, x) in vehicle_lane_ends(assets, &arm) {
            let joined = plate.lanes.iter().filter(|l| l.1 == 0).any(|(s, _, _)| (s.start - e).length() < 0.05 || (s.end() - e).length() < 0.05);
            assert!(joined, "{}: the lane at x = {x} ({e:?}) has no path", arm.file);
        }
    }
}

#[test]
fn a_taper_between_the_same_road_type_is_laid_with_its_roadway_and_lanes() {
    let (mut assets, dir, [two, ..]) = taper_fixture("taper-same");
    let t = north_taper(&two, &two, 20.0, 0.0);
    let plate = crate::junction::layout_plate(&mut assets, &t.plate_arms(), None).expect("a plate");
    print_pairs("two -> two", &plate);
    assert!(!plate.flat_textured().is_empty(), "the roadway has a mesh");
    assert!(plate.lanes.iter().any(|l| l.1 == 0), "it has AI paths");
    assert_every_lane_joined(&mut assets, &t, &plate);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_taper_between_roads_as_wide_but_with_other_lanes_is_always_laid() {
    let (mut assets, dir, [two, _, same, one_way]) = taper_fixture("taper-width");
    let (a, b, c) = (assets.spline_wait(&two, 5000).unwrap(), assets.spline_wait(&same, 5000).unwrap(), assets.spline_wait(&one_way, 5000).unwrap());
    assert!((a.half_width - b.half_width).abs() < 1e-4 && (a.half_width - c.half_width).abs() < 1e-4, "as wide: {} {} {}", a.half_width, b.half_width, c.half_width);
    for to in [&same, &one_way] {
        let t = north_taper(&two, to, 15.0, 0.0);
        let plate = crate::junction::layout_plate(&mut assets, &t.plate_arms(), None).unwrap_or_else(|| panic!("a plate to {to}"));
        print_pairs(&format!("two -> {to}"), &plate);
        assert!(!plate.flat_textured().is_empty());
        assert!(plate.lanes.iter().any(|l| l.1 == 0));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_taper_to_three_lanes_shifted_right_ends_its_lanes_at_q_along_the_way() {
    let (mut assets, dir, [two, three, ..]) = taper_fixture("taper-shift");
    let t = north_taper(&two, &three, 20.0, 1.75);
    let (q, h) = t.end();
    assert!((q - DVec2::new(1.75, 20.0)).length() < 1e-9);
    let plate = crate::junction::layout_plate(&mut assets, &t.plate_arms(), None).expect("a plate");
    print_pairs("two -> three, X = +1.75", &plate);
    let target = t.plate_arms()[1].clone();
    let half = assets.spline_wait(&three, 5000).unwrap().half_width as f64;
    let ends = vehicle_lane_ends(&mut assets, &target);
    assert_eq!(ends.len(), 3);
    let diff = |a: f64, b: f64| ((a - b).rem_euclid(180.0)).min(180.0 - (a - b).rem_euclid(180.0));
    for (e, x) in ends {
        assert!((e - q).length() <= half + 0.01, "lane x = {x} at {e:?} is by Q");
        // (the paths meet the lane along the road: heading h or against it)
        let mut met = 0;
        for (s, kind, _) in plate.lanes.iter().filter(|l| l.1 == 0) {
            let at = if (s.start - e).length() < 0.05 { Some(s.heading) } else if (s.end() - e).length() < 0.05 { Some(s.end_heading()) } else { None };
            if let Some(a) = at {
                met += 1;
                assert!(diff(a, h) < 5.0, "kind {kind}: heading {a} at lane x = {x}, the way is {h}");
            }
        }
        println!("lane x = {x}: {met} paths");
        assert!(met > 0, "lane x = {x} joined");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_taper_between_two_and_three_lanes_joins_every_lane() {
    let (mut assets, dir, [two, three, ..]) = taper_fixture("taper-fill");
    for (from, to, x) in [(&two, &three, 1.75), (&three, &two, -1.75)] {
        let t = north_taper(from, to, 25.0, x);
        let plate = crate::junction::layout_plate(&mut assets, &t.plate_arms(), None).expect("a plate");
        print_pairs(&format!("{from} -> {to}, X = {x}"), &plate);
        assert_every_lane_joined(&mut assets, &t, &plate);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_taper_is_in_the_baked_object_and_on_the_map_without_build_plate() {
    let (mut assets, dir, [two, three, ..]) = taper_fixture("taper-consumers");
    let content = dir.join("content");
    let road = crate::cjunction::Piece { file: two.clone(), segs: vec![crate::cjunction::LSeg { x: 0.0, y: -30.0, heading: 0.0, radius: 0.0, length: 30.0 }], lift: 0.0, hidden: false, auto: false };
    let mut layout = crate::cjunction::Layout { name: "Taper bake".into(), pieces: vec![road.clone()], ..Default::default() };
    layout.build.plate = false;
    let t = north_taper(&two, &three, 20.0, 1.75);
    layout.tapers.push(t.clone());
    let plate = crate::junction::layout_plate(&mut assets, &t.plate_arms(), None).expect("a plate");
    let n = plate.lanes.len();
    // the editor's preview: what `all_plates` gives is what it draws
    let kinds = crate::cjunction::road_kinds(&layout, &mut assets);
    let is_road = |f: &str| kinds.get(&f.to_lowercase()).map(|k| k.0).unwrap_or(false);
    let half = |f: &str| kinds.get(&f.to_lowercase()).map(|k| k.1).unwrap_or(1.5);
    let resolved = layout.resolve(&is_road, &half);
    assert!(resolved.plates.is_empty());
    assert_eq!(layout.all_plates(&resolved), vec![t.plate_arms().to_vec()]);
    // saved as an object: the taper's paths come on top of the road's
    let roots = crate::install::Roots { omsi: dir.join("omsi"), content: content.clone() };
    let with = crate::cjunction::bake(&roots, &layout, &mut assets, false).unwrap();
    let mut without = layout.clone();
    without.name = "Taper bake none".into();
    without.tapers.clear();
    let bare = crate::cjunction::bake(&roots, &without, &mut assets, false).unwrap();
    let paths = |rel: &str| omsi_scenery::SceneryObject::load(&crate::editable::sco_path(&roots, rel).unwrap()).unwrap().paths.len();
    assert_eq!(paths(&with), paths(&bare) + n, "the taper's {n} paths are baked");
    // put on the map: the taper's plate is an object of its own
    let mut doc = flat_map(1);
    doc.dir = content.join("maps").join("Taper");
    let mut ed = Editor::default();
    ed.content_dir = Some(content.clone());
    ed.open(doc);
    ed.place_layout(&mut assets, &layout, DVec2::new(150.0, 150.0), 0.0);
    let tile = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
    let objects: Vec<_> = tile.objects.iter().filter(|o| o.file.contains("junction_")).collect();
    assert_eq!(objects.len(), 1, "the taper's plate is put");
    let sco = omsi_scenery::SceneryObject::load(&omsi_cfg::resolve_path(&content, &objects[0].file)).unwrap();
    assert_eq!(sco.paths.len(), n);
    let _ = std::fs::remove_dir_all(&dir);
}
