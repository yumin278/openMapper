//! Complex and custom junctions: a junction drawn from above out of small splines (asphalt,
//! road markings, footways, kerbs, whatever the catalogue has), each laid along a line or an
//! arc.
//!
//! A *complex junction* is put on the map as those splines, as they are: nothing about it is
//! automatic, every piece an ordinary spline that can be selected and changed afterwards.
//!
//! A *custom junction* is the same drawing with its arms marked (where a road comes in, which
//! way it faces, of what type). It is written as a scenery object with `[splinehelper]`s at the
//! arms and a `[path]` along every lane of its pieces, as the junction plates openMapper makes
//! are, and is kept as a layout beside the content (`openMapper\junctions\<name>.json`). The
//! Roads tool's automatic junctions use such a layout where roads meet the way it was drawn for:
//! the same number of arms, facing about the same ways; the roads are cut back to it and joined
//! to its arms.
//!
//! Layout coordinates are metres in the layout's own frame: x east, y north, headings in degrees
//! clockwise from north, as on the map.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glam::DVec2;
use omsi_map::MapSpline;
use serde::{Deserialize, Serialize};

use crate::assets::{Assets, SplineModel};
use crate::doc::{tile_of, tile_origin, Document, TileKey};
use crate::i18n::tr;
use crate::roads::{self, Seg};

/// One arc or straight of a piece, in the layout's frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LSeg {
    pub x: f64,
    pub y: f64,
    pub heading: f64,
    /// 0 straight, > 0 turns right (m).
    pub radius: f64,
    pub length: f64,
}

impl LSeg {
    pub fn seg(&self) -> Seg {
        Seg { start: DVec2::new(self.x, self.y), heading: self.heading, length: self.length, radius: self.radius }
    }

    pub fn of(s: &Seg) -> LSeg {
        LSeg { x: s.start.x, y: s.start.y, heading: s.heading, radius: s.radius, length: s.length }
    }
}

/// A small spline laid along one or more segments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Piece {
    /// The spline type (`Splines\...sli`).
    pub file: String,
    pub segs: Vec<LSeg>,
    /// Lifted above the plate (m): markings lie a little over the asphalt they are painted on.
    #[serde(default)]
    pub lift: f64,
    /// Switched off in the editor's layers: it is not put on the map or written.
    #[serde(default)]
    pub hidden: bool,
    /// Made by Auto-Junction from the arms: kept in step with them, never edited by hand.
    #[serde(default)]
    pub auto: bool,
}

/// A scenery object stood in the drawing (a lamp, a bin, a bus shelter), in the layout's frame.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LObject {
    /// The scenery object (`Sceneryobjects\...sco`).
    pub file: String,
    pub x: f64,
    pub y: f64,
    /// Height above the plate (m).
    #[serde(default)]
    pub z: f64,
    /// Degrees clockwise from north.
    #[serde(default)]
    pub heading: f64,
}

/// A place a road joins: its position, the way it faces out of the junction and its type.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Arm {
    pub x: f64,
    pub y: f64,
    /// The heading pointing away from the junction along the road.
    pub out: f64,
    /// The road type that joins here (empty: any).
    #[serde(default)]
    pub road: String,
    /// Found by the editor from a road's free end (Auto-Junction), not placed by hand.
    #[serde(default)]
    pub auto: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub name: String,
    pub pieces: Vec<Piece>,
    pub arms: Vec<Arm>,
    /// What Auto-Junction builds from the arms.
    pub build: AutoBuild,
    /// Objects stood on the drawing: put on the map with it, as ordinary objects.
    pub objects: Vec<LObject>,
    /// Used where the Roads tool makes junctions by itself, when the roads meet as the arms do.
    pub auto: bool,
    /// The scenery object it was written as (`Sceneryobjects\...sco`), once saved as custom.
    pub baked: Option<String>,
}

/// What Auto-Junction builds between the arms of a drawing (each switchable).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoBuild {
    /// The junction's plate, as the Roads tool makes it: the roadway between the arms, kerbs
    /// rounded at each corner and the footways round them, with lanes through it.
    pub plate: bool,
    /// A turn of the arm's road type between each two neighbouring arms (a biarc).
    pub turns: bool,
    /// A rounded corner of `radius` where the two roads' middle lines meet instead of a turn.
    pub corners: bool,
    /// A piece straight on between arms facing each other.
    pub through: bool,
    /// The corner's radius (m).
    pub radius: f64,
}

impl Default for AutoBuild {
    fn default() -> Self {
        AutoBuild { plate: true, turns: false, corners: false, through: false, radius: 8.0 }
    }
}

/// The most the roads' headings may differ from the arms' for a layout to fit them (degrees).
pub const FIT_TOLERANCE: f64 = 30.0;

/// Where a layout's files are kept.
pub fn folder(content: &Path) -> PathBuf {
    content.join("openMapper").join("junctions")
}

pub fn file_stem(name: &str) -> String {
    crate::editable::safe_name(name)
}

impl Layout {
    pub fn save(&self, content: &Path) -> anyhow::Result<PathBuf> {
        let dir = folder(content);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", file_stem(&self.name)));
        crate::doc::safe::write_atomic(&path, serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(path)
    }

    /// Every layout saved in the content folder, by name.
    pub fn load_all(content: &Path) -> Vec<Layout> {
        let Ok(rd) = std::fs::read_dir(folder(content)) else { return Vec::new() };
        let mut out: Vec<Layout> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
            .filter_map(|p| serde_json::from_str::<Layout>(&std::fs::read_to_string(p).ok()?).ok())
            .collect();
        out.sort_by_key(|l| l.name.to_lowercase());
        out
    }

    /// How far from the layout's origin its pieces and arms reach (m).
    pub fn radius(&self) -> f64 {
        let seg_pts = self.pieces.iter().flat_map(|p| p.segs.iter()).flat_map(|s| [s.seg().point(0.0), s.seg().end()]);
        seg_pts.chain(self.arms.iter().map(|a| DVec2::new(a.x, a.y))).map(|p| p.length()).fold(0.0, f64::max)
    }
}

/// A point of the layout's frame on the map, the layout put at `origin` and turned `rot`
/// degrees clockwise.
pub fn to_world(p: DVec2, origin: DVec2, rot: f64) -> DVec2 {
    let r = rot.to_radians();
    origin + DVec2::new(p.x * r.cos() + p.y * r.sin(), -p.x * r.sin() + p.y * r.cos())
}

/// The arc through three points: from `a`, by `b`, to `c`. A straight when they lie on a line.
pub fn arc3(a: DVec2, b: DVec2, c: DVec2) -> Option<Seg> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-6 * (a - c).length_squared().max(1.0) {
        return roads::straight(a, c);
    }
    let (a2, b2, c2) = (a.length_squared(), b.length_squared(), c.length_squared());
    let ux = (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d;
    let uy = (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d;
    let centre = DVec2::new(ux, uy);
    let r = (a - centre).length();
    if r > 4000.0 {
        return roads::straight(a, c);
    }
    // (clockwise round the centre: a right turn, a positive radius)
    let cross = (b - a).perp_dot(c - b);
    let cw = cross < 0.0;
    let ang = |p: DVec2| (p.y - centre.y).atan2(p.x - centre.x);
    let tau = std::f64::consts::TAU;
    let sweep = if cw { (ang(a) - ang(c)).rem_euclid(tau) } else { (ang(c) - ang(a)).rem_euclid(tau) };
    let radial = a - centre;
    let velocity = if cw { DVec2::new(radial.y, -radial.x) } else { DVec2::new(-radial.y, radial.x) };
    Some(Seg { start: a, heading: roads::heading_of(velocity), length: r * sweep, radius: if cw { r } else { -r } })
}

/// The segments of a piece on the map.
fn world_segs(piece: &Piece, origin: DVec2, rot: f64) -> Vec<Seg> {
    piece.segs.iter().map(|s| Seg { start: to_world(DVec2::new(s.x, s.y), origin, rot), heading: roads::norm_deg(s.heading + rot), length: s.length, radius: s.radius }).collect()
}

/// The layout laid on the map as ordinary splines, linked piece by piece: its origin at
/// `origin` (the height `z` there), turned `rot` degrees. Gives back the splines made.
pub fn place(doc: &mut Document, layout: &Layout, origin: DVec2, z: f64, rot: f64) -> Result<Vec<(TileKey, i64)>, &'static str> {
    // (every tile it touches must be there: nothing is laid if one is not)
    for piece in layout.pieces.iter().filter(|p| !p.hidden) {
        for s in world_segs(piece, origin, rot) {
            for p in [s.start, s.end()] {
                if !doc.tiles.contains_key(&tile_of(p)) {
                    return Err(tr("The junction reaches past the map: add tiles there first", "Перекрёсток выходит за карту: сначала добавьте тайлы"));
                }
            }
        }
    }
    let mut made = Vec::new();
    for piece in layout.pieces.iter().filter(|p| !p.hidden) {
        let segs = world_segs(piece, origin, rot);
        let ids: Vec<i64> = segs.iter().map(|_| doc.new_id()).collect();
        let mut chain = 0.0;
        for (i, s) in segs.iter().enumerate() {
            let k = tile_of(s.start);
            let org = tile_origin(k);
            let sp = MapSpline {
                file: piece.file.clone(),
                id: ids[i],
                prev_id: if i > 0 { ids[i - 1] } else { 0 },
                next_id: ids.get(i + 1).copied().unwrap_or(0),
                pos: [s.start.x - org.x, s.start.y - org.y, z + piece.lift],
                heading: s.heading,
                length: s.length,
                radius: s.radius,
                tex_offset: chain,
                map_chain_offset: Some(chain),
                ..Default::default()
            };
            chain += s.length;
            let t = doc.tiles.get_mut(&k).unwrap();
            t.tile.splines.push(sp);
            t.touch_content();
            made.push((k, ids[i]));
        }
    }
    Ok(made)
}

/// The layout's objects as map objects at `origin` (height `z` there), turned `rot` degrees:
/// the tile each is in must be there (those that are not are left out). Gives back how many
/// were put and their places.
pub fn place_objects(doc: &mut Document, layout: &Layout, origin: DVec2, z: f64, rot: f64) -> Vec<(TileKey, i64)> {
    let mut made = Vec::new();
    for o in &layout.objects {
        let p = to_world(DVec2::new(o.x, o.y), origin, rot);
        let k = tile_of(p);
        let id = doc.new_id();
        let org = tile_origin(k);
        let Some(t) = doc.tiles.get_mut(&k) else { continue };
        t.tile.objects.push(omsi_map::MapObject { file: o.file.clone(), id, pos: [p.x - org.x, p.y - org.y, z + o.z], rot: [roads::norm_deg(o.heading + rot), 0.0, 0.0], ..Default::default() });
        t.touch_content();
        made.push((k, id));
    }
    made
}

/// The lanes of a piece as `[path]` records of the junction object (object frame).
fn path_records(piece: &Piece, model: &SplineModel, out: &mut String) {
    let num = |v: f64| {
        let s = format!("{v:.4}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s.is_empty() || s == "-0" { "0".to_string() } else { s.to_string() }
    };
    for seg in &piece.segs {
        let s = seg.seg();
        for p in &model.def.paths {
            let x = p.start[0] as f64;
            let start = s.start + roads::right(s.heading) * x;
            // (an arc keeps its centre: its radius grows or shrinks by the offset)
            let (radius, length) = if s.radius == 0.0 {
                (0.0, s.length)
            } else {
                let r = s.radius - x;
                if r == 0.0 || r.signum() != s.radius.signum() {
                    continue;
                }
                (r, s.length * r / s.radius)
            };
            let z = piece.lift + p.start[2] as f64;
            let _ = write!(out, "[path]\n{}\n{}\n{}\n{}\n{}\n{}\n0\n0\n{}\n{}\n{}\n0\n\n", num(start.x), num(start.y), num(z), num(s.heading), num(radius), num(length), p.kind, num(p.width as f64), p.direction);
        }
    }
}

/// The scenery object's extra lines: a helper at every arm and a path along every lane.
pub fn extra_lines(layout: &Layout, models: &dyn Fn(&str) -> Option<std::sync::Arc<SplineModel>>) -> String {
    let num = |v: f64| format!("{v:.4}");
    let mut s = String::new();
    for a in &layout.arms {
        // (OMSI's order: x, height, forward; the heading is the one a road arrives with
        // turned round, as in the plates openMapper makes)
        let _ = write!(s, "[splinehelper]\n{}\n{}\n{}\n{}\n{}\n0.1\n0\n\n", if a.road.is_empty() { layout.pieces.first().map(|p| p.file.as_str()).unwrap_or("") } else { &a.road }, num(a.x), num(0.0), num(a.y), num(roads::norm_deg(a.out + 180.0)));
    }
    for p in layout.pieces.iter().filter(|p| !p.hidden) {
        if let Some(m) = models(&p.file) {
            path_records(p, &m, &mut s);
        }
    }
    s
}

/// Which of the layout's spline types are roads (a junction piece for traffic) and how wide
/// (half, m), read from their models.
pub fn road_kinds(layout: &Layout, assets: &mut Assets) -> std::collections::HashMap<String, (bool, f64)> {
    let mut out = std::collections::HashMap::new();
    for p in &layout.pieces {
        out.entry(p.file.to_lowercase()).or_insert_with(|| assets.spline_wait(&p.file, 5000).map(|m| (m.joinable() && !m.is_track(), m.half_width as f64)).unwrap_or((false, 1.5)));
    }
    out
}

/// Write the layout as a scenery object (`custom`: a junction piece that automatic junctions
/// may use, with at least two arms; else any drawing as a plain object, arms or none) (its mesh swept from the pieces' splines, its helpers
/// and lanes) and keep the layout beside it. Gives back the object's relative path.
pub fn bake(roots: &crate::install::Roots, layout: &Layout, assets: &mut Assets, custom: bool) -> anyhow::Result<String> {
    let kinds = road_kinds(layout, assets);
    let resolved = layout.resolve(&|f| kinds.get(&f.to_lowercase()).map(|k| k.0).unwrap_or(false), &|f| kinds.get(&f.to_lowercase()).map(|k| k.1).unwrap_or(1.5));
    anyhow::ensure!(!layout.pieces.is_empty(), "{}", tr("draw some pieces first", "сначала нарисуйте детали"));
    anyhow::ensure!(!custom || layout.arms.len() >= 2, "{}", tr("mark the arms (where roads come in): at least two", "отметьте въезды (где входят дороги): не меньше двух"));
    let mut src = crate::editable::Source { name: layout.name.trim().to_string(), abs_height: true, ..Default::default() };
    let mut textures = std::collections::HashMap::new();
    let mut joined = openmapper_mesh::Mesh::default();
    let rel = layout.baked.clone().filter(|r| crate::editable::sco_path(roots, r).map(|p| p.exists()).unwrap_or(false)).unwrap_or_else(|| crate::editable::new_rel(roots, "Junctions", &if custom { format!("cj_{}", file_stem(&layout.name)) } else { file_stem(&layout.name) }));
    let (dir, _) = crate::editable::folder_of(roots, &rel);
    std::fs::create_dir_all(dir.join("texture"))?;
    // the plate between the arms: the roadway, kerbs and footways, with its lanes
    let mut plate_paths = String::new();
    let asphalt = crate::junction::asphalt_texture(&roots.omsi);
    for arms in &resolved.plates {
        for a in arms {
            anyhow::ensure!(assets.spline_wait(&a.file, 5000).is_some(), "{}: {}", tr("the spline cannot be read", "сплайн не читается"), a.file);
        }
        let plate = crate::junction::layout_plate(assets, arms, asphalt.as_deref()).ok_or_else(|| anyhow::anyhow!("{}", tr("the plate could not be made from these arms", "плиту не удалось построить из этих въездов")))?;
        joined.append(&plate.model_mesh(&mut src.model.materials, &mut textures), 0);
        plate_paths.push_str(&plate.paths);
    }
    for piece in resolved.pieces.iter().filter(|p| !p.hidden) {
        let Some(model) = assets.spline_wait(&piece.file, 5000) else {
            anyhow::bail!("{}: {}", tr("the spline cannot be read", "сплайн не читается"), piece.file);
        };
        for seg in &piece.segs {
            let ms = MapSpline { file: piece.file.clone(), pos: [seg.x, seg.y, piece.lift], heading: seg.heading, length: seg.length, radius: seg.radius, ..Default::default() };
            let Some(md) = crate::render::build_spline(&model, &ms, DVec2::ZERO) else { continue };
            let mesh = crate::app::mesh::mesh_of(&md, &model.materials, &mut src.model.materials, &mut textures);
            joined.append(&mesh, 0);
        }
    }
    anyhow::ensure!(!joined.is_empty(), "{}", tr("these splines have no shape to see", "у этих сплайнов нет видимой формы"));
    src.model.parts.push(openmapper_mesh::Part::new("plate", joined));
    let mut models = std::collections::HashMap::new();
    for p in resolved.pieces.iter().filter(|p| !p.hidden) {
        if let Some(m) = assets.spline_wait(&p.file, 5000) {
            models.insert(p.file.to_lowercase(), m);
        }
    }
    let mut cut = layout.clone();
    cut.pieces = resolved.pieces.clone();
    src.extra = extra_lines(&cut, &|f| models.get(&f.to_lowercase()).cloned());
    src.extra.push_str(&plate_paths);
    for (name, path) in &textures {
        let _ = name;
        crate::junction::copy_texture(path, &dir.join("texture"));
    }
    crate::editable::save(roots, &rel, &src)?;
    Ok(rel)
}

/// How well a layout's arms fit roads leaving a junction at these headings (degrees, with
/// the roads' types): the turn to give the layout, the road for each arm (index into
/// `roads`, in the arms' order) and the largest disagreement of a heading (degrees). None
/// when the numbers of arms differ.
pub fn fit(layout: &Layout, roads_out: &[(f64, String)], same_type: &dyn Fn(&str, &str) -> bool) -> Option<(f64, Vec<usize>, f64)> {
    let n = layout.arms.len();
    if n < 3 || n != roads_out.len() {
        return None;
    }
    // both sorted by heading, clockwise: the arms of the layout against the roads, round
    // every way
    let mut arm_order: Vec<usize> = (0..n).collect();
    arm_order.sort_by(|a, b| layout.arms[*a].out.rem_euclid(360.0).total_cmp(&layout.arms[*b].out.rem_euclid(360.0)));
    let mut road_order: Vec<usize> = (0..n).collect();
    road_order.sort_by(|a, b| roads_out[*a].0.rem_euclid(360.0).total_cmp(&roads_out[*b].0.rem_euclid(360.0)));
    let diff = |a: f64, b: f64| (a - b + 180.0).rem_euclid(360.0) - 180.0;
    let mut best: Option<(f64, Vec<usize>, f64)> = None;
    for shift in 0..n {
        // arm_order[i] meets road_order[(i + shift) % n]
        let pairs: Vec<(usize, usize)> = (0..n).map(|i| (arm_order[i], road_order[(i + shift) % n])).collect();
        if !pairs.iter().all(|(a, r)| layout.arms[*a].road.is_empty() || same_type(&layout.arms[*a].road, &roads_out[*r].1)) {
            continue;
        }
        // the turn: the mean of how far each road is turned from its arm
        let (mut sx, mut sy) = (0.0, 0.0);
        for (a, r) in &pairs {
            let d = diff(roads_out[*r].0, layout.arms[*a].out).to_radians();
            sx += d.cos();
            sy += d.sin();
        }
        let rot = roads::norm_deg(sy.atan2(sx).to_degrees());
        let err = pairs.iter().map(|(a, r)| diff(roads_out[*r].0, layout.arms[*a].out + rot).abs()).fold(0.0, f64::max);
        let mut by_arm = vec![0; n];
        for (a, r) in &pairs {
            by_arm[*a] = *r;
        }
        if best.as_ref().map(|b| err < b.2).unwrap_or(true) {
            best = Some((rot, by_arm, err));
        }
    }
    best.filter(|b| b.2 <= FIT_TOLERANCE)
}

// ------------------------------------------------------------ drawing helpers

impl LSeg {
    /// The segment mirrored across the y axis (`mirror`), then turned `rot` degrees clockwise
    /// about the origin; `about` is subtracted first and added back (a piece turned in place).
    pub fn transformed(&self, mirror: bool, rot: f64, about: DVec2) -> LSeg {
        let p = DVec2::new(self.x, self.y) - about;
        let (p, h, r) = if mirror { (DVec2::new(-p.x, p.y), roads::norm_deg(-self.heading), -self.radius) } else { (p, self.heading, self.radius) };
        let p = to_world(p, about, rot);
        LSeg { x: p.x, y: p.y, heading: roads::norm_deg(h + rot), radius: r, length: self.length }
    }
}

impl Piece {
    /// The piece the other way round: its segments reversed in order and each run backwards.
    pub fn reversed(&self) -> Piece {
        Piece { file: self.file.clone(), segs: self.segs.iter().rev().map(|s| LSeg::of(&roads::reverse(&s.seg()))).collect(), lift: self.lift, hidden: self.hidden, auto: self.auto }
    }

    /// Where it starts and the heading it starts with, where it ends and the heading it ends with.
    pub fn ends(&self) -> Option<(DVec2, f64, DVec2, f64)> {
        let (f, l) = (self.segs.first()?.seg(), self.segs.last()?.seg());
        Some((f.start, f.heading, l.end(), l.end_heading()))
    }

    /// How far `p` is from the piece's middle line (m).
    pub fn distance_to(&self, p: DVec2) -> f64 {
        let mut best = f64::MAX;
        for s in &self.segs {
            let seg = s.seg();
            let n = (s.length / 0.4).ceil().max(2.0) as usize;
            for k in 0..=n {
                best = best.min((seg.point(s.length * k as f64 / n as f64) - p).length());
            }
        }
        best
    }
}

impl Layout {
    /// The whole drawing mirrored across the y axis and/or turned `rot` degrees.
    pub fn transform(&mut self, mirror: bool, rot: f64) {
        for p in &mut self.pieces {
            for s in &mut p.segs {
                *s = s.transformed(mirror, rot, DVec2::ZERO);
            }
        }
        for a in &mut self.arms {
            let q = if mirror { DVec2::new(-a.x, a.y) } else { DVec2::new(a.x, a.y) };
            let q = to_world(q, DVec2::ZERO, rot);
            a.x = q.x;
            a.y = q.y;
            a.out = roads::norm_deg(if mirror { -a.out } else { a.out } + rot);
        }
    }

    /// The box the drawing fills: (low corner, high corner).
    pub fn bounds(&self) -> Option<(DVec2, DVec2)> {
        let pts: Vec<DVec2> = self
            .pieces
            .iter()
            .flat_map(|p| p.segs.iter())
            .flat_map(|s| {
                let seg = s.seg();
                let n = (s.length / 1.0).ceil().max(2.0) as usize;
                (0..=n).map(move |k| seg.point(s.length * k as f64 / n as f64))
            })
            .chain(self.arms.iter().map(|a| DVec2::new(a.x, a.y)))
            .collect();
        let first = *pts.first()?;
        Some(pts.iter().fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p))))
    }

    /// The ends of road pieces that no other road piece covers: where roads come in. `half`
    /// gives a spline type's half width.
    pub fn free_ends(&self, is_road: &dyn Fn(&str) -> bool, half: &dyn Fn(&str) -> f64) -> Vec<Arm> {
        let mut out = Vec::new();
        for (i, p) in self.pieces.iter().enumerate() {
            let Some((a, ha, b, hb)) = p.ends().filter(|_| is_road(&p.file) && !p.hidden && !p.auto) else { continue };
            for (pt, h) in [(a, roads::norm_deg(ha + 180.0)), (b, hb)] {
                let covered = self.pieces.iter().enumerate().any(|(j, q)| j != i && !q.hidden && !q.auto && is_road(&q.file) && q.distance_to(pt) < (half(&q.file) * 0.9).max(0.4));
                if !covered {
                    out.push(Arm { x: pt.x, y: pt.y, out: h, road: p.file.clone(), auto: true });
                }
            }
        }
        out
    }

    /// The arms the editor finds (Auto-Junction) replaced by those of the drawing as it is
    /// now; arms put by hand stay, and a found one near a hand-put one is not added.
    pub fn sync_arms(&mut self, is_road: &dyn Fn(&str) -> bool, half: &dyn Fn(&str) -> f64) {
        let found = self.free_ends(is_road, half);
        self.arms.retain(|a| !a.auto);
        for f in found {
            if !self.arms.iter().any(|a| (DVec2::new(a.x, a.y) - DVec2::new(f.x, f.y)).length() < 1.5) {
                self.arms.push(f);
            }
        }
    }
}

/// How far apart two neighbouring arms may face for a turn or corner between them, and how far
/// from facing each other for a piece straight on (degrees).
const TURN_MAX: f64 = 165.0;
const THROUGH_SLACK: f64 = 15.0;

/// The way round from heading `a` to `b` clockwise (0..360).
fn gap(a: f64, b: f64) -> f64 {
    (b - a).rem_euclid(360.0)
}

/// A rounded corner between two arms: the arms' middle lines run in to where they meet and a
/// tangent arc of `radius` takes the turn. None where the lines do not meet ahead of both arms
/// or the arc does not fit.
fn corner(a: &Arm, b: &Arm, radius: f64) -> Option<Vec<LSeg>> {
    let (pa, pb) = (DVec2::new(a.x, a.y), DVec2::new(b.x, b.y));
    let (da, db) = (roads::dir(roads::norm_deg(a.out + 180.0)), roads::dir(b.out));
    // the lines meet at pa + s*da == pb + w*db (w = -t)
    let det = da.x * db.y - da.y * db.x;
    if det.abs() < 1e-6 {
        return None;
    }
    let v = pb - pa;
    let s = (v.x * db.y - v.y * db.x) / det;
    let t = (v.x * da.y - v.y * da.x) / det;
    // (ahead of the way in, and behind the way out)
    if s < 0.5 || t > -0.5 {
        return None;
    }
    let x = pa + da * s;
    let turn = da.angle_to(db).abs();
    let reach = radius * (turn / 2.0).tan();
    if !reach.is_finite() || reach > s - 0.3 || reach > -t - 0.3 {
        return None;
    }
    let (ta, tb) = (x - da * reach, x + db * reach);
    let ha = roads::norm_deg(a.out + 180.0);
    let mut out = Vec::new();
    if (ta - pa).length() > 0.05 {
        out.push(LSeg::of(&roads::straight(pa, ta)?));
    }
    out.push(LSeg::of(&tangent_arc(ta, ha, tb)?));
    if (pb - tb).length() > 0.05 {
        out.push(LSeg::of(&roads::straight(tb, pb)?));
    }
    Some(out)
}

impl Layout {
    /// The pieces Auto-Junction builds from the arms, replacing those it built before; pieces
    /// drawn by hand are not touched and come first. Gives back where each old piece went: the
    /// index it has now, or None for a removed one.
    pub fn sync_built(&mut self) -> Vec<Option<usize>> {
        let mut map = Vec::new();
        let mut kept = Vec::new();
        for p in std::mem::take(&mut self.pieces) {
            if p.auto {
                map.push(None);
            } else {
                map.push(Some(kept.len()));
                kept.push(p);
            }
        }
        self.pieces = kept;
        let arms: Vec<&Arm> = self.arms.iter().filter(|a| !a.road.is_empty()).collect();
        let b = self.build;
        let mut made = Vec::new();
        let mut order: Vec<usize> = (0..arms.len()).collect();
        order.sort_by(|&i, &j| arms[i].out.rem_euclid(360.0).total_cmp(&arms[j].out.rem_euclid(360.0)));
        if b.turns || b.corners {
            let n = order.len();
            for k in 0..n {
                if n == 2 && k == 1 {
                    break;
                }
                let (a, c) = (arms[order[k]], arms[order[(k + 1) % n]]);
                if n < 2 || gap(a.out, c.out) > TURN_MAX {
                    continue;
                }
                let segs = if b.corners { corner(a, c, b.radius.max(1.0)) } else { None }.or_else(|| {
                    if !b.turns {
                        return None;
                    }
                    let segs = roads::biarc(DVec2::new(a.x, a.y), roads::norm_deg(a.out + 180.0), DVec2::new(c.x, c.y), c.out)?;
                    Some(segs.iter().map(LSeg::of).collect())
                });
                if let Some(segs) = segs {
                    made.push(Piece { file: a.road.clone(), segs, lift: 0.0, hidden: false, auto: true });
                }
            }
        }
        if b.through {
            for (x, &i) in order.iter().enumerate() {
                for &j in &order[x + 1..] {
                    let (a, c) = (arms[i], arms[j]);
                    if (gap(a.out, c.out) - 180.0).abs() > THROUGH_SLACK {
                        continue;
                    }
                    let (pa, pc) = (DVec2::new(a.x, a.y), DVec2::new(c.x, c.y));
                    let ha = roads::norm_deg(a.out + 180.0);
                    // (straight on when the middle lines coincide, else one smooth bend)
                    let side = (pc - pa).dot(roads::right(ha));
                    let segs: Option<Vec<LSeg>> = if side.abs() < 0.05 {
                        roads::straight(pa, pc).map(|s| vec![LSeg::of(&s)])
                    } else {
                        roads::biarc(pa, ha, pc, c.out).map(|v| v.iter().map(LSeg::of).collect())
                    };
                    if let Some(segs) = segs {
                        made.push(Piece { file: a.road.clone(), segs, lift: 0.0, hidden: false, auto: true });
                    }
                }
            }
        }
        self.pieces.extend(made);
        map
    }
}

/// What a drawing comes to with its junctions made as the Roads tool makes them: the roads
/// cut back where they cross or meet, and for each junction the roads that end at it.
pub struct Resolved {
    /// Every piece to put on the map or in the object: the roads cut back, the rest as drawn.
    pub pieces: Vec<Piece>,
    /// The roads that end at each junction, for its plate.
    pub plates: Vec<Vec<crate::junction::PlateArm>>,
    /// Junctions found but not made: the roads are too short or too close for the room one needs.
    pub skipped: usize,
}

/// Something of a piece to cut away: the stretch `from..to` along it.
type Cut = (usize, f64, f64);

impl Piece {
    pub fn length(&self) -> f64 {
        self.segs.iter().map(|s| s.length).sum()
    }

    /// The point and heading `d` along the piece.
    fn at(&self, d: f64) -> (DVec2, f64) {
        let mut off = 0.0;
        for (i, s) in self.segs.iter().enumerate() {
            let seg = s.seg();
            if d <= off + s.length + 1e-9 || i + 1 == self.segs.len() {
                let l = (d - off).clamp(0.0, s.length);
                return (seg.point(l), seg.heading_at(l));
            }
            off += s.length;
        }
        (DVec2::ZERO, 0.0)
    }

    /// The part of the piece from `from` to `to` along it.
    pub fn part(&self, from: f64, to: f64) -> Option<Piece> {
        let mut segs = Vec::new();
        let mut off = 0.0;
        for s in &self.segs {
            let (a, b) = (from.max(off), to.min(off + s.length));
            if b - a > 0.01 {
                let seg = s.seg();
                let (d0, d1) = (a - off, b - off);
                segs.push(LSeg { x: seg.point(d0).x, y: seg.point(d0).y, heading: seg.heading_at(d0), radius: s.radius, length: d1 - d0 });
            }
            off += s.length;
        }
        (!segs.is_empty()).then(|| Piece { segs, ..self.clone() })
    }

    /// Its middle line every 0.5 m: (point, distance along).
    fn polyline(&self) -> Vec<(DVec2, f64)> {
        let l = self.length();
        let n = (l / 0.5).ceil().max(2.0) as usize;
        (0..=n).map(|i| (self.at(l * i as f64 / n as f64).0, l * i as f64 / n as f64)).collect()
    }
}

impl Layout {
    /// The drawing with junctions made as the Roads tool makes them (when `build.plate`): where
    /// one road crosses another, ends on another's middle, or three or more roads end at one
    /// place, the roads are cut back by the room the junction needs, and the roads that end at
    /// it are given for its plate. A junction whose roads leave too little is not made.
    pub fn resolve(&self, is_road: &dyn Fn(&str) -> bool, half: &dyn Fn(&str) -> f64) -> Resolved {
        let plain = Resolved { pieces: self.pieces.clone(), plates: Vec::new(), skipped: 0 };
        if !self.build.plate {
            return plain;
        }
        let roads_ix: Vec<usize> = (0..self.pieces.len()).filter(|&i| { let p = &self.pieces[i]; is_road(&p.file) && !p.hidden && !p.auto && p.length() > 1.0 }).collect();
        let polys: Vec<Vec<(DVec2, f64)>> = roads_ix.iter().map(|&i| self.pieces[i].polyline()).collect();
        let hw = |i: usize| half(&self.pieces[i].file).max(1.0);
        let head_diff = |a: f64, b: f64| (a - b + 180.0).rem_euclid(360.0) - 180.0;
        // each junction: where the roads cut and by how much, as (piece, from, to) with its centre
        let mut events: Vec<Vec<Cut>> = Vec::new();
        let mut skipped = 0usize;
        // the room a junction takes, kept to what the roads have: a short road gives a smaller
        // junction (down to the least that holds the kerbs), not none
        let fit = |r: f64, room: f64, wide: f64| -> Option<f64> {
            let r = r.min(room);
            (r >= wide + 2.5).then_some(r)
        };
        let mut used_ends: Vec<(usize, bool)> = Vec::new();
        // roads running into another's middle (T junctions)
        for (xi, &i) in roads_ix.iter().enumerate() {
            let pi = &self.pieces[i];
            let li = pi.length();
            for at_end in [false, true] {
                let (pt, h) = pi.at(if at_end { li } else { 0.0 });
                for (xj, &j) in roads_ix.iter().enumerate() {
                    if xj == xi {
                        continue;
                    }
                    let lj = self.pieces[j].length();
                    let Some((q, aj)) = polys[xj].iter().map(|(q, d)| ((*q - pt).length(), (*q, *d))).filter(|(d, _)| *d < 0.6).min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| x.1) else { continue };
                    let _ = q;
                    let hj = self.pieces[j].at(aj).1;
                    let r = crate::junction::radius_at(&[hw(i), hw(j)], head_diff(h, hj));
                    // (at the other road's own end the two are only joined)
                    if aj < 1.5 || aj > lj - 1.5 {
                        continue;
                    }
                    let Some(r) = fit(r, (aj.min(lj - aj) - 0.6).min(li - 0.6), hw(i).max(hw(j))) else {
                        skipped += 1;
                        continue;
                    };
                    let own = if at_end { (i, li - r, li) } else { (i, 0.0, r) };
                    events.push(vec![own, (j, aj - r, aj + r)]);
                    used_ends.push((i, at_end));
                }
            }
        }
        // roads crossing (X junctions)
        for (xi, &i) in roads_ix.iter().enumerate() {
            for (xj, &j) in roads_ix.iter().enumerate().skip(xi + 1) {
                let (li, lj) = (self.pieces[i].length(), self.pieces[j].length());
                for (pt, ai, aj) in crate::junction::crossings(&polys[xi], &polys[xj]) {
                    // (not at a road's end: that is a T junction or a joint)
                    if ai < 1.5 || ai > li - 1.5 || aj < 1.5 || aj > lj - 1.5 {
                        continue;
                    }
                    let _ = pt;
                    let r = crate::junction::radius_at(&[hw(i), hw(j)], head_diff(self.pieces[i].at(ai).1, self.pieces[j].at(aj).1));
                    let room = (ai.min(li - ai)).min(aj.min(lj - aj)) - 0.6;
                    let Some(r) = fit(r, room, hw(i).max(hw(j))) else {
                        skipped += 1;
                        continue;
                    };
                    events.push(vec![(i, ai - r, ai + r), (j, aj - r, aj + r)]);
                }
            }
        }
        // three or more roads ending at one place
        let mut ends: Vec<(usize, bool, DVec2, f64)> = Vec::new();
        for &i in &roads_ix {
            let p = &self.pieces[i];
            let l = p.length();
            let (a, ha) = p.at(0.0);
            let (b, hb) = p.at(l);
            ends.push((i, false, a, ha));
            ends.push((i, true, b, roads::norm_deg(hb + 180.0)));
        }
        let mut taken = vec![false; ends.len()];
        for x in 0..ends.len() {
            if taken[x] || used_ends.contains(&(ends[x].0, ends[x].1)) {
                continue;
            }
            let group: Vec<usize> = (0..ends.len()).filter(|&y| !taken[y] && !used_ends.contains(&(ends[y].0, ends[y].1)) && (ends[y].2 - ends[x].2).length() < 1.0).collect();
            if group.len() < 3 || group.iter().map(|&y| ends[y].0).collect::<std::collections::HashSet<_>>().len() < group.len() {
                continue;
            }
            // (the sharpest angle between roads side by side)
            let mut hs: Vec<f64> = group.iter().map(|&y| ends[y].3.rem_euclid(360.0)).collect();
            hs.sort_by(|a, b| a.total_cmp(b));
            let sharp = (0..hs.len()).map(|k| (hs[(k + 1) % hs.len()] - hs[k]).rem_euclid(360.0)).fold(360.0, f64::min);
            let r = crate::junction::radius_at(&group.iter().map(|&y| hw(ends[y].0)).collect::<Vec<_>>(), sharp);
            let mut cuts = Vec::new();
            let shortest = group.iter().map(|&y| self.pieces[ends[y].0].length()).fold(f64::MAX, f64::min);
            let widest = group.iter().map(|&y| hw(ends[y].0)).fold(0.0, f64::max);
            let Some(r) = fit(r, shortest - 0.6, widest) else {
                skipped += 1;
                continue;
            };
            for &y in &group {
                taken[y] = true;
                let (i, at_end, _, _) = ends[y];
                let l = self.pieces[i].length();
                cuts.push(if at_end { (i, l - r, l) } else { (i, 0.0, r) });
            }
            if !cuts.is_empty() {
                events.push(cuts);
            }
        }
        // the arms of each junction, from what is left of its roads beside the cuts
        let mut plates = Vec::new();
        let mut cuts_of: Vec<Vec<(f64, f64)>> = vec![Vec::new(); self.pieces.len()];
        for ev in &events {
            let mut arms = Vec::new();
            for &(i, lo, hi) in ev {
                let p = &self.pieces[i];
                let l = p.length();
                if lo > 0.5 {
                    let (pt, h) = p.at(lo);
                    arms.push(crate::junction::PlateArm { file: p.file.clone(), pos: pt, out: roads::norm_deg(h + 180.0), at_end: true });
                }
                if hi < l - 0.5 {
                    let (pt, h) = p.at(hi);
                    arms.push(crate::junction::PlateArm { file: p.file.clone(), pos: pt, out: h, at_end: false });
                }
            }
            if arms.len() < 3 {
                continue;
            }
            for &(i, lo, hi) in ev {
                cuts_of[i].push((lo.max(0.0), hi));
            }
            plates.push(arms);
        }
        let mut pieces = Vec::new();
        for (i, p) in self.pieces.iter().enumerate() {
            if cuts_of[i].is_empty() {
                pieces.push(p.clone());
                continue;
            }
            let mut cuts = cuts_of[i].clone();
            cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
            let (mut at, l) = (0.0, p.length());
            for (lo, hi) in cuts.into_iter().chain(std::iter::once((l, l))) {
                if lo - at > 0.3 {
                    pieces.extend(p.part(at, lo));
                }
                at = at.max(hi);
            }
        }
        Resolved { pieces, plates, skipped }
    }
}

/// The arc from `p` that leaves heading `h` and reaches `q` (a tangent arc); a straight when
/// `q` lies ahead on the line. None when `q` is behind or the arc would be more than a half
/// circle round.
pub fn tangent_arc(p: DVec2, h: f64, q: DVec2) -> Option<Seg> {
    let v = q - p;
    let d = v.length();
    if d < 0.5 {
        return None;
    }
    let t = roads::dir(h);
    let sin_t = t.perp_dot(v) / d; // (positive: q to the left)
    let cos_t = t.dot(v) / d;
    if cos_t < -0.2 {
        return None;
    }
    let theta = sin_t.atan2(cos_t); // deflection of the chord from the tangent
    if theta.abs() < 0.2_f64.to_radians() {
        return Some(Seg { start: p, heading: h, length: d, radius: 0.0 });
    }
    let r = d / (2.0 * theta.abs().sin());
    let length = r * 2.0 * theta.abs();
    // (a right turn: a positive radius)
    Some(Seg { start: p, heading: h, length, radius: if theta < 0.0 { r } else { -r } })
}

/// A smooth chain of arcs through `pts`, leaving the first with heading `start` and
/// arriving at the last with heading `end` when those are given; else the ends bend the
/// way the chain does.
pub fn curve_through(pts: &[DVec2], start: Option<f64>, end: Option<f64>) -> Vec<Seg> {
    let n = pts.len();
    if n < 2 {
        return Vec::new();
    }
    let chord = |i: usize| roads::heading_of(pts[i + 1] - pts[i]);
    let diff = |a: f64, b: f64| (a - b + 180.0).rem_euclid(360.0) - 180.0;
    let mut hs: Vec<f64> = (0..n)
        .map(|i| {
            if i == 0 {
                chord(0)
            } else if i == n - 1 {
                chord(n - 2)
            } else {
                // the middle of the way in and the way out
                let (a, b) = (chord(i - 1), chord(i));
                roads::norm_deg(a + diff(b, a) / 2.0)
            }
        })
        .collect();
    if n > 2 {
        hs[0] = roads::norm_deg(chord(0) - diff(hs[1], chord(0)));
        hs[n - 1] = roads::norm_deg(chord(n - 2) + diff(chord(n - 2), hs[n - 2]));
    }
    if let Some(h) = start {
        hs[0] = h;
    }
    if let Some(h) = end {
        hs[n - 1] = h;
    }
    let mut out = Vec::new();
    for i in 0..n - 1 {
        let segs = if (diff(hs[i], chord(i)).abs() < 0.3 && diff(hs[i + 1], chord(i)).abs() < 0.3) || (pts[i + 1] - pts[i]).length() < 1.0 { None } else { roads::biarc(pts[i], hs[i], pts[i + 1], hs[i + 1]) };
        match segs.or_else(|| roads::straight(pts[i], pts[i + 1]).map(|s| vec![s])) {
            Some(v) => out.extend(v.into_iter().filter(|s| s.length > 0.05)),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::tests::flat_map;

    #[test]
    fn an_arc_through_three_points_passes_through_them() {
        // a quarter circle of radius 20 about (0, 0), clockwise from (0, 20) by (14.14, 14.14) to (20, 0)
        let (a, b, c) = (DVec2::new(0.0, 20.0), DVec2::new(14.142135, 14.142135), DVec2::new(20.0, 0.0));
        let s = arc3(a, b, c).unwrap();
        assert!((s.radius - 20.0).abs() < 1e-3, "{}", s.radius);
        assert!((s.length - 20.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-3);
        assert!((s.heading - 90.0).abs() < 1e-3, "heads east at the top");
        assert!((s.end() - c).length() < 1e-3);
        assert!((s.point(s.length / 2.0) - b).length() < 1e-3);
        // the other way round: counter-clockwise, a negative radius
        let s = arc3(c, b, a).unwrap();
        assert!((s.radius + 20.0).abs() < 1e-3 && (s.end() - a).length() < 1e-3);
        // three points in a line: a straight
        let s = arc3(DVec2::ZERO, DVec2::new(5.0, 5.0), DVec2::new(10.0, 10.0)).unwrap();
        assert_eq!(s.radius, 0.0);
    }

    #[test]
    fn a_layout_is_put_on_the_map_turned_and_linked() {
        let mut doc = flat_map(1);
        let layout = Layout {
            name: "T".into(),
            pieces: vec![Piece { file: "Splines\\a.sli".into(), segs: vec![LSeg { x: 0.0, y: 0.0, heading: 0.0, radius: 0.0, length: 10.0 }, LSeg { x: 0.0, y: 10.0, heading: 0.0, radius: 0.0, length: 5.0 }], lift: 0.02, hidden: false, auto: false }],
            ..Default::default()
        };
        let made = place(&mut doc, &layout, DVec2::new(100.0, 100.0), 3.0, 90.0).unwrap();
        assert_eq!(made.len(), 2);
        let (a, b) = (doc.spline(made[0].0, made[0].1).unwrap(), doc.spline(made[1].0, made[1].1).unwrap());
        // turned a quarter clockwise: what ran north runs east
        assert!((a.heading - 90.0).abs() < 1e-9 && (a.pos[0] - 100.0).abs() < 1e-9 && (a.pos[1] - 100.0).abs() < 1e-9);
        assert!((b.pos[0] - 110.0).abs() < 1e-9 && (b.pos[1] - 100.0).abs() < 1e-6, "{:?}", b.pos);
        assert!((a.pos[2] - 3.02).abs() < 1e-9);
        assert_eq!((a.next_id, b.prev_id), (b.id, a.id));
        // past the map nothing is laid
        let n = doc.tiles[&(0, 0)].tile.splines.len();
        assert!(place(&mut doc, &layout, DVec2::new(100.0, 295.0), 0.0, 0.0).is_err());
        assert_eq!(doc.tiles[&(0, 0)].tile.splines.len(), n);
    }

    fn crossing(road: &str) -> Layout {
        Layout { name: "X".into(), arms: [0.0, 90.0, 180.0, 270.0].iter().map(|h| Arm { x: 0.0, y: 0.0, out: *h, road: road.into(), ..Default::default() }).collect(), ..Default::default() }
    }

    #[test]
    fn a_layout_fits_roads_facing_about_the_same_ways() {
        let any = |_: &str, _: &str| true;
        let l = crossing("");
        // the same crossing turned 20 degrees, the roads listed in another order
        let roads_out: Vec<(f64, String)> = [200.0, 20.0, 110.0, 290.0].iter().map(|h| (*h, String::new())).collect();
        let (rot, by_arm, err) = fit(&l, &roads_out, &any).unwrap();
        assert!((rot - 20.0).abs() < 1e-6 && err < 1e-6, "{rot} {err}");
        // arm 0 (north) meets the road that now faces 20
        assert_eq!(by_arm[0], 1);
        // a crossing that is not square still fits within the tolerance
        let skew: Vec<(f64, String)> = [0.0, 75.0, 180.0, 255.0].iter().map(|h| (*h, String::new())).collect();
        assert!(fit(&l, &skew, &any).is_some());
        // too far off, or another number of arms: no
        let bad: Vec<(f64, String)> = [0.0, 20.0, 180.0, 200.0].iter().map(|h| (*h, String::new())).collect();
        assert!(fit(&l, &bad, &any).is_none());
        assert!(fit(&l, &roads_out[..3], &any).is_none());
        // an arm made for one road type does not take another
        let typed = crossing("Splines\\a.sli");
        assert!(fit(&typed, &roads_out, &|a, b| a == b).is_none());
    }

    #[test]
    fn layouts_are_kept_and_read_back() {
        let dir = std::env::temp_dir().join(format!("om-cj-{}", std::process::id()));
        let mut l = crossing("Splines\\a.sli");
        l.name = "My crossing".into();
        l.auto = true;
        l.save(&dir).unwrap();
        let back = Layout::load_all(&dir);
        assert_eq!(back, vec![l]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_tangent_arc_leaves_the_way_it_was_going_and_reaches_the_point() {
        let s = tangent_arc(DVec2::ZERO, 0.0, DVec2::new(10.0, 10.0)).unwrap();
        // north, then a right turn through a quarter circle to a point 10 east, 10 north
        assert!((s.radius - 10.0).abs() < 1e-6 && (s.end() - DVec2::new(10.0, 10.0)).length() < 1e-6, "{s:?}");
        let l = tangent_arc(DVec2::ZERO, 0.0, DVec2::new(-10.0, 10.0)).unwrap();
        assert!(l.radius < 0.0 && (l.end() - DVec2::new(-10.0, 10.0)).length() < 1e-6);
        assert_eq!(tangent_arc(DVec2::ZERO, 0.0, DVec2::new(0.0, 10.0)).unwrap().radius, 0.0);
        assert!(tangent_arc(DVec2::ZERO, 0.0, DVec2::new(0.0, -10.0)).is_none());
    }

    #[test]
    fn a_curve_goes_through_every_point_and_a_turned_layout_turns_back() {
        let pts = [DVec2::new(0.0, 0.0), DVec2::new(10.0, 10.0), DVec2::new(0.0, 20.0), DVec2::new(-10.0, 30.0)];
        let segs = curve_through(&pts, Some(0.0), None);
        assert!(segs.len() >= 3);
        for w in segs.windows(2) {
            assert!((w[0].end() - w[1].start).length() < 1e-3, "joined: {w:?}");
        }
        assert!((segs.last().unwrap().end() - pts[3]).length() < 1e-3);
        for p in &pts[1..3] {
            assert!(segs.iter().any(|s| (s.end() - *p).length() < 1e-3), "passes {p:?}");
        }
        let mut l = Layout { pieces: vec![Piece { file: "a".into(), segs: segs.iter().map(LSeg::of).collect(), lift: 0.0, hidden: false, auto: false }], arms: vec![Arm { x: 3.0, y: 4.0, out: 20.0, ..Default::default() }], ..Default::default() };
        let before = l.clone();
        l.transform(true, 90.0);
        assert!(l != before);
        l.transform(false, -90.0);
        l.transform(true, 0.0);
        for (a, b) in l.pieces[0].segs.iter().zip(&before.pieces[0].segs) {
            assert!((a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.heading - b.heading).abs() < 1e-6 && (a.radius - b.radius).abs() < 1e-6);
        }
        assert!((l.arms[0].out - 20.0).abs() < 1e-6 && (l.arms[0].x - 3.0).abs() < 1e-6);
    }

    #[test]
    fn auto_junction_builds_turns_corners_and_straight_on_between_arms() {
        let road = |x: f64, y: f64, h: f64| Piece { file: "r".into(), segs: vec![LSeg { x, y, heading: h, radius: 0.0, length: 20.0 }], lift: 0.0, hidden: false, auto: false };
        // a cross: four stubs ending 20 m from the middle
        let mut l = Layout { pieces: vec![road(0.0, 0.0, 0.0), road(0.0, 0.0, 90.0), road(0.0, 0.0, 180.0), road(0.0, 0.0, 270.0)], ..Default::default() };
        let (is_road, half) = (|f: &str| f == "r", |_: &str| 3.0);
        l.sync_arms(&is_road, &half);
        assert_eq!(l.arms.len(), 4);
        // the arms' ends are free only while the built pieces do not count as covering them
        l.build = AutoBuild { plate: false, turns: true, corners: false, through: true, radius: 8.0 };
        let map = l.sync_built();
        assert_eq!(map, vec![Some(0), Some(1), Some(2), Some(3)]);
        let built: Vec<Piece> = l.pieces.iter().filter(|p| p.auto).cloned().collect();
        // four turns and two straight-on pieces
        assert_eq!(built.len(), 6);
        assert_eq!(l.pieces.iter().filter(|p| !p.auto).count(), 4);
        l.sync_arms(&is_road, &half);
        assert_eq!(l.arms.len(), 4, "built pieces must not hide or add arms");
        // a built piece joins two arms
        for p in &built {
            let (a, _, b, _) = p.ends().unwrap();
            assert!(l.arms.iter().any(|m| (DVec2::new(m.x, m.y) - a).length() < 0.1));
            assert!(l.arms.iter().any(|m| (DVec2::new(m.x, m.y) - b).length() < 0.1));
        }
        // building again changes nothing, and the hand-drawn pieces keep their places
        let before = l.pieces.clone();
        let map = l.sync_built();
        assert_eq!(l.pieces, before);
        assert_eq!(&map[..4], &[Some(0), Some(1), Some(2), Some(3)]);
        assert!(map[4..].iter().all(|m| m.is_none()));
        // rounded corners: an arc of the radius takes each 90° turn, tangent to both roads
        l.build = AutoBuild { plate: false, turns: false, corners: true, through: false, radius: 8.0 };
        l.sync_built();
        let corners: Vec<Piece> = l.pieces.iter().filter(|p| p.auto).cloned().collect();
        assert_eq!(corners.len(), 4);
        for p in &corners {
            let arc = p.segs.iter().find(|s| s.radius != 0.0).expect("an arc");
            assert!((arc.radius.abs() - 8.0).abs() < 0.05, "{arc:?}");
            assert!((arc.length - 8.0 * std::f64::consts::FRAC_PI_2).abs() < 0.1, "{arc:?}");
        }
        // everything switched off: only the hand-drawn pieces are left
        l.build = AutoBuild { plate: false, turns: false, corners: false, through: false, radius: 8.0 };
        l.sync_built();
        assert_eq!(l.pieces.len(), 4);
        // a layout from before has no auto marks and reads back
        let old: Layout = serde_json::from_str(r#"{"pieces":[{"file":"r","segs":[]}]}"#).unwrap();
        assert!(!old.pieces[0].auto && old.build.plate && !old.build.turns);
    }

    #[test]
    fn roads_come_in_where_a_road_end_is_free() {
        let road = |y0: f64, y1: f64, x: f64| Piece { file: "r".into(), segs: vec![LSeg { x, y: y0, heading: 0.0, radius: 0.0, length: y1 - y0 }], lift: 0.0, hidden: false, auto: false };
        // a road north-south in two pieces, one crossing road: free ends are the four outer ones... a T: west-east stub meets it
        let mut l = Layout { pieces: vec![road(-20.0, 0.0, 0.0), road(0.0, 20.0, 0.0), Piece { file: "r".into(), segs: vec![LSeg { x: 0.0, y: 0.0, heading: 90.0, radius: 0.0, length: 20.0 }], lift: 0.0, hidden: false, auto: false }], ..Default::default() };
        let is_road = |f: &str| f == "r";
        let half = |_: &str| 3.0;
        let ends = l.free_ends(&is_road, &half);
        assert_eq!(ends.len(), 3, "{ends:?}");
        assert!(ends.iter().any(|a| (a.out - 180.0).abs() < 1e-6 && (a.y + 20.0).abs() < 1e-6));
        assert!(ends.iter().any(|a| (a.out - 90.0).abs() < 1e-6 && (a.x - 20.0).abs() < 1e-6));
        // the arms found are replaced on the next look; one put by hand stays
        l.arms.push(Arm { x: 50.0, y: 50.0, out: 0.0, ..Default::default() });
        l.sync_arms(&is_road, &half);
        l.sync_arms(&is_road, &half);
        assert_eq!(l.arms.len(), 4);
    }
}
