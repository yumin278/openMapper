#[test]
fn drawn_junctions_place_rules_on_the_baked_bus_routes() {
    use crate::cjunction::{LSeg, Layout, Piece};
    use crate::road_maker::{Direction, RoadSpec, Strip};
    let (mut assets, dir, plain, _) = pocket_fixture("drawn-bus");
    let content = dir.join("content");
    let lane = |dir, bus_only| Strip::Lane { dir, width: 3.5, bus_only, path_offset: 0.0 };
    let road = RoadSpec {
        name: "Bus".into(), left_hand: Some(false),
        roadway_tex: dir.join("asph.png"), footway_tex: dir.join("side.png"),
        strips: vec![lane(Direction::Back, true), lane(Direction::Back, false), lane(Direction::Fwd, false), lane(Direction::Fwd, true)],
        ..Default::default()
    };
    let bus = road.write(&content).unwrap();
    assert!(assets.spline_wait(&bus, 5000).is_some());
    let line = |file: &str, x, y, heading| Piece {
        file: file.into(), segs: vec![LSeg { x, y, heading, radius: 0.0, length: 80.0 }],
        lift: 0.0, hidden: false, auto: false,
    };
    let mut layout = Layout { name: "Drawn bus".into(), auto: true,
        pieces: vec![line(&bus, 0.0, -40.0, 0.0), line(&plain, -40.0, 0.0, 90.0)], ..Default::default() };
    let kinds = crate::cjunction::road_kinds(&layout, &mut assets);
    let is_road = |f: &str| kinds.get(&f.to_lowercase()).map(|k| k.0).unwrap_or(false);
    let half = |f: &str| kinds.get(&f.to_lowercase()).map(|k| k.1).unwrap_or(1.5);
    layout.sync_arms(&is_road, &half);
    let resolved = layout.resolve(&is_road, &half);
    assert_eq!(resolved.plates.len(), 1);
    let plate = crate::junction::layout_plate(&mut assets, &resolved.plates[0], None, None).unwrap();
    assert!(!plate.bus_paths.is_empty());
    // Independently locate the road's declared bus paths at each plate arm and
    // follow the emitted segments from one bus endpoint to another.
    let bus_indices = crate::road_maker::bus_rules::parts_at(&assets.resolve(&bus), false).0;
    let model = assets.spline_wait(&bus, 5000).unwrap();
    let mut bus_ends = Vec::new();
    for arm in resolved.plates[0].iter().filter(|a| a.file == bus) {
        let along = if arm.at_end { crate::roads::norm_deg(arm.out + 180.0) } else { arm.out };
        for &index in &bus_indices {
            bus_ends.push(arm.pos + crate::roads::right(along) * model.def.paths[index].start[0] as f64);
        }
    }
    assert!(bus_ends.len() >= 2);
    let at_bus_end = |p: DVec2| bus_ends.iter().any(|q| p.distance(*q) < 0.02);
    let mut inside = false;
    for &i in &plate.bus_paths {
        let seg = &plate.lanes[i].0;
        if !inside { assert!(at_bus_end(seg.start), "route must start at a declared bus lane"); }
        inside = !at_bus_end(seg.end());
    }
    assert!(!inside, "last bus route must reach a declared bus lane");
    let check = |object: &omsi_map::MapObject, offset: usize| {
        let abs = omsi_cfg::resolve_path(&content, &object.file);
        let sco = omsi_scenery::SceneryObject::load(&abs).unwrap();
        // (the plate's bus-only rules are kept beside its `.sco`, not in the object's rules)
        assert!(object.rules.is_empty(), "{}: a junction object keeps no rules of its own", object.file);
        let template = crate::crossing_config::CrossingConfig::load(&abs, sco.paths.len()).map(|c| c.rules).unwrap_or_default();
        let actual: Vec<_> = template.iter().map(|r| (r.path_index as usize, r.kind.clone())).collect();
        let expected: Vec<_> = plate.bus_paths.iter().flat_map(|i| [(i + offset, "bus".to_string()), (i + offset, "no_cars".to_string())]).collect();
        assert_eq!(actual, expected, "{}", object.file);
        for (i, (seg, kind, direction)) in plate.lanes.iter().enumerate() {
            let p = &sco.paths[offset + i];
            assert_eq!((p.kind, p.direction), (*kind, *direction));
            assert!((p.start[0] as f64 - seg.start.x).abs() < 0.002);
            assert!((p.start[1] as f64 - seg.start.y).abs() < 0.002);
            assert_eq!(template.iter().any(|r| r.path_index as usize == offset + i), plate.bus_paths.contains(&i));
        }
    };
    let mut doc = flat_map(1);
    doc.dir = content.join("maps").join("Drawn");
    let mut ed = Editor::default();
    ed.content_dir = Some(content.clone());
    ed.open(doc);
    ed.place_layout(&mut assets, &layout, DVec2::new(150.0, 150.0), 0.0);
    let tile = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
    let object = tile.objects.iter().find(|o| o.file.contains("junction_")).expect("plate placed");
    check(object, 0);

    let roots = crate::install::Roots { omsi: dir.join("omsi"), content: content.clone() };
    // (the plate a drawing puts on the map is in the catalogue at once, without a rescan)
    let mut lib = crate::library::Library::default();
    assert!(lib.refresh_pending(&roots) > 0, "the placed plate is queued for the catalogue");
    assert!(lib.find(&object.file).is_some(), "the plate placed on the map is in the catalogue");
    let baked = crate::cjunction::bake(&roots, &layout, &mut assets, true).unwrap();
    layout.baked = Some(baked.clone());
    let mut doc = flat_map(1);
    doc.dir = content.join("maps").join("Custom");
    let mut ends = Vec::new();
    for (i, arm) in layout.arms.iter().enumerate() {
        let pos = DVec2::new(150.0 + arm.x, 150.0 + arm.y) + crate::roads::dir(arm.out) * 80.0;
        let id = i as i64 + 1;
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.push(MapSpline {
            file: arm.road.clone(), id, pos: [pos.x, pos.y, 0.0], heading: crate::roads::norm_deg(arm.out + 180.0), length: 70.0, ..Default::default()
        });
        ends.push(Some(((0, 0), id, true)));
    }
    crate::ed_cjunction::apply_custom(&mut doc, &mut assets, &[layout], &ends, DVec3::new(150.0, 150.0, 0.0), false).expect("custom junction placed");
    let object = doc.tiles[&(0, 0)].tile.objects.iter().find(|o| o.file == baked).unwrap();
    let baked_sco = omsi_scenery::SceneryObject::load(&omsi_cfg::resolve_path(&content, &baked)).unwrap();
    let offset = baked_sco.paths.len() - plate.lanes.len();
    assert!(offset > 0, "piece paths precede plate paths");
    check(object, offset);
    std::fs::remove_dir_all(dir).unwrap();
}
