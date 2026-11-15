//! Copy, cut and paste: the selected objects and splines with what hangs on them, kept
//! relative to their middle and put down again round another point with fresh IDCodes.
//! Links between copied splines are kept; links to splines left behind are not.

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use omsi_map::{EntryPoint, MapObject, MapSpline, SplineAttachment};

use crate::doc::{tile_of, tile_origin, Document, Sel, TileKey};

#[derive(Debug, Clone)]
struct ClipObject {
    o: MapObject,
    /// From the clip's middle (x, y; z only for an object at an absolute height).
    off: DVec3,
    abs: bool,
    /// The start point's name, when it is one.
    entry: Option<String>,
}

#[derive(Debug, Clone)]
struct ClipSpline {
    s: MapSpline,
    /// Its start from the clip's middle (z: from the ground there).
    off: DVec3,
    /// Its rows of objects (`spline_index` unused).
    rows: Vec<SplineAttachment>,
}

/// What was copied.
#[derive(Debug, Clone, Default)]
pub struct Clip {
    objects: Vec<ClipObject>,
    splines: Vec<ClipSpline>,
    /// `[attachObj]` objects hanging on the copied objects or rows (by their old IDCodes).
    attached: Vec<MapObject>,
    /// Where its middle was (the ground's height there).
    pub at: DVec3,
}

impl Clip {
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty() && self.splines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.objects.len() + self.splines.len()
    }
}

impl Clip {
    /// The clip as a tile of its own, laid round (0, 0) (a saved group: see `groups`).
    pub fn to_tile(&self) -> omsi_map::Tile {
        let mut t = omsi_map::Tile::default();
        for c in &self.objects {
            let mut o = c.o.clone();
            o.pos = [c.off.x, c.off.y, if c.abs { c.off.z } else { o.pos[2] }];
            t.objects.push(o);
        }
        for c in &self.splines {
            let mut s = c.s.clone();
            s.pos = [c.off.x, c.off.y, c.off.z];
            let idx = t.splines.len() as i32;
            t.splines.push(s);
            for r in &c.rows {
                t.spline_attachments.push(SplineAttachment { spline_index: idx, ..r.clone() });
            }
        }
        t.attach_objects = self.attached.clone();
        t
    }

    /// A clip from a tile `to_tile` made; `abs` tells an object type that stands at an
    /// absolute height.
    pub fn from_tile(t: &omsi_map::Tile, abs: impl Fn(&str) -> bool) -> Clip {
        let mut clip = Clip::default();
        for o in &t.objects {
            let a = abs(&o.file);
            clip.objects.push(ClipObject { o: o.clone(), off: DVec3::new(o.pos[0], o.pos[1], if a { o.pos[2] } else { 0.0 }), abs: a, entry: None });
        }
        for (i, s) in t.splines.iter().enumerate().filter(|(_, s)| !s.deleted) {
            let rows = t.spline_attachments.iter().filter(|r| r.spline_index == i as i32).cloned().collect();
            clip.splines.push(ClipSpline { s: s.clone(), off: DVec3::new(s.pos[0], s.pos[1], s.pos[2]), rows });
        }
        clip.attached = t.attach_objects.clone();
        clip
    }
}

/// Copy a selection; `abs` tells an object type that stands at an absolute height.
pub fn copy(doc: &Document, sels: &[Sel], abs: impl Fn(&str) -> bool) -> Clip {
    let Some(mid) = crate::selection::centre(doc, sels) else { return Clip::default() };
    let at = DVec3::new(mid.x, mid.y, doc.ground(mid));
    let mut clip = Clip { at, ..Default::default() };
    let mut parents: Vec<(TileKey, i64)> = Vec::new();
    for s in sels {
        match *s {
            Sel::Object(k, id) => {
                let Some(o) = doc.object(k, id) else { continue };
                let org = tile_origin(k);
                let a = abs(&o.file);
                let off = DVec3::new(org.x + o.pos[0] - at.x, org.y + o.pos[1] - at.y, if a { o.pos[2] - at.z } else { 0.0 });
                let entry = doc.global.entry_points.iter().find(|e| e.object_id == id).map(|e| e.name.clone());
                clip.objects.push(ClipObject { o: o.clone(), off, abs: a, entry });
                parents.push((k, id));
            }
            Sel::Attached(k, id) => {
                // (copied as the free object it looks like, where its first copy stands)
                let Some(a) = doc.attachment(k, id) else { continue };
                let Some(first) = crate::selection::attachment_first(doc, k, id) else { continue };
                let org = tile_origin(k);
                let world = first.pose.pos;
                let a2 = abs(&a.file);
                let o = MapObject {
                    file: a.file.clone(),
                    id: a.id,
                    pos: [world.x - org.x, world.y - org.y, if a2 { world.z } else { world.z - doc.ground(world.truncate()) }],
                    rot: [crate::selection::attachment_world_rot(doc, k, a), a.rot[1], a.rot[2]],
                    flag: a.strings.len() as i32,
                    extra: a.strings.clone(),
                    var_parent: a.var_parent,
                    rules: a.rules.clone(),
                    ..Default::default()
                };
                let off = DVec3::new(world.x - at.x, world.y - at.y, if a2 { world.z - at.z } else { 0.0 });
                clip.objects.push(ClipObject { o, off, abs: a2, entry: None });
                parents.push((k, id));
            }
            Sel::Spline(k, id) => {
                let Some(t) = doc.tiles.get(&k) else { continue };
                let Some(idx) = t.tile.splines.iter().position(|x| x.id == id && !x.deleted) else { continue };
                let sp = &t.tile.splines[idx];
                let org = tile_origin(k);
                let off = DVec3::new(org.x + sp.pos[0] - at.x, org.y + sp.pos[1] - at.y, sp.pos[2] - at.z);
                // (a repeater carries on a row of another tile: it is not copied with this one)
                let rows: Vec<SplineAttachment> = t.tile.spline_attachments.iter().filter(|a| a.spline_index == idx as i32 && a.repeater.is_none()).cloned().collect();
                for r in &rows {
                    parents.push((k, r.id));
                }
                clip.splines.push(ClipSpline { s: sp.clone(), off, rows });
            }
        }
    }
    // what hangs on the copied things, and on what hangs on them
    let mut i = 0;
    while i < parents.len() {
        let (k, pid) = parents[i];
        if let Some(t) = doc.tiles.get(&k) {
            for a in t.tile.attach_objects.iter().filter(|a| a.parent_id == Some(pid)) {
                if !clip.attached.iter().any(|x| x.id == a.id) {
                    clip.attached.push(a.clone());
                    parents.push((k, a.id));
                }
            }
        }
        i += 1;
    }
    clip
}

/// Put a clip down with its middle at `at` (a ground point), with new IDCodes. What would
/// lie outside the map is left out. Returns what was pasted.
pub fn paste(doc: &mut Document, clip: &Clip, at: DVec3) -> Vec<Sel> {
    let dz = at.z - clip.at.z;
    let mut out = Vec::new();
    let mut ids: HashMap<i64, i64> = HashMap::new();
    // where each copied thing goes (left out: no tile there)
    let obj_tiles: Vec<Option<TileKey>> = clip.objects.iter().map(|c| Some(tile_of(at.truncate() + c.off.truncate())).filter(|k| doc.tiles.contains_key(k))).collect();
    let spl_tiles: Vec<Option<TileKey>> = clip.splines.iter().map(|c| Some(tile_of(at.truncate() + c.off.truncate())).filter(|k| doc.tiles.contains_key(k))).collect();
    for (c, k) in clip.objects.iter().zip(&obj_tiles) {
        if k.is_some() {
            ids.insert(c.o.id, doc.new_id());
        }
    }
    for (c, k) in clip.splines.iter().zip(&spl_tiles) {
        if k.is_some() {
            ids.insert(c.s.id, doc.new_id());
            for r in &c.rows {
                ids.insert(r.id, doc.new_id());
            }
        }
    }
    // (where each parent went, for what hangs on it)
    let mut parent_tile: HashMap<i64, TileKey> = HashMap::new();
    let mut entries = Vec::new();
    for (c, k) in clip.objects.iter().zip(&obj_tiles) {
        let Some(k) = *k else { continue };
        let org = tile_origin(k);
        let p = at.truncate() + c.off.truncate();
        let mut o = c.o.clone();
        o.id = ids[&c.o.id];
        // (a signal head copied with its junction takes its light program from the copy)
        o.var_parent = o.var_parent.map(|p| ids.get(&p).copied().unwrap_or(p));
        o.pos[0] = p.x - org.x;
        o.pos[1] = p.y - org.y;
        if c.abs {
            o.pos[2] = at.z + c.off.z;
        }
        parent_tile.insert(c.o.id, k);
        if let Some(name) = &c.entry {
            entries.push(EntryPoint { object_id: o.id, name: format!("{name} 2"), ..Default::default() });
        }
        let t = doc.tiles.get_mut(&k).unwrap();
        t.tile.objects.push(o);
        t.touch_content();
        out.push(Sel::Object(k, ids[&c.o.id]));
    }
    let link = |old: i64| ids.get(&old).copied().unwrap_or(0);
    for (c, k) in clip.splines.iter().zip(&spl_tiles) {
        let Some(k) = *k else { continue };
        let org = tile_origin(k);
        let p = at + c.off;
        let mut s = c.s.clone();
        s.id = ids[&c.s.id];
        s.prev_id = if c.s.prev_id != 0 { link(c.s.prev_id) } else { 0 };
        s.next_id = if c.s.next_id != 0 { link(c.s.next_id) } else { 0 };
        s.pos = [p.x - org.x, p.y - org.y, c.s.pos[2] + dz];
        let t = doc.tiles.get_mut(&k).unwrap();
        let idx = t.tile.splines.len() as i32;
        t.tile.splines.push(s);
        for r in &c.rows {
            parent_tile.insert(r.id, k);
            let mut r = r.clone();
            r.id = ids[&r.id];
            r.spline_index = idx;
            t.tile.spline_attachments.push(r);
        }
        t.touch_content();
        out.push(Sel::Spline(k, ids[&c.s.id]));
    }
    let made: Vec<_> = out.iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((*k, *id)) } else { None }).collect();
    crate::road_maker::bus_rules::apply_for_splines(doc, &made, "clipboard::paste");
    // what hangs on them, in order (a parent comes before what hangs on it)
    for a in &clip.attached {
        let Some(pid) = a.parent_id else { continue };
        let (Some(&k), Some(&np)) = (parent_tile.get(&pid), ids.get(&pid)) else { continue };
        let mut a = a.clone();
        let id = doc.new_id();
        ids.insert(a.id, id);
        parent_tile.insert(a.id, k);
        a.id = id;
        a.parent_id = Some(np);
        let t = doc.tiles.get_mut(&k).unwrap();
        t.tile.attach_objects.push(a);
        t.touch_content();
    }
    if !entries.is_empty() {
        doc.global.entry_points.extend(entries);
        crate::editor::sync_entry_points(doc);
    }
    doc.global_dirty = true;
    out
}

/// The tiles a paste at `at` writes into.
pub fn tiles_touched(doc: &Document, clip: &Clip, at: DVec2) -> Vec<TileKey> {
    let mut v: Vec<TileKey> = clip
        .objects
        .iter()
        .map(|c| c.off.truncate())
        .chain(clip.splines.iter().map(|c| c.off.truncate()))
        .map(|o| tile_of(at + o))
        .filter(|k| doc.tiles.contains_key(k))
        .collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::tests::{add_object, add_spline, flat_map};

    #[test]
    fn a_paste_keeps_the_layout_and_the_links_inside_it() {
        let mut doc = flat_map(2);
        add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
        add_spline(&mut doc, (0, 0), 2, 90.0, 80.0, 0.0, 20.0, 9, 3);
        add_spline(&mut doc, (0, 0), 3, 90.0, 100.0, 0.0, 20.0, 2, 0);
        add_spline(&mut doc, (0, 0), 9, 90.0, 60.0, 0.0, 20.0, 0, 2);
        let sels = [Sel::Object((0, 0), 1), Sel::Spline((0, 0), 2), Sel::Spline((0, 0), 3)];
        let clip = copy(&doc, &sels, |_| false);
        assert_eq!(clip.len(), 3);
        let before = doc.global.next_id_code;
        let to = clip.at + DVec3::new(200.0, 50.0, 0.0);
        let got = paste(&mut doc, &clip, to);
        assert_eq!(got.len(), 3);
        // fresh IDCodes, none of the old ones
        let new_ids: Vec<i64> = got.iter().map(|s| match s { Sel::Object(_, id) | Sel::Spline(_, id) | Sel::Attached(_, id) => *id }).collect();
        assert!(new_ids.iter().all(|id| *id >= before), "{new_ids:?}");
        // the object stands 200 east, 50 north: over in the next tile
        let Sel::Object(k, oid) = got[0] else { panic!() };
        assert_eq!(k, (1, 0));
        let o = doc.object(k, oid).unwrap();
        assert!((o.pos[0] - 0.0).abs() < 1e-9 && (o.pos[1] - 150.0).abs() < 1e-9, "{:?}", o.pos);
        // the copied chain is linked in itself, and not to the spline left behind
        let (Sel::Spline(ka, a), Sel::Spline(kb, b)) = (got[1], got[2]) else { panic!() };
        let (sa, sb) = (doc.spline(ka, a).unwrap(), doc.spline(kb, b).unwrap());
        assert_eq!((sa.prev_id, sa.next_id), (0, b));
        assert_eq!((sb.prev_id, sb.next_id), (a, 0));
        // the originals are as they were
        assert_eq!(doc.spline((0, 0), 2).unwrap().prev_id, 9);
    }

    #[test]
    fn rows_and_hanging_objects_come_along_with_new_ids() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 2, 50.0, 50.0, 0.0, 20.0, 0, 0);
        add_object(&mut doc, (0, 0), 1, 60.0, 60.0);
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        t.tile.spline_attachments.push(SplineAttachment { file: "lamp.sco".into(), id: 5, spline_index: 0, interval: 10.0, range: 20.0, ..Default::default() });
        t.tile.attach_objects.push(MapObject { file: "sign.sco".into(), id: 6, parent_id: Some(1), ..Default::default() });
        t.tile.attach_objects.push(MapObject { file: "bulb.sco".into(), id: 7, parent_id: Some(5), ..Default::default() });
        let clip = copy(&doc, &[Sel::Spline((0, 0), 2), Sel::Object((0, 0), 1)], |_| false);
        let got = paste(&mut doc, &clip, clip.at + DVec3::new(10.0, 0.0, 0.0));
        assert_eq!(got.len(), 2);
        let t = &doc.tiles[&(0, 0)].tile;
        assert_eq!(t.spline_attachments.len(), 2);
        let row = &t.spline_attachments[1];
        assert_eq!(row.spline_index, 1, "on the pasted spline");
        assert_ne!(row.id, 5);
        assert_eq!(t.attach_objects.len(), 4);
        let new_obj = got.iter().find_map(|s| if let Sel::Object(_, id) = s { Some(*id) } else { None }).unwrap();
        assert!(t.attach_objects.iter().any(|a| a.parent_id == Some(new_obj) && a.file == "sign.sco"));
        assert!(t.attach_objects.iter().any(|a| a.parent_id == Some(row.id) && a.file == "bulb.sco"));
        let ids: std::collections::BTreeSet<i64> = t.attach_objects.iter().map(|a| a.id).collect();
        assert_eq!(ids.len(), 4, "every hanging object has its own IDCode");
    }

    /// A clip kept as a tile file and read back pastes the same.
    #[test]
    fn a_clip_survives_a_tile_file() {
        let make = || {
            let mut doc = flat_map(1);
            add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
            add_spline(&mut doc, (0, 0), 2, 90.0, 80.0, 0.0, 20.0, 0, 3);
            add_spline(&mut doc, (0, 0), 3, 90.0, 100.0, 0.0, 20.0, 2, 0);
            let t = doc.tiles.get_mut(&(0, 0)).unwrap();
            t.tile.spline_attachments.push(SplineAttachment { file: "lamp.sco".into(), id: 5, spline_index: 1, interval: 10.0, range: 20.0, ..Default::default() });
            t.tile.attach_objects.push(MapObject { file: "sign.sco".into(), id: 6, parent_id: Some(1), ..Default::default() });
            doc
        };
        let mut doc = make();
        let clip = copy(&doc, &[Sel::Object((0, 0), 1), Sel::Spline((0, 0), 2), Sel::Spline((0, 0), 3)], |_| false);
        let dir = std::env::temp_dir().join(format!("openmapper-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("group.map");
        std::fs::write(&f, crate::doc::save::utf16(&crate::doc::save::tile_text(&clip.to_tile()))).unwrap();
        let back = Clip::from_tile(&omsi_map::Tile::load(&f).unwrap(), |_| false);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(back.len(), 3);
        let at = DVec3::new(150.0, 150.0, 0.0);
        let mut a = make();
        let got_a = paste(&mut a, &clip, at);
        let got_b = paste(&mut doc, &back, at);
        assert_eq!(got_a.len(), got_b.len());
        for (x, y) in got_a.iter().zip(&got_b) {
            match (x, y) {
                (Sel::Object(k, i), Sel::Object(l, j)) => assert!((a.object(*k, *i).unwrap().pos[0] - doc.object(*l, *j).unwrap().pos[0]).abs() < 1e-6),
                (Sel::Spline(k, i), Sel::Spline(l, j)) => assert!(a.spline(*k, *i).unwrap().pos.iter().zip(doc.spline(*l, *j).unwrap().pos).all(|(p, q)| (p - q).abs() < 1e-6)),
                _ => panic!(),
            }
        }
        // the row and the sign came along
        assert_eq!(doc.tiles[&(0, 0)].tile.spline_attachments.len(), 2);
        assert_eq!(doc.tiles[&(0, 0)].tile.attach_objects.len(), 2);
    }

    #[test]
    fn what_falls_off_the_map_is_left_out() {
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 10.0, 10.0);
        add_object(&mut doc, (0, 0), 2, 290.0, 10.0);
        let clip = copy(&doc, &[Sel::Object((0, 0), 1), Sel::Object((0, 0), 2)], |_| false);
        // the middle at 150: moved 100 east, the second goes off the map's east edge
        let got = paste(&mut doc, &clip, clip.at + DVec3::new(100.0, 0.0, 0.0));
        assert_eq!(got.len(), 1);
        assert_eq!(doc.tiles[&(0, 0)].tile.objects.len(), 3);
    }

    #[test]
    fn heights_follow_the_ground_where_it_is_pasted() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 2, 50.0, 50.0, 0.0, 20.0, 0, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].pos[2] = 0.5;
        let clip = copy(&doc, &[Sel::Spline((0, 0), 2)], |_| false);
        let got = paste(&mut doc, &clip, clip.at + DVec3::new(0.0, 100.0, 3.0));
        let Sel::Spline(k, id) = got[0] else { panic!() };
        assert!((doc.spline(k, id).unwrap().pos[2] - 3.5).abs() < 1e-9);
    }

    /// A junction copied with its signal heads: the copies' lights follow the copied
    /// junction, not the original (which would also take them away when it is deleted).
    #[test]
    fn signal_heads_pasted_with_their_junction_follow_the_copy() {
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
        add_object(&mut doc, (0, 0), 2, 110.0, 100.0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects[1].var_parent = Some(1);
        let clip = copy(&doc, &[Sel::Object((0, 0), 2), Sel::Object((0, 0), 1)], |_| false);
        let got = paste(&mut doc, &clip, clip.at + DVec3::new(0.0, 50.0, 0.0));
        let (Sel::Object(_, head), Sel::Object(_, junction)) = (got[0], got[1]) else { panic!() };
        assert_eq!(doc.object((0, 0), head).unwrap().var_parent, Some(junction));
        assert_eq!(crate::junction::children(&doc, 1), vec![((0, 0), 2)]);
    }
}
