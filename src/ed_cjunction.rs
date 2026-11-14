//! The editor's side of complex and custom junctions (`cjunction`): a complex junction put on
//! the map as splines, and a custom junction used where the Roads tool makes a junction by
//! itself.

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use omsi_map::{MapObject, MapSpline};

use crate::assets::Assets;
use crate::cjunction::{self, Layout};
use crate::doc::{tile_of, tile_origin, Document, Sel, TileKey};
use crate::editor::{conform_disc, reach_along, Editor};
use crate::i18n::tr;
use crate::roads;

/// How far past a custom junction's arm a road is cut: the piece that bends the road onto the
/// arm where they meet at a slightly different heading (m).
const CONNECT: f64 = 3.0;

impl Editor {
    /// The custom junctions saved in the content folder (read once, again after one is saved).
    pub fn custom_layouts(&mut self) -> &[Layout] {
        if self.custom_layouts.is_none() {
            self.custom_layouts = Some(self.content_dir.as_deref().map(Layout::load_all).unwrap_or_default());
        }
        self.custom_layouts.as_deref().unwrap_or(&[])
    }

    /// A complex junction put on the map at `origin`, turned `rot` degrees: ordinary splines,
    /// selected, the ground brought level under them; one undo step.
    pub fn place_layout(&mut self, assets: &mut Assets, layout: &Layout, origin: DVec2, rot: f64) {
        let follow = self.follow_ground;
        let (content, asphalt) = (self.content_dir.clone(), self.asphalt.clone());
        let Some(doc) = self.doc.as_mut() else { return };
        if layout.pieces.iter().all(|p| p.hidden) {
            self.toast(tr("Draw some pieces first", "Сначала нарисуйте детали"));
            return;
        }
        self.history.begin(tr("Complex junction", "Сложный перекрёсток"));
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for k in keys {
            self.history.touch_tile(doc, k);
        }
        self.history.touch_global(doc);
        let z = doc.ground(origin);
        let kinds = cjunction::road_kinds(layout, assets);
        let resolved = layout.resolve(&|f| kinds.get(&f.to_lowercase()).map(|k| k.0).unwrap_or(false), &|f| kinds.get(&f.to_lowercase()).map(|k| k.1).unwrap_or(1.5));
        let mut cut = layout.clone();
        cut.pieces = resolved.pieces;
        match cjunction::place(doc, &cut, origin, z, rot) {
            Ok(made) => {
                let mut things = cjunction::place_objects(doc, layout, origin, z, rot);
                // (the plate between the arms: an object of its own, turned with the drawing)
                if let Some(content) = content {
                    let folder = crate::junction::folder_name(doc);
                    let asphalt = crate::junction::junction_asphalt(Some(&doc.dir), asphalt.as_deref());
                    for arms in &resolved.plates {
                        let plate = crate::junction::layout_plate(assets, arms, asphalt.as_deref());
                        let Some(rel) = plate.and_then(|p| p.write_object(&content, &folder).ok()) else { continue };
                        let k = tile_of(origin);
                        if doc.tiles.contains_key(&k) {
                            let id = doc.new_id();
                            let org = tile_origin(k);
                            let Some(t) = doc.tiles.get_mut(&k) else { return };
                            t.tile.objects.push(MapObject { file: rel, id, pos: [origin.x - org.x, origin.y - org.y, z], rot: [rot, 0.0, 0.0], ..Default::default() });
                            t.touch_content();
                            things.push((k, id));
                        }
                    }
                }
                if follow {
                    conform_disc(doc, DVec3::new(origin.x, origin.y, z), layout.radius() + 1.0, &[]);
                }
                self.history.commit();
                self.set_selection(made.into_iter().map(|(k, id)| Sel::Spline(k, id)).chain(things.into_iter().map(|(k, id)| Sel::Object(k, id))).collect());
                self.toast(tr("Complex junction put on the map: its pieces are ordinary splines", "Сложный перекрёсток на карте: его детали — обычные сплайны"));
            }
            Err(e) => {
                if let Some(d) = self.doc.as_mut() {
                    self.history.rollback(d);
                }
                self.error(e);
            }
        }
    }
}

/// A custom junction in place of an automatic one: where the roads `ends` (each a spline end
/// at `centre`, `None` where there is no road) meet as one saved layout's arms do, the roads
/// are cut back to its arms, bent onto them, and the layout's object put there. Gives back
/// the roads' new ends (in the order given) when it was used; nothing is changed otherwise.
pub(crate) fn apply_custom(doc: &mut Document, assets: &mut Assets, layouts: &[Layout], ends: &[Option<(TileKey, i64, bool)>], centre: DVec3, follow: bool) -> Option<Vec<Option<(TileKey, i64, bool)>>> {
    let candidates: Vec<&Layout> = layouts.iter().filter(|l| l.auto && l.baked.is_some()).collect();
    if candidates.is_empty() || ends.iter().flatten().count() < 3 {
        return None;
    }
    // the roads: where they face and what they are
    let roads_at: Vec<(usize, f64, String)> = ends
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let (k, id, at_end) = (*e)?;
            let s = doc.spline(k, id)?;
            let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
            let out = if at_end { c.heading_at(s.length) + 180.0 } else { c.heading_at(0.0) };
            Some((i, roads::norm_deg(out), s.file.clone()))
        })
        .collect();
    let mut halves: HashMap<String, f32> = HashMap::new();
    for f in roads_at.iter().map(|r| r.2.clone()).chain(candidates.iter().flat_map(|l| l.arms.iter().map(|a| a.road.clone()))).filter(|f| !f.is_empty()) {
        if let Some(m) = assets.spline_wait(&f, 500) {
            halves.insert(f.to_lowercase(), m.half_width);
        }
    }
    let same = |want: &str, have: &str| {
        want.eq_ignore_ascii_case(have) || matches!((halves.get(&want.to_lowercase()), halves.get(&have.to_lowercase())), (Some(a), Some(b)) if (a - b).abs() < 0.75)
    };
    let listed: Vec<(f64, String)> = roads_at.iter().map(|r| (r.1, r.2.clone())).collect();
    let (layout, rot, by_arm) = candidates.iter().filter_map(|l| cjunction::fit(l, &listed, &same).map(|(rot, by, err)| (err, *l, rot, by))).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, l, rot, by)| (l, rot, by))?;
    let baked = layout.baked.clone()?;
    let z = centre.z;
    // everything is checked before anything is cut: the roads must be long enough and their
    // tiles there
    let plan: Vec<(usize, DVec2, f64, f64)> = layout
        .arms
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let p = cjunction::to_world(DVec2::new(a.x, a.y), centre.truncate(), rot);
            (roads_at[by_arm[i]].0, p, roads::norm_deg(a.out + rot + 180.0), DVec2::new(a.x, a.y).length())
        })
        .collect();
    for (slot, p, _, rho) in &plan {
        let (k, id, at_end) = ends[*slot]?;
        if !doc.tiles.contains_key(&tile_of(*p)) || reach_along(doc, k, id, at_end) < rho + CONNECT + 1.0 {
            return None;
        }
    }
    if !doc.tiles.contains_key(&tile_of(centre.truncate())) {
        return None;
    }
    let mut now: Vec<Option<(TileKey, i64, bool)>> = ends.to_vec();
    let mut ends_world = Vec::new();
    for (slot, p, into, rho) in &plan {
        let (k, id, at_end) = ends[*slot]?;
        let file = doc.spline(k, id)?.file.clone();
        let cut = doc.trim(k, id, at_end, rho + CONNECT)?;
        now[*slot] = Some(cut);
        let s = doc.spline(cut.0, cut.1)?.clone();
        let c = omsi_geometry::SplineCurve::from_map(&s, tile_origin(cut.0));
        let (e, h) = if cut.2 { (c.end_point(), c.heading_at(s.length)) } else { (c.point_at(0.0), c.heading_at(0.0) + 180.0) };
        lay_connector(doc, cut, &file, e, roads::norm_deg(h), DVec3::new(p.x, p.y, z), *into);
        ends_world.push(DVec3::new(p.x, p.y, z));
    }
    let k = tile_of(centre.truncate());
    let org = tile_origin(k);
    let id = doc.new_id();
    let t = doc.tiles.get_mut(&k)?;
    t.tile.objects.push(MapObject { file: baked, id, pos: [centre.x - org.x, centre.y - org.y, z], rot: [rot, 0.0, 0.0], ..Default::default() });
    t.touch_content();
    cjunction::place_objects(doc, layout, centre.truncate(), z, rot);
    if follow {
        conform_disc(doc, centre, layout.radius() + 1.0, &ends_world);
    }
    Some(now)
}

/// The piece of road that bends the end `at` of a road (at `from`, going into the junction
/// with heading `h0`) onto a custom junction's arm at `to`, where a road arrives heading
/// `h1`. None is made when they already meet.
fn lay_connector(doc: &mut Document, at: (TileKey, i64, bool), file: &str, from: DVec3, h0: f64, to: DVec3, h1: f64) {
    let diff = ((h1 - h0 + 180.0).rem_euclid(360.0) - 180.0).abs();
    if (from.truncate() - to.truncate()).length() < 0.4 && diff < 2.0 {
        return;
    }
    let Some(segs) = roads::biarc(from.truncate(), h0, to.truncate(), h1).or_else(|| roads::straight(from.truncate(), to.truncate()).map(|s| vec![s])) else { return };
    let total: f64 = segs.iter().map(|s| s.length).sum::<f64>().max(0.01);
    let grad = (to.z - from.z) / total * 100.0;
    // the road runs into the junction from its end; from its start the connector is laid
    // from the junction back to it, so it runs the road's way
    let forward = at.2;
    let ordered: Vec<(roads::Seg, f64)> = {
        let mut acc = 0.0;
        let list: Vec<(roads::Seg, f64)> = segs
            .iter()
            .map(|s| {
                let z0 = from.z + (to.z - from.z) * acc / total;
                acc += s.length;
                (*s, z0)
            })
            .collect();
        if forward {
            list
        } else {
            list.into_iter().rev().map(|(s, z0)| (roads::reverse(&s), z0 + (s.length * grad / 100.0))).collect()
        }
    };
    let ids: Vec<i64> = ordered.iter().map(|_| doc.new_id()).collect();
    for (i, (s, z0)) in ordered.iter().enumerate() {
        let k = tile_of(s.start);
        let Some(t) = doc.tiles.get_mut(&k) else { return };
        let org = tile_origin(k);
        let g = if forward { grad } else { -grad };
        t.tile.splines.push(MapSpline {
            file: file.to_string(),
            id: ids[i],
            prev_id: if i > 0 { ids[i - 1] } else if forward { at.1 } else { 0 },
            next_id: if i + 1 < ids.len() { ids[i + 1] } else if forward { 0 } else { at.1 },
            pos: [s.start.x - org.x, s.start.y - org.y, *z0],
            heading: s.heading,
            length: s.length,
            radius: s.radius,
            grad_start: g,
            grad_end: g,
            ..Default::default()
        });
        t.touch_content();
    }
    // the road's end takes the link
    if let Some(t) = doc.tiles.get_mut(&at.0) {
        if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == at.1) {
            if forward { s.next_id = ids[0] } else { s.prev_id = *ids.last().unwrap() }
        }
        t.touch_content();
    }
}
