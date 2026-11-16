use super::*;
use crate::selection::tests::flat_map;

/// The AI path types, a crossing object with a path, and a map of flat ground with the
/// editor drawing AI paths on it.
struct Paths {
    dir: std::path::PathBuf,
    street: String,
    walk: String,
    cross: String,
    assets: Assets,
    editor: Editor,
}

impl Paths {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("om-path-snap-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let content = dir.join("content");
        std::fs::create_dir_all(&content).unwrap();
        let street = "Splines\\Marcel\\invis_street.sli".to_string();
        let walk = "Splines\\invis_sidewalk.sli".to_string();
        let cross = "Sceneryobjects\\cross.sco".to_string();
        // (`[path]` of a `.sli`: kind, x to the right, z up, width, direction; of a `.sco`:
        // its start, heading / radius / length, and the same kind, width and direction)
        for (rel, text) in [
            (&street, "[path]\n0\n0\n0.1\n3\n0\n"),
            (&walk, "[path]\n1\n0\n-2\n2\n2\n"),
            (&cross, "[path]\n0\n0\n0.1\n0\n0\n50\n0\n0\n0\n3\n0\n0\n"),
        ] {
            let p = omsi_cfg::resolve_path(&content, rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
        }
        let mut assets = Assets::new(dir.join("omsi"), content.clone());
        for rel in [&street, &walk] {
            assert!(assets.spline_wait(rel, 5000).is_some(), "{rel} loads");
        }
        assert!(assets.object_wait(&cross, 5000).is_some(), "{cross} loads");
        let mut doc = flat_map(1);
        doc.dir = content.join("maps/Test");
        let mut editor = Editor::default();
        editor.content_dir = Some(content.clone());
        editor.open(doc);
        editor.tool = Tool::Logic;
        editor.logic = LogicMode::Paths;
        editor.auto_junctions = false;
        Self { dir, street, walk, cross, assets, editor }
    }

    /// A spline of `file` laid from (`x`, `y`), heading, along the ground at z 0.
    fn lane(&mut self, id: i64, file: &str, x: f64, y: f64, heading: f64, length: f64) {
        let doc = self.editor.doc.as_mut().unwrap();
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        t.tile.splines.push(MapSpline { file: file.to_string(), id, pos: [x, y, 0.0], heading, length, ..Default::default() });
        t.touch_content();
    }

    /// A crossing object of `file` standing at (`x`, `y`) on the ground.
    fn object(&mut self, id: i64, file: &str, x: f64, y: f64) {
        let doc = self.editor.doc.as_mut().unwrap();
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        t.tile.objects.push(MapObject { file: file.to_string(), id, pos: [x, y, 0.0], ..Default::default() });
        t.touch_content();
    }

    fn type_of(&mut self, file: &str) {
        self.editor.path_type = Some(file.to_string());
        self.editor.road_type = Some(file.to_string());
    }

    /// A click at (`x`, `y`) on the ground: starts the path, or builds it.
    fn click(&mut self, x: f64, y: f64) {
        self.editor.cursor = Some(DVec3::new(x, y, 0.0));
        self.editor.plan_road(&mut self.assets);
        self.editor.road_click(&mut self.assets, &Library::default());
    }

    /// The plan drawn to (`x`, `y`) as (start height, end height).
    fn plan(&mut self, x: f64, y: f64) -> (f64, f64) {
        self.editor.cursor = Some(DVec3::new(x, y, 0.0));
        self.editor.plan_road(&mut self.assets);
        let p = self.editor.road_plan.as_ref().expect("a plan");
        (p.1, p.2)
    }

    /// The spline of `file` made in the map, starting nearest (`x`, `y`).
    fn made(&self, file: &str, x: f64, y: f64) -> Option<(TileKey, MapSpline)> {
        let doc = self.editor.doc.as_ref().unwrap();
        let mut best: Option<(f64, TileKey, MapSpline)> = None;
        for (k, t) in &doc.tiles {
            for s in t.tile.splines.iter().filter(|s| !s.deleted && s.file.eq_ignore_ascii_case(file)) {
                let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(*k));
                let d = (c.point_at(0.0).truncate() - DVec2::new(x, y)).length();
                if d < 1.0 && best.as_ref().map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, *k, s.clone()));
                }
            }
        }
        best.map(|b| (b.1, b.2))
    }
}

impl Drop for Paths {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The path's own height along a spline, at its start and at its end (world).
fn world_heights(k: TileKey, s: &MapSpline, file: &str, assets: &mut Assets) -> (f64, f64) {
    let z = assets.spline(file).and_then(|m| m.def.paths.first().map(|p| p.start[2] as f64)).unwrap_or_else(|| panic!("{file} loads"));
    let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
    (c.point_at(0.0).z + z, c.end_point().z + z)
}

#[test]
fn a_path_started_on_a_street_lane_keeps_the_lane_height() {
    let mut f = Paths::new("street");
    let street = f.street.clone();
    f.lane(1, &street, 100.0, 100.0, 0.0, 50.0);
    f.lane(2, &street, 100.0, 160.0, 0.0, 50.0);
    f.type_of(&street);
    // from the far end of one lane to the near end of another: both lie 0.1 m above
    // the ground, so the new path's line is laid 0.1 m below them
    f.click(100.0, 150.0);
    let (start, _) = f.editor.road_start.expect("started on the lane's end");
    assert!((start.pos.z - 0.0).abs() < 1e-9, "started at {}", start.pos.z);
    let (z0, z1) = f.plan(100.0, 160.0);
    assert!((z0 - 0.0).abs() < 1e-9, "z0 {z0}");
    assert!((z1 - 0.0).abs() < 1e-9, "z1 {z1}");
    f.click(100.0, 160.0);
    let (k, s) = f.made(&street, 100.0, 150.0).expect("the path is made");
    let (at_start, at_end) = world_heights(k, &s, &street, &mut f.assets);
    assert!((at_start - 0.1).abs() < 0.001, "start path height {at_start}");
    assert!((at_end - 0.1).abs() < 0.001, "end path height {at_end}");
}

#[test]
fn a_path_started_on_a_footway_lane_keeps_the_lane_height() {
    let mut f = Paths::new("walk");
    let walk = f.walk.clone();
    f.lane(1, &walk, 100.0, 100.0, 0.0, 50.0);
    f.lane(2, &walk, 100.0, 160.0, 0.0, 50.0);
    f.type_of(&walk);
    // the footpath's line lies 2 m below the lane's
    f.click(100.0, 150.0);
    let (start, _) = f.editor.road_start.expect("started on the lane's end");
    assert!((start.pos.z - 0.0).abs() < 1e-9, "started at {}", start.pos.z);
    let (z0, z1) = f.plan(100.0, 160.0);
    assert!((z0 - 0.0).abs() < 1e-9, "z0 {z0}");
    assert!((z1 - 0.0).abs() < 1e-9, "z1 {z1}");
    f.click(100.0, 160.0);
    let (k, s) = f.made(&walk, 100.0, 150.0).expect("the path is made");
    let (at_start, at_end) = world_heights(k, &s, &walk, &mut f.assets);
    assert!((at_start + 2.0).abs() < 0.001, "start path height {at_start}");
    assert!((at_end + 2.0).abs() < 0.001, "end path height {at_end}");
}

#[test]
fn a_path_started_on_the_ground_keeps_the_ground_height() {
    let mut f = Paths::new("ground");
    let walk = f.walk.clone();
    // (a lane 10 m away: no end and no place of it is taken)
    f.lane(1, &walk, 100.0, 100.0, 0.0, 50.0);
    f.type_of(&walk);
    f.click(250.0, 250.0);
    let (start, _) = f.editor.road_start.expect("started on the ground");
    assert!((start.pos.z - 0.0).abs() < 1e-9, "started at {}", start.pos.z);
    let (z0, _) = f.plan(250.0, 280.0);
    assert!((z0 - 0.0).abs() < 1e-9, "z0 {z0}");
}

#[test]
fn a_path_started_in_the_middle_of_a_lane_keeps_the_lane_height() {
    let mut f = Paths::new("mid");
    let street = f.street.clone();
    f.lane(1, &street, 100.0, 100.0, 0.0, 50.0);
    f.type_of(&street);
    f.click(100.0, 125.0);
    let (start, fixed) = f.editor.road_start.expect("started in the lane");
    assert!(start.mid.is_some(), "a place along the lane");
    assert!(fixed, "the lane's way kept");
    assert!((start.pos.z - 0.0).abs() < 1e-9, "started at {}", start.pos.z);
    let (z0, _) = f.plan(150.0, 125.0);
    assert!((z0 - 0.0).abs() < 1e-9, "z0 {z0}");
}

#[test]
fn a_path_started_on_a_crossing_objects_lane_keeps_the_lane_height() {
    let mut f = Paths::new("object");
    let cross = f.cross.clone();
    let street = f.street.clone();
    f.object(1, &cross, 250.0, 100.0);
    f.type_of(&street);
    // the object's path runs from (250, 100) to (250, 150) 0.1 m above the ground
    f.click(250.0, 150.0);
    let (start, _) = f.editor.road_start.expect("started on the object's path end");
    assert!((start.pos.z - 0.0).abs() < 1e-9, "started at {}", start.pos.z);
    let (z0, z1) = f.plan(250.0, 160.0);
    assert!((z0 - 0.0).abs() < 1e-9, "z0 {z0}");
    assert!((z1 - 0.0).abs() < 1e-9, "z1 {z1}");
    f.click(250.0, 160.0);
    let (k, s) = f.made(&street, 250.0, 150.0).expect("the path is made");
    let (at_start, at_end) = world_heights(k, &s, &street, &mut f.assets);
    assert!((at_start - 0.1).abs() < 0.001, "start path height {at_start}");
    assert!((at_end - 0.1).abs() < 0.001, "end path height {at_end}");
}
