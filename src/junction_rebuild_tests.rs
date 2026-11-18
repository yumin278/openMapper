fn template_of(content: &std::path::Path, rel: &str) -> Vec<crate::crossing_config::TemplateRule> {
    let abs = omsi_cfg::resolve_path(content, rel);
    let paths = omsi_scenery::SceneryObject::load(&abs).map(|s| s.paths.len()).unwrap_or(0);
    crate::crossing_config::CrossingConfig::load(&abs, paths).map(|c| c.rules).unwrap_or_default()
}

#[test]
fn a_manual_junction_rebuild_keeps_the_bus_bay_taper_without_unresolved_rules() {
    let (mut assets, dir, plain, _) = pocket_fixture("manual-taper-rebuild");
    let content = dir.join("content");
    let key = (0, 0);
    let mut doc = flat_map(1);
    doc.dir = content.join("maps").join("Pocket");
    doc.tiles.get_mut(&key).unwrap().tile.splines.push(MapSpline {
        file: plain.clone(), id: 1, pos: [150.0, 60.0, 0.0], heading: 0.0, length: 180.0,
        ..Default::default()
    });
    let mut ed = Editor::default();
    ed.content_dir = Some(content.clone());
    ed.open(doc);
    ed.set_tool(Tool::Roads);
    ed.bus_pocket = true;
    ed.cursor = Some(DVec3::new(150.0, 150.0, 0.0));
    ed.pocket_click(&mut assets);

    let tile = &ed.doc.as_ref().unwrap().tiles[&key].tile;
    assert_eq!(tile.objects.len(), 2);
    let entry = tile.objects.iter().min_by(|a, b| a.pos[1].total_cmp(&b.pos[1])).unwrap().clone();
    let mut exit = tile.objects.iter().max_by(|a, b| a.pos[1].total_cmp(&b.pos[1])).unwrap().clone();
    let outside = tile.splines.iter().find(|s| !s.deleted && s.pos[1] > exit.pos[1]).unwrap().id;
    let bay = tile.splines.iter().find(|s| !s.deleted && crate::pocket::is_bay_spline(&s.file)).unwrap().clone();
    assert!(is_bus_bay_junction(&exit.file));
    assert!(!template_of(&content, &exit.file).is_empty(), "the bay's rules are its template");

    let road = |name: &str, lanes_fwd, lanes_back, lane_width| crate::road_maker::RoadSpec {
        name: name.into(), lanes_fwd, lanes_back, lane_width,
        roadway_tex: dir.join("asph.png"), footway_tex: dir.join("side.png"),
        ..Default::default()
    }.write(&content).unwrap();
    let wider = road("Wider", 1, 1, 4.0);
    let three_lanes = road("ThreeLanes", 2, 1, 3.75);
    // All outside lanes now approach the exit, so the bay lane has nowhere to go.
    // The outside lanes can still connect to the bay spline's ordinary backward lane.
    let backward = road("Backward", 0, 2, 4.5);
    let bay_model = assets.spline_wait(&bay.file, 5000).unwrap();
    let sco_path = |file: &str| omsi_cfg::resolve_path(&content, file);
    let mesh_text = |file: &str| {
        let sco = sco_path(file);
        std::fs::read_to_string(sco.parent().unwrap().join("model").join(sco.file_name().unwrap()).with_extension("x")).unwrap()
    };
    for (file, new_lanes, restricted) in [(&wider, 1, true), (&three_lanes, 0, false), (&backward, 1, false), (&plain, 1, true)] {
        let model = assets.spline_wait(file, 5000).unwrap();
        let indices = crate::pocket::bay_path_indices(&model, &bay_model);
        assert_eq!(indices.len(), new_lanes, "{file}: {indices:?}");
        ed.select(Some(Sel::Spline(key, outside)));
        ed.retype_splines(file);
        ed.select(Some(Sel::Object(key, exit.id)));
        let before_path = sco_path(&exit.file);
        let options = crate::junction::Options::read(&before_path);
        ed.set_junction_options(exit.id, options, true);
        ed.tick(&mut assets, 0.0, false);
        let tile = &ed.doc.as_ref().unwrap().tiles[&key].tile;
        assert_eq!(tile.objects.len(), 2);
        assert!(tile.objects.iter().any(|o| o.id == entry.id && o.file == entry.file && o.rules == entry.rules));
        assert_eq!(tile.splines.iter().find(|s| s.id == bay.id).unwrap().rules, bay.rules);
        let rebuilt = tile.objects.iter().find(|o| o.id != entry.id).unwrap();
        let after_path = sco_path(&rebuilt.file);
        let before = omsi_cfg::vfs::read_text(&before_path).unwrap();
        let after = omsi_cfg::vfs::read_text(&after_path).unwrap();
        println!("Replacement {file}, bay indices {indices:?}\nBefore {}: {:#?}\n{before}\nAfter {}: {:#?}\n{after}",
            exit.id, exit.rules, rebuilt.id, rebuilt.rules);
        assert_ne!(rebuilt.id, exit.id);
        assert_eq!(rebuilt.pos, exit.pos);
        assert_eq!(ed.selected, Some(Sel::Object(key, rebuilt.id)));
        assert!(is_bus_bay_junction(&rebuilt.file));
        assert!(after.lines().any(|line| line == file));
        assert_ne!(mesh_text(&exit.file), mesh_text(&rebuilt.file), "{file}: mesh must fit the changed road width");
        let sco = omsi_scenery::SceneryObject::load(&after_path).unwrap();
        assert!(sco.paths.iter().any(|p| p.kind == 0), "{file}: vehicle paths must survive");
        // (the bay's bus-only rules are kept beside the object, not in its own rules)
        assert!(rebuilt.rules.is_empty(), "{file}");
        assert_eq!(!template_of(&content, &rebuilt.file).is_empty(), restricted, "{file}");
        exit = rebuilt.clone();
    }
    std::fs::remove_dir_all(dir).unwrap();
}
