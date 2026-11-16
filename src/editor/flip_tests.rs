use super::*;
use crate::selection::tests::{add_spline, flat_map};

/// A road's start, end and heading as the map has them.
fn shape(doc: &Document, k: TileKey, id: i64) -> (DVec3, DVec3, f64) {
    let s = doc.spline(k, id).expect("the road");
    let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
    (c.point_at(0.0), c.end_point(), c.heading_at(s.length))
}

fn editor_with(id: i64, heading: f64) -> Editor {
    let mut doc = flat_map(1);
    add_spline(&mut doc, (0, 0), id, 100.0, 100.0, heading, 5.0, 0, 0);
    let mut ed = Editor::default();
    ed.open(doc);
    ed.set_selection(vec![Sel::Spline((0, 0), id)]);
    ed
}

#[test]
fn one_road_turns_round_about_its_start() {
    let mut ed = editor_with(1, 30.0);
    ed.flip_selected();
    let doc = ed.doc.as_ref().unwrap();
    let (start, end, heading) = shape(doc, (0, 0), 1);
    let p = DVec2::new(100.0, 100.0);
    assert!((end.truncate() - p).length() < 0.001, "end {:?}", end.truncate());
    assert!((heading - 30.0).abs() < 1e-6, "heading {heading}");
    let wanted = p - roads::dir(30.0) * 5.0;
    assert!((start.truncate() - wanted).length() < 0.001, "start {:?}, wanted {wanted:?}", start.truncate());
}

#[test]
fn angle_snapping_does_not_stop_the_half_turn() {
    let mut ed = editor_with(1, 37.0);
    ed.snap_angle = true;
    ed.flip_selected();
    let doc = ed.doc.as_ref().unwrap();
    let (start, end, heading) = shape(doc, (0, 0), 1);
    let p = DVec2::new(100.0, 100.0);
    assert!((end.truncate() - p).length() < 0.001, "end {:?}", end.truncate());
    assert!((heading - 37.0).abs() < 1e-6, "heading {heading}");
    let wanted = p - roads::dir(37.0) * 5.0;
    assert!((start.truncate() - wanted).length() < 0.001, "start {:?}, wanted {wanted:?}", start.truncate());
}

#[test]
fn one_undo_puts_the_road_back() {
    let mut ed = editor_with(1, 30.0);
    let before = shape(ed.doc.as_ref().unwrap(), (0, 0), 1);
    ed.flip_selected();
    ed.history.undo(ed.doc.as_mut().unwrap());
    let after = shape(ed.doc.as_ref().unwrap(), (0, 0), 1);
    assert!((after.0 - before.0).length() < 1e-9, "start {:?} from {:?}", after.0, before.0);
    assert!((after.1 - before.1).length() < 1e-9, "end {:?} from {:?}", after.1, before.1);
    assert!((after.2 - before.2).abs() < 1e-9, "heading {} from {}", after.2, before.2);
}

#[test]
fn two_roads_are_left_alone() {
    let mut doc = flat_map(1);
    add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 30.0, 5.0, 0, 0);
    add_spline(&mut doc, (0, 0), 2, 200.0, 100.0, 60.0, 5.0, 0, 0);
    let mut ed = Editor::default();
    ed.open(doc);
    ed.set_selection(vec![Sel::Spline((0, 0), 1), Sel::Spline((0, 0), 2)]);
    let before = (shape(ed.doc.as_ref().unwrap(), (0, 0), 1), shape(ed.doc.as_ref().unwrap(), (0, 0), 2));
    ed.flip_selected();
    let after = (shape(ed.doc.as_ref().unwrap(), (0, 0), 1), shape(ed.doc.as_ref().unwrap(), (0, 0), 2));
    for (a, b) in [(after.0, before.0), (after.1, before.1)] {
        assert!((a.0 - b.0).length() < 1e-12, "start {:?} from {:?}", a.0, b.0);
        assert!((a.1 - b.1).length() < 1e-12, "end {:?} from {:?}", a.1, b.1);
        assert!((a.2 - b.2).abs() < 1e-12, "heading {} from {}", a.2, b.2);
    }
    assert_eq!(ed.selection(), vec![Sel::Spline((0, 0), 1), Sel::Spline((0, 0), 2)]);
}
