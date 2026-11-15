//! Bus pockets: a stretch of a road given another road type (one made with a pocket lane, see
//! `RoadSpec::pocket`), so buses can stop beside the through lanes. The stretch is cut out of
//! the road where it was pressed; the road types at its two ends differ in width, and the
//! editor's width adapter lays the tapering plates there (with their lanes, so traffic
//! flows through the bay and on).

use glam::DVec2;
use omsi_geometry::SplineCurve;

use crate::doc::{tile_origin, Document, TileKey};
use crate::i18n::tr;

/// The pieces of a road chain with where each starts along it.
type Chain = Vec<(TileKey, i64, f64)>;

/// The chain through spline `id`, walked both ways along its links while the neighbours run
/// the same way: its pieces in order, each with the distance from the chain's start.
fn chain_of(doc: &Document, k: TileKey, id: i64) -> Chain {
    // back to the first piece
    let (mut first, mut guard) = ((k, id), 0);
    while guard < 64 {
        guard += 1;
        let Some(s) = doc.spline(first.0, first.1) else { break };
        match doc.find_spline(s.prev_id) {
            Some((pk, p)) if p.next_id == s.id && p.id != id => first = (pk, p.id),
            _ => break,
        }
        if first.1 == id {
            break;
        }
    }
    let out = walk(doc, first);
    // a closed road (a ring): its chain has no end, so it starts on the far side from the
    // click, and a bay near where the walk began is no bay at the end of a road
    let closed = out.last().and_then(|(ck, cid, _)| doc.spline(*ck, *cid)).map(|l| l.next_id == out[0].1).unwrap_or(false) && out.len() > 1;
    if closed {
        let total = out.last().and_then(|(ck, cid, at)| doc.spline(*ck, *cid).map(|s| at + s.length)).unwrap_or(0.0);
        let clicked = out.iter().find(|c| c.1 == id).map(|c| c.2).unwrap_or(0.0);
        // (the piece that begins nearest half the ring away from the clicked one)
        let far = (clicked + total / 2.0).rem_euclid(total);
        if let Some(start) = out.iter().min_by(|a, b| (a.2 - far).abs().total_cmp(&(b.2 - far).abs())) {
            return walk(doc, (start.0, start.1));
        }
    }
    out
}

/// The pieces from `first` on along the links, each with where it begins.
fn walk(doc: &Document, first: (TileKey, i64)) -> Chain {
    let mut out: Chain = Vec::new();
    let (mut cur, mut at) = (Some(first), 0.0);
    while let Some((ck, cid)) = cur {
        if out.iter().any(|c| c.1 == cid) {
            break;
        }
        let Some(s) = doc.spline(ck, cid) else { break };
        out.push((ck, cid, at));
        at += s.length;
        cur = match doc.find_spline(s.next_id) {
            Some((nk, n)) if n.prev_id == s.id => Some((nk, n.id)),
            _ => None,
        };
    }
    out
}

/// The road's chain with the stretch `[from, to]` (distances along the chain) cut out.
pub struct Cut {
    /// The pieces of the stretch, in order.
    pub inner: Vec<(TileKey, i64)>,
    /// The pieces beside it that stay (None: the stretch reaches the chain's end).
    pub before: Option<(TileKey, i64)>,
    pub after: Option<(TileKey, i64)>,
}

/// Cut the chain through `(k, id)` at `from` and `to` along the chain. The chain must be
/// long enough on each side of the click for the stretch to fit.
pub fn cut_out(doc: &mut Document, k: TileKey, id: i64, from: f64, to: f64) -> Result<Cut, &'static str> {
    let chain = chain_of(doc, k, id);
    let total = chain.last().and_then(|(ck, cid, at)| doc.spline(*ck, *cid).map(|s| at + s.length)).unwrap_or(0.0);
    if from < -0.01 || to > total + 0.01 || to - from < 2.0 {
        return Err(tr("The road is too short here for a pocket of that length", "Дорога здесь слишком короткая для кармана такой длины"));
    }
    // (the later cut first: the earlier distance stays where it was)
    for pos in [to, from] {
        let chain = chain_of(doc, k, id);
        let Some((ck, cid, at)) = chain.iter().rev().find(|c| c.2 <= pos + 1e-6).copied() else { continue };
        let len = doc.spline(ck, cid).map(|s| s.length).unwrap_or(0.0);
        let local = pos - at;
        if local > 0.5 && local < len - 0.5 {
            doc.split_spline(ck, cid, local).ok_or(tr("The road cannot be cut there", "Здесь дорогу нельзя разрезать"))?;
        }
    }
    let chain = chain_of(doc, k, id);
    let mut cut = Cut { inner: Vec::new(), before: None, after: None };
    for (ck, cid, at) in &chain {
        let len = doc.spline(*ck, *cid).map(|s| s.length).unwrap_or(0.0);
        if at + len <= from + 0.6 {
            cut.before = Some((*ck, *cid));
        } else if *at >= to - 0.6 {
            if cut.after.is_none() {
                cut.after = Some((*ck, *cid));
            }
        } else {
            cut.inner.push((*ck, *cid));
        }
    }
    if cut.inner.is_empty() {
        return Err(tr("The road cannot be cut there", "Здесь дорогу нельзя разрезать"));
    }
    Ok(cut)
}

/// Where along the chain through `(k, id)` its piece `id` begins, and the chain's length.
pub fn chain_position(doc: &Document, k: TileKey, id: i64) -> Option<(f64, f64)> {
    let chain = chain_of(doc, k, id);
    let start = chain.iter().find(|c| c.1 == id)?.2;
    let last = chain.last()?;
    Some((start, last.2 + doc.spline(last.0, last.1)?.length))
}

/// Give the stretch its type, moved `d` m to the right of the road's way (negative: left),
/// the road's pieces linked through it as before. Gives back the stretch's new pieces.
pub fn retype(doc: &mut Document, cut: &Cut, file: &str, d: f64) -> Result<Vec<(TileKey, i64)>, &'static str> {
    let pieces: Vec<(TileKey, i64)> = if d.abs() < 0.05 {
        for (k, id) in &cut.inner {
            if let Some(t) = doc.tiles.get_mut(k) {
                if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == *id) {
                    s.file = file.to_string();
                }
                t.touch_content();
            }
        }
        cut.inner.clone()
    } else {
        // the stretch copied alongside; the originals go and the copies take their place
        let copies = crate::parallel::copy(doc, &cut.inner, d)?;
        let first_old = doc.spline(cut.inner[0].0, cut.inner[0].1).cloned();
        let last_old = doc.spline(cut.inner[cut.inner.len() - 1].0, cut.inner[cut.inner.len() - 1].1).cloned();
        for (k, id) in &cut.inner {
            doc.delete_spline(*k, *id);
        }
        for (k, id) in &copies {
            if let Some(t) = doc.tiles.get_mut(k) {
                if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == *id) {
                    s.file = file.to_string();
                }
                t.touch_content();
            }
        }
        // (the first and last copies take the links the originals had to the road beside)
        if let (Some(a), Some(b)) = (first_old, last_old) {
            let (fk, fid) = copies[0];
            let (lk, lid) = copies[copies.len() - 1];
            for (k, id, prev, next) in [(fk, fid, a.prev_id, 0), (lk, lid, 0, b.next_id)] {
                if let Some(t) = doc.tiles.get_mut(&k) {
                    if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == id) {
                        if prev != 0 {
                            s.prev_id = prev;
                        }
                        if next != 0 {
                            s.next_id = next;
                        }
                    }
                }
            }
            for (id, other, at_end) in [(fid, a.prev_id, false), (lid, b.next_id, true)] {
                if other == 0 {
                    continue;
                }
                let Some((ok, _)) = doc.find_spline(other) else { continue };
                if let Some(t) = doc.tiles.get_mut(&ok) {
                    if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == other) {
                        if at_end { s.prev_id = id } else { s.next_id = id }
                    }
                    t.touch_content();
                }
            }
        }
        copies
    };
    Ok(pieces)
}

/// The spline's outermost extent across it (left, right of its middle, m) from the profiles
/// it is drawn with.
pub fn extent(model: &crate::assets::SplineModel) -> Option<(f32, f32)> {
    let xs: Vec<f32> = model.def.profiles.iter().flat_map(|p| p.points.iter().map(|q| q.x)).collect();
    if xs.is_empty() {
        return None;
    }
    Some((xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max)))
}

/// The side of a road a bay is made on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// The kerb the road's own traffic keeps to, in the road type's frame: the side its lanes along
/// the road's way lie on (a mirrored spline runs them the other way, so its against lanes).
/// Where there are none to tell, `left_hand` says.
pub fn bay_side(model: &crate::assets::SplineModel, mirrored: bool, left_hand: bool) -> Side {
    let along = if mirrored { 1 } else { 0 };
    let xs: Vec<f32> = model.def.paths.iter().filter(|p| p.kind == 0 && p.direction == along).map(|p| p.start[0]).collect();
    if xs.is_empty() {
        return if left_hand != mirrored { Side::Left } else { Side::Right };
    }
    if xs.iter().sum::<f32>() >= 0.0 { Side::Right } else { Side::Left }
}

/// One `[tag]` of a road type's text and the lines that follow it.
struct Item {
    tag: String,
    lines: Vec<String>,
}

impl Item {
    /// The numbers of the lines that are one (index of the line, value).
    fn values(&self) -> Vec<(usize, f32)> {
        self.lines.iter().enumerate().filter_map(|(i, l)| Some((i, l.trim().parse::<f32>().ok()?))).collect()
    }

    fn set(&mut self, nth: usize, v: f32) {
        if let Some((i, _)) = self.values().get(nth).copied() {
            self.lines[i] = format!("{v:.3}");
        }
    }

    fn new(tag: &str, values: &[String]) -> Item {
        Item { tag: tag.to_string(), lines: values.to_vec() }
    }
}

fn items_of(text: &str) -> Vec<Item> {
    let mut items = vec![Item { tag: String::new(), lines: Vec::new() }];
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') && t.ends_with(']') {
            items.push(Item { tag: t.to_lowercase(), lines: Vec::new() });
        } else {
            items.last_mut().unwrap().lines.push(l.to_string());
        }
    }
    items
}

/// A road type's text made again with a bay of `width` m along its kerb on `side`: everything
/// beyond the roadway's edge on that side (footway, kerb, its lane) moved out by the width, and
/// a strip of the roadway's own asphalt, a height profile and a street lane (the nearest
/// lane's way) laid in the gap. The other side is left as it is. None where the text has no
/// roadway to widen.
pub fn bay_sli(text: &str, side: Side, width: f32) -> Option<String> {
    let mut items = items_of(text);
    let sign = if side == Side::Right { 1.0f32 } else { -1.0 };
    let eps = 0.01;
    // the roadway: the lowest height profiles; its edge on the bay's side
    let heights: Vec<(usize, f32, f32, f32)> = items.iter().enumerate().filter(|(_, i)| i.tag == "[heightprofile]").filter_map(|(ix, i)| {
        let v = i.values();
        (v.len() >= 3).then(|| (ix, v[0].1, v[1].1, v[2].1))
    }).collect();
    let road_h = heights.iter().map(|h| h.3).fold(f32::MAX, f32::min);
    let low = heights.iter().filter(|h| (h.3 - road_h).abs() < 1e-4);
    let edge = if side == Side::Right { low.map(|h| h.1.max(h.2)).fold(f32::MIN, f32::max) } else { low.map(|h| h.1.min(h.2)).fold(f32::MAX, f32::min) };
    if heights.is_empty() || edge.abs() > 60.0 {
        return None;
    }
    let beyond = |x: f32| if side == Side::Right { x >= edge - eps } else { x <= edge + eps };
    // the profiles with their points
    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for (ix, i) in items.iter().enumerate() {
        match i.tag.as_str() {
            "[profile]" => groups.push((ix, Vec::new())),
            "[profilepnt]" => {
                if let Some(g) = groups.last_mut() {
                    if g.1.last().map(|l| *l + 1 == ix).unwrap_or(ix == g.0 + 1) {
                        g.1.push(ix);
                    }
                }
            }
            _ => {}
        }
    }
    let point = |items: &Vec<Item>, ix: usize| -> Option<(f32, f32, f32, f32)> {
        let v = items[ix].values();
        (v.len() >= 4).then(|| (v[0].1, v[1].1, v[2].1, v[3].1))
    };
    // the profile that ends at the roadway's edge: its asphalt carries on into the bay
    let mut strip: Option<(usize, String, (f32, f32, f32, f32), f32)> = None;
    let mut moved: Vec<usize> = Vec::new();
    for (pix, pts) in &groups {
        let p: Vec<(f32, f32, f32, f32)> = pts.iter().filter_map(|i| point(&items, *i)).collect();
        if p.len() < 2 {
            continue;
        }
        if p.iter().all(|q| beyond(q.0)) {
            moved.extend(pts.iter().copied());
        } else if strip.is_none() {
            if let Some(at) = p.iter().find(|q| (q.0 - edge).abs() < eps) {
                let inner = p.iter().filter(|q| (q.0 - edge).abs() >= eps).map(|q| q.2).next().unwrap_or(at.2);
                let tex = items[*pix].values().first().map(|v| items[*pix].lines[v.0].clone()).unwrap_or_else(|| "0".into());
                // (the last point of the profile's last point: after it the strip goes in)
                strip = Some((*pts.last().unwrap(), tex, *at, (at.2 + inner) / 2.0));
            }
        }
    }
    let (after_profile, tex, at, u_mid) = strip?;
    for ix in moved {
        let v = items[ix].values();
        let x = v[0].1;
        items[ix].set(0, x + sign * width);
    }
    // the footway's (and anything else's) height profiles and lanes beyond the edge
    let mut lanes: Vec<(usize, f32, f32, i32)> = Vec::new();
    for (ix, i) in items.iter_mut().enumerate() {
        match i.tag.as_str() {
            "[heightprofile]" => {
                let v = i.values();
                if v.len() >= 2 && beyond(v[0].1) && beyond(v[1].1) {
                    i.set(0, v[0].1 + sign * width);
                    i.set(1, v[1].1 + sign * width);
                }
            }
            "[path]" => {
                let v = i.values();
                if v.len() >= 5 {
                    if beyond(v[1].1) {
                        i.set(1, v[1].1 + sign * width);
                    } else if v[0].1 as i32 == 0 {
                        lanes.push((ix, v[1].1, v[2].1, v[4].1 as i32));
                    }
                }
            }
            _ => {}
        }
    }
    // the bay's lane goes the way of the street lane beside it
    let near = lanes.iter().max_by(|a, b| (a.1 * sign).total_cmp(&(b.1 * sign)))?;
    let n = |v: f32| format!("{v:.3}");
    let (a, b) = if side == Side::Right { (edge, edge + width) } else { (edge - width, edge) };
    let mut add: Vec<(usize, Vec<Item>)> = Vec::new();
    add.push((
        items.iter().rposition(|i| i.tag == "[heightprofile]")?,
        vec![Item::new("[heightprofile]", &[n(a), n(b), n(road_h), n(road_h), String::new()])],
    ));
    add.push((
        after_profile,
        vec![
            Item::new("[profile]", &[tex, String::new()]),
            Item::new("[profilepnt]", &[n(a), n(at.1), n(u_mid - 0.01), n(at.3), String::new()]),
            Item::new("[profilepnt]", &[n(b), n(at.1), n(u_mid + 0.01), n(at.3), String::new()]),
        ],
    ));
    add.push((
        items.iter().rposition(|i| i.tag == "[path]")?,
        vec![Item::new("[path]", &["0".into(), n((a + b) / 2.0), n(near.2), n(width * 0.9), near.3.to_string(), String::new()])],
    ));
    // (from the back, so the places stay where they were)
    add.sort_by(|x, y| y.0.cmp(&x.0));
    for (ix, new) in add {
        for (k, it) in new.into_iter().enumerate() {
            items.insert(ix + 1 + k, it);
        }
    }
    for i in items.iter_mut().filter(|i| i.tag == "[friendlyname]") {
        if let Some(l) = i.lines.iter_mut().find(|l| !l.trim().is_empty()) {
            *l = format!("{} bay {:.1} m", l.trim(), width);
        }
    }
    Some(items_text(&items))
}

fn items_text(items: &[Item]) -> String {
    let mut out = String::new();
    for i in items {
        if !i.tag.is_empty() {
            out.push_str(&i.tag);
            out.push('\n');
        }
        for l in &i.lines {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// The road type of a bay on `main_rel`'s road (made in the content folder as
/// `Splines\openMapper\<name>_bay_<side>_<width>_<id>.sli`, with its own textures beside it): the road
/// type's own text with the bay along the kerb its traffic keeps to. Gives back its path and
/// the side. Made once: the same bay is found again.
pub fn bay_type(assets: &mut crate::assets::Assets, content: &std::path::Path, main_rel: &str, mirrored: bool, left_hand: bool, width: f32) -> Result<(String, Side), String> {
    let main = assets.spline_wait(main_rel, 3000).ok_or_else(|| "the road type could not be read".to_string())?;
    let side = bay_side(&main, mirrored, left_hand);
    let name = bay_name(main_rel, &format!("{}_{width:.1}m", side.name()), width);
    write_bay_type(assets, content, main_rel, &main, &name, |text| bay_sli(text, side, width)).map(|rel| (rel, side))
}

/// Copy a texture family under a name owned by this bay type.
fn bay_texture_name(tex: &std::path::Path, dir: &std::path::Path, bay: &str, index: usize) -> std::io::Result<String> {
    const SEASONS: [&str; 6] = ["Spring", "SummerDry", "Fall", "Winter", "WinterSnow", "WinterSnowfall"];
    let stem = tex.file_stem().unwrap_or_default().to_string_lossy();
    let new_stem = format!("{bay}_{index}_{}", file_stem(&stem));
    let extension = tex.extension().map(|ext| ext.to_string_lossy());
    let name = extension.map_or_else(|| new_stem.clone(), |ext| format!("{new_stem}.{ext}"));
    let file = tex.file_name().unwrap_or_default().to_string_lossy();
    if let Some(parent) = tex.parent() {
        let root = parent.parent().filter(|_| parent.file_name().is_some_and(|name| {
            SEASONS.iter().any(|s| name.eq_ignore_ascii_case(s))
        })).unwrap_or(parent);
        let base = crate::doc::safe::own_path(root, &file);
        copy_bay_texture(&base, dir, &stem, &new_stem)?;
        for season in SEASONS {
            let from = crate::doc::safe::own_path(root, season);
            let to = crate::doc::safe::own_path(dir, season);
            let variant = crate::doc::safe::own_path(&from, &file);
            copy_bay_texture(&variant, &to, &stem, &new_stem)?;
        }
    }
    Ok(name)
}

fn copy_bay_texture(tex: &std::path::Path, dir: &std::path::Path, stem: &str, new_stem: &str) -> std::io::Result<()> {
    for source in std::iter::once(tex.to_path_buf()).filter(|p| omsi_cfg::vfs::is_file(p)).chain(crate::junction::companions(tex)) {
        let name = source.file_name().unwrap_or_default().to_string_lossy();
        let Some(suffix) = name.get(stem.len()..) else { continue };
        let target = crate::doc::safe::own_path(dir, &format!("{new_stem}{suffix}"));
        if !target.is_file() {
            std::fs::create_dir_all(dir)?;
            crate::doc::safe::write_atomic(&target, &omsi_cfg::vfs::read(&source)?)?;
        }
    }
    Ok(())
}

fn file_stem(s: &str) -> String {
    s.chars().take(48).map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn bay_name(main_rel: &str, suffix: &str, width: f32) -> String {
    use sha2::{Digest, Sha256};
    let rel = main_rel.replace('\\', "/");
    let canonical_rel = rel.to_lowercase();
    let stem = std::path::Path::new(&canonical_rel).file_stem().unwrap_or_default().to_string_lossy();
    let mut hash = Sha256::new();
    hash.update(canonical_rel.as_bytes());
    hash.update(width.to_le_bytes());
    let id = u64::from_be_bytes(hash.finalize()[..8].try_into().unwrap());
    format!("{}_bay_{suffix}_{id:016x}", file_stem(&stem))
}

fn write_bay_type(assets: &crate::assets::Assets, content: &std::path::Path, main_rel: &str, main: &crate::assets::SplineModel, name: &str, widen: impl FnOnce(&str) -> Option<String>) -> Result<String, String> {
    // The spline is published after its textures. Serialize two editor operations on the same output.
    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let rel = format!("Splines\\openMapper\\{name}.sli");
    let path = crate::doc::safe::own_path(content, &rel);
    if path.is_file() {
        return Ok(rel);
    }
    let text = omsi_cfg::vfs::read_text(&assets.resolve(main_rel)).map_err(|e| e.to_string())?;
    let sli = widen(&text).ok_or_else(|| "the road type has no roadway to widen".to_string())?;
    let tex_dir = crate::doc::safe::own_path(content, "Splines/openMapper/texture");
    std::fs::create_dir_all(&tex_dir).map_err(|e| e.to_string())?;
    let mut names = std::collections::HashMap::new();
    for (index, (definition, material)) in main.def.textures.iter().zip(&main.materials).enumerate() {
        if let Some(source) = &material.texture {
            let key = definition.file.trim().replace('\\', "/").to_lowercase();
            if let std::collections::hash_map::Entry::Vacant(entry) = names.entry(key) {
                entry.insert(bay_texture_name(source, &tex_dir, name, index).map_err(|e| e.to_string())?);
            }
        }
    }
    let mut items = items_of(&sli);
    for item in items.iter_mut().filter(|i| matches!(i.tag.as_str(), "[texture]" | "[matl]")) {
        if let Some(line) = item.lines.iter_mut().find(|l| !l.trim().is_empty()) {
            if let Some(name) = names.get(&line.trim().replace('\\', "/").to_lowercase()) {
                *line = name.clone();
            }
        }
    }
    let encoded = omsi_cfg::encode_text(&text_crlf(&items_text(&items)));
    crate::doc::safe::write_atomic(&path, &encoded).map_err(|e| e.to_string())?;
    Ok(rel)
}

fn text_crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// The road nearest `p` (within `within` m of its middle) that `wanted` accepts by file name:
/// tile, id and the distance along it.
pub fn road_near(doc: &Document, p: DVec2, within: f64, wanted: &mut dyn FnMut(&str) -> bool) -> Option<(TileKey, i64, f64)> {
    let mut best: Option<(f64, (TileKey, i64, f64))> = None;
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        for s in t.tile.splines.iter().filter(|s| !s.deleted && wanted(&s.file)) {
            if (DVec2::new(org.x + s.pos[0], org.y + s.pos[1]) - p).length() > s.length + within {
                continue;
            }
            let c = SplineCurve::from_map(s, org);
            let n = (s.length / 0.25).ceil().max(2.0) as usize;
            for i in 0..=n {
                let at = s.length * i as f64 / n as f64;
                let d = (c.point_at(at).truncate() - p).length();
                if d < within && best.map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, (*k, s.id, at)));
                }
            }
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::tests::{add_spline, flat_map};

    fn files(doc: &Document, ids: &[(TileKey, i64)]) -> Vec<String> {
        ids.iter().filter_map(|(k, id)| doc.spline(*k, *id)).map(|s| s.file.clone()).collect()
    }

    /// A stretch cut out of a road of two pieces keeps the road linked through it, and takes
    /// the new type.
    #[test]
    fn a_stretch_is_cut_out_and_retyped() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 50.0, 0.0, 50.0, 0, 2);
        add_spline(&mut doc, (0, 0), 2, 100.0, 100.0, 0.0, 50.0, 1, 0);
        // 30 m, from 35 to 65 along the road: across the joint
        let cut = cut_out(&mut doc, (0, 0), 1, 35.0, 65.0).unwrap();
        assert!(cut.before.is_some() && cut.after.is_some());
        assert_eq!(cut.inner.len(), 2);
        let total: f64 = cut.inner.iter().map(|(k, id)| doc.spline(*k, *id).unwrap().length).sum();
        assert!((total - 30.0).abs() < 1e-6);
        let inner = retype(&mut doc, &cut, "Splines\\pocket.sli", 0.0).unwrap();
        assert!(files(&doc, &inner).iter().all(|f| f == "Splines\\pocket.sli"));
        // the road before and after keeps its type, and all of it is still one chain
        assert_eq!(doc.spline((0, 0), cut.before.unwrap().1).unwrap().file, "Splines\\road.sli");
        let chain = chain_of(&doc, (0, 0), 1);
        assert_eq!(chain.len(), 4);
        let sum: f64 = chain.iter().map(|(k, id, _)| doc.spline(*k, *id).unwrap().length).sum();
        assert!((sum - 100.0).abs() < 1e-6);
    }

    /// Moved sideways, the stretch lies the distance off the road's middle and stays joined.
    #[test]
    fn a_stretch_moved_aside_stays_joined() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 50.0, 0.0, 100.0, 0, 0);
        let cut = cut_out(&mut doc, (0, 0), 1, 30.0, 60.0).unwrap();
        let inner = retype(&mut doc, &cut, "Splines\\pocket.sli", 1.5).unwrap();
        assert_eq!(inner.len(), 1);
        let s = doc.spline(inner[0].0, inner[0].1).unwrap();
        // heading north: right is east
        assert!((s.pos[0] - 101.5).abs() < 1e-9 && (s.pos[1] - 80.0).abs() < 1e-9, "{:?}", s.pos);
        let before = cut.before.unwrap();
        assert_eq!(doc.spline(before.0, before.1).unwrap().next_id, inner[0].1);
        assert_eq!(s.prev_id, before.1);
        let after = cut.after.unwrap();
        assert_eq!(s.next_id, after.1);
        assert_eq!(doc.spline(after.0, after.1).unwrap().prev_id, inner[0].1);
    }

    /// A ring has no end: a bay near where its walk would have begun is cut all the same, with
    /// road on both sides.
    #[test]
    fn a_bay_on_a_ring_has_road_either_side_wherever_it_is() {
        for click in 1..=4 {
            let mut doc = flat_map(1);
            for id in 1..=4 {
                add_spline(&mut doc, (0, 0), id, 100.0 + 100.0 * id as f64, 50.0, 0.0, 100.0, if id == 1 { 4 } else { id - 1 }, if id == 4 { 1 } else { id + 1 });
            }
            let (start, total) = chain_position(&doc, (0, 0), click).unwrap();
            assert!((total - 400.0).abs() < 1e-6);
            // (the clicked piece lies in the middle of the walk, not at its start)
            assert!(start > 100.0 && start < 300.0, "{click}: {start}");
            let cut = cut_out(&mut doc, (0, 0), click, start + 5.0, start + 35.0).unwrap();
            assert!(cut.before.is_some() && cut.after.is_some(), "{click}");
        }
    }

    fn extremes(text: &str) -> ((f32, f32), usize, usize) {
        let items = items_of(text);
        let xs: Vec<f32> = items.iter().filter(|i| i.tag == "[profilepnt]").filter_map(|i| i.values().first().map(|v| v.1)).collect();
        let n = |t: &str| items.iter().filter(|i| i.tag == t).count();
        ((xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max)), n("[heightprofile]"), n("[path]"))
    }

    /// A road type's text with a bay moves what lies past its roadway's edge on one side only,
    /// and lays the bay's strip, height and lane in the gap.
    #[test]
    fn a_bay_widens_one_side_of_the_text_and_leaves_the_other() {
        let tex = std::env::temp_dir().join("om-bay-text.png");
        let spec = crate::road_maker::RoadSpec { name: "T".into(), roadway_tex: tex.clone(), footway_tex: tex, footway: 2.0, ..Default::default() };
        let text = spec.sli();
        let ((lo, hi), heights, paths) = extremes(&text);
        for (side, sign) in [(Side::Right, 1.0f32), (Side::Left, -1.0)] {
            let out = bay_sli(&text, side, 3.0).unwrap();
            let ((l2, h2), heights2, paths2) = extremes(&out);
            if sign > 0.0 {
                assert!((l2 - lo).abs() < 1e-3 && (h2 - hi - 3.0).abs() < 1e-3, "{l2} {h2} from {lo} {hi}");
            } else {
                assert!((h2 - hi).abs() < 1e-3 && (l2 - lo + 3.0).abs() < 1e-3, "{l2} {h2} from {lo} {hi}");
            }
            assert_eq!((heights2, paths2), (heights + 1, paths + 1));
            // (a lane for the bay, 3 m wide, with the road's own traffic: out beyond the lanes)
            assert!(out.contains("2.700"));
        }
    }

    #[test]
    fn junction_bus_bay_owns_its_textures_and_preserves_manual_changes() {
        let dir = std::env::temp_dir().join(format!("om-bay-textures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let content = dir.join("content");
        let omsi = dir.join("omsi");
        let mut assets = crate::assets::Assets::new(omsi.clone(), content.clone());
        let mut outputs = Vec::new();
        for (pack, red) in [("One", 64), ("Two", 192)] {
            let source = omsi.join("Splines").join(pack);
            let variants = source.join("texture").join("WinterSnow");
            std::fs::create_dir_all(&variants).unwrap();
            let texture = source.join("texture").join("road.png");
            image::RgbaImage::from_pixel(2, 2, image::Rgba([red, 32, 16, 255])).save(&texture).unwrap();
            image::RgbaImage::from_pixel(2, 2, image::Rgba([255, red, 255, 255])).save(variants.join("road.png")).unwrap();
            std::fs::write(source.join("texture").join("road.png.cfg"), b"texture settings").unwrap();
            std::fs::write(source.join("texture").join("road.surf"), b"surface settings").unwrap();
            let spec = crate::road_maker::RoadSpec { name: "Same".into(), roadway_tex: texture.clone(), ..Default::default() };
            let text = format!("{}\n[matl]\nROAD.PNG\n0\n", spec.sli());
            std::fs::write(source.join("Same.sli"), omsi_cfg::encode_text(&text)).unwrap();
            let main = format!("Splines\\{pack}\\Same.sli");
            let rel = bay_type(&mut assets, &content, &main, false, false, 3.0).unwrap().0;
            let path = crate::doc::safe::own_path(&content, &rel);
            let generated = omsi_cfg::vfs::read_text(&path).unwrap();
            let def = omsi_scenery::Spline::load(&path).unwrap();
            let name = &def.textures[0].file;
            let tex_dir = path.parent().unwrap().join("texture");
            let copied = tex_dir.join(name);
            let winter = tex_dir.join("WinterSnow").join(name);
            let copied_cfg = tex_dir.join(format!("{name}.cfg"));
            let surface = copied.with_extension("surf");
            println!("Original {main}\n{text}\nGenerated {rel}\n{generated}\nCopied: {copied:?}, {winter:?}, {copied_cfg:?}, {surface:?}");
            assert_ne!(name, "road.png");
            assert_eq!(image::open(&copied).unwrap().to_rgba8().get_pixel(0, 0).0, [red, 32, 16, 255]);
            assert_eq!(image::open(&winter).unwrap().to_rgba8().get_pixel(0, 0).0, [255, red, 255, 255]);
            assert_eq!(std::fs::read(copied_cfg).unwrap(), b"texture settings");
            assert_eq!(std::fs::read(surface).unwrap(), b"surface settings");
            assert!(generated.replace("\r\n", "\n").contains(&format!("[matl]\n{name}\n")));
            assert_eq!(omsi_cfg::vfs::read_text(&source.join("Same.sli")).unwrap(), text);
            outputs.push((main, rel, path, copied, generated, name.clone()));
        }

        assert_ne!(outputs[0].1, outputs[1].1);
        assert_ne!(outputs[0].5, outputs[1].5);
        let (main, rel, path, copied, generated, name) = &outputs[0];
        let edited = generated.replace(name, "custom-paving.png");
        std::fs::write(path, edited.as_bytes()).unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 40, 60, 255])).save(copied).unwrap();
        let repeated = bay_type(&mut assets, &content, main, false, false, 3.0).unwrap().0;
        let saved = omsi_cfg::vfs::read_text(path).unwrap();
        println!("After editing and placing the same bay again ({repeated})\n{saved}");
        assert_eq!(&repeated, rel);
        assert_eq!(saved, edited);
        assert_eq!(image::open(copied).unwrap().to_rgba8().get_pixel(0, 0).0, [20, 40, 60, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn too_short_a_road_is_refused() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 50.0, 0.0, 20.0, 0, 0);
        assert!(cut_out(&mut doc, (0, 0), 1, -5.0, 25.0).is_err());
        assert_eq!(doc.tiles[&(0, 0)].tile.splines.len(), 1);
    }
}
