//! A parallel copy of roads or tracks (a second track, the other carriageway): each spline
//! offset sideways as OMSI's arcs allow exactly (an arc keeps its centre, its radius grows or
//! shrinks by the offset), the copies linked as the originals are.

use std::collections::{HashMap, VecDeque};

use glam::DVec2;
use omsi_map::MapSpline;

use crate::doc::{tile_of, tile_origin, Document, TileKey};
use crate::i18n::tr;

/// The copies of `splines`, `d` metres to the right (negative: left) of the way the first
/// runs; a chain keeps one side however its splines are turned. Their keys and IDCodes.
pub fn copy(doc: &mut Document, splines: &[(TileKey, i64)], d: f64) -> Result<Vec<(TileKey, i64)>, &'static str> {
    if d.abs() < 0.01 {
        return Err(tr("Give a distance", "Укажите расстояние"));
    }
    let wanted: HashMap<i64, TileKey> = splines.iter().map(|(k, id)| (*id, *k)).collect();
    // each spline's side, walked along the links: a neighbour running back has its right on
    // this one's left
    let mut side: HashMap<i64, f64> = HashMap::new();
    for (_, first) in splines {
        if side.contains_key(first) {
            continue;
        }
        side.insert(*first, 1.0);
        let mut queue = VecDeque::from([*first]);
        while let Some(id) = queue.pop_front() {
            let Some((_, s)) = doc.find_spline(id) else { continue };
            let sg = side[&id];
            for (link, at_end) in [(s.next_id, true), (s.prev_id, false)] {
                if link == 0 || !wanted.contains_key(&link) || side.contains_key(&link) {
                    continue;
                }
                let Some((_, n)) = doc.find_spline(link) else { continue };
                // (same way: it starts where this ends, or ends where this starts)
                let same = if at_end { n.prev_id == id } else { n.next_id == id };
                side.insert(link, if same { sg } else { -sg });
                queue.push_back(link);
            }
        }
    }
    // the copies, worked out first: nothing changes if one cannot be made
    let mut made: Vec<(i64, TileKey, MapSpline)> = Vec::new();
    for (k, id) in splines {
        let Some(s) = doc.spline(*k, *id) else { continue };
        let off = d * side.get(id).copied().unwrap_or(1.0);
        let org = tile_origin(*k);
        let h = s.heading.to_radians();
        let right = DVec2::new(h.cos(), -h.sin());
        let start = org + DVec2::new(s.pos[0], s.pos[1]) + right * off;
        let (radius, length) = if s.radius == 0.0 {
            (0.0, s.length)
        } else {
            // (the centre lies on the side the arc turns to: right for a positive radius)
            let r = s.radius - off;
            if r == 0.0 || r.signum() != s.radius.signum() {
                return Err(tr("The bend is too tight for that distance on its inner side", "Поворот слишком крутой для такого расстояния с внутренней стороны"));
            }
            (r, s.length * r / s.radius)
        };
        let nk = tile_of(start);
        if !doc.tiles.contains_key(&nk) {
            return Err(tr("The copy leaves the map: add tiles there first", "Копия выходит за карту: сначала добавьте тайлы"));
        }
        let no = tile_origin(nk);
        let scale = s.length / length.max(0.01);
        let copy = MapSpline {
            pos: [start.x - no.x, start.y - no.y, s.pos[2]],
            radius,
            length,
            grad_start: s.grad_start * scale,
            grad_end: s.grad_end * scale,
            tex_offset: s.tex_offset / scale,
            map_chain_offset: s.map_chain_offset.map(|o| o / scale),
            prev_id: 0,
            next_id: 0,
            deleted: false,
            ..s.clone()
        };
        made.push((*id, nk, copy));
    }
    let ids: HashMap<i64, i64> = made.iter().map(|(old, _, _)| (*old, doc.new_id())).collect();
    let mut out = Vec::new();
    for (old, k, mut s) in made {
        let o = doc.spline(wanted[&old], old).cloned().unwrap_or_default();
        s.id = ids[&old];
        // linked as the originals are, among the copies
        s.prev_id = ids.get(&o.prev_id).copied().unwrap_or(0);
        s.next_id = ids.get(&o.next_id).copied().unwrap_or(0);
        out.push((k, s.id));
        let t = doc.tiles.get_mut(&k).unwrap();
        t.tile.splines.push(s);
        t.touch_content();
    }
    crate::road_maker::bus_rules::apply_for_splines(doc, &out, "parallel::copy");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::tests::{add_spline, flat_map};
    use omsi_geometry::SplineCurve;

    fn curve(doc: &Document, id: i64) -> SplineCurve {
        let (k, s) = doc.find_spline(id).unwrap();
        SplineCurve::from_map(s, tile_origin(k))
    }

    /// A straight, a bend to the right and one coming back the other way: every copy runs
    /// at the distance asked, on one side, and the copies meet as the originals do.
    #[test]
    fn a_chain_is_copied_alongside() {
        let mut doc = flat_map(2);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 50.0, 0, 2);
        // a right bend from the straight's end
        let a = SplineCurve::from_map(doc.spline((0, 0), 1).unwrap(), DVec2::ZERO);
        let e = a.end_point();
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.push(MapSpline { file: "Splines\\road.sli".into(), id: 2, prev_id: 1, next_id: 3, pos: [e.x, e.y, 0.0], heading: 0.0, length: 40.0, radius: 60.0, ..Default::default() });
        // the third runs back into the bend's end
        let b = curve(&doc, 2);
        let (be, bh) = (b.end_point(), b.heading_at(40.0));
        let far = be.truncate() + DVec2::new(bh.to_radians().sin(), bh.to_radians().cos()) * 30.0;
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.push(MapSpline { file: "Splines\\road.sli".into(), id: 3, prev_id: 0, next_id: 2, pos: [far.x, far.y, 0.0], heading: crate::roads::norm_deg(bh + 180.0), length: 30.0, ..Default::default() });
        let made = copy(&mut doc, &[((0, 0), 1), ((0, 0), 2), ((0, 0), 3)], -4.0).unwrap();
        assert_eq!(made.len(), 3);
        // each copy 4 m from its original all along
        for ((_, new), old) in made.iter().zip([1, 2, 3]) {
            let (c, o) = (curve(&doc, *new), curve(&doc, old));
            for i in 0..=10 {
                let p = c.point_at(c.length * i as f64 / 10.0).truncate();
                let near = (0..=400).map(|j| (o.point_at(o.length * j as f64 / 400.0).truncate() - p).length()).fold(f64::MAX, f64::min);
                assert!((near - 4.0).abs() < 0.05, "spline {old}: {near}");
            }
        }
        // on the left of the first one's way (the bend's outside is its left: radius grows)
        assert!((doc.spline(made[1].0, made[1].1).unwrap().radius - 64.0).abs() < 1e-9);
        // linked as the originals: 1 -> 2, 3 runs back into 2
        let (n1, n2, n3) = (made[0].1, made[1].1, made[2].1);
        assert_eq!(doc.spline((0, 0), n1).unwrap().next_id, n2);
        assert_eq!(doc.spline((0, 0), n2).unwrap().next_id, n3);
        assert_eq!(doc.spline((0, 0), n3).unwrap().next_id, n2);
        // the ends meet
        assert!((curve(&doc, n1).end_point() - curve(&doc, n2).point_at(0.0)).length() < 1e-6);
        assert!((curve(&doc, n2).end_point() - curve(&doc, n3).end_point()).length() < 1e-6);
    }

    #[test]
    fn too_tight_a_bend_is_refused() {
        let mut doc = flat_map(1);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.push(MapSpline { file: "x.sli".into(), id: 1, pos: [100.0, 100.0, 0.0], length: 10.0, radius: 3.0, ..Default::default() });
        assert!(copy(&mut doc, &[((0, 0), 1)], 4.0).is_err());
        assert_eq!(doc.tiles[&(0, 0)].tile.splines.len(), 1);
    }
}
