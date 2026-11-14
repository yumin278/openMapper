//! Junctions made on the spot, as Transport Fever makes them: where a new road meets or
//! crosses another, both are cut back from the meeting point and a scenery object of its own
//! fills the gap. The object is ordinary OMSI content, written into openOMSI's content folder
//! (`Sceneryobjects\openMapper\<map>\junction_<id>.sco`): a flat asphalt plate (a DirectX
//! `.x` mesh) and a `[path]` for every way through it - each lane coming in joined to the
//! lanes going out of the other arms (straight on, right, left: the AI follows them as it
//! follows any crossing's paths, which openOMSI links to the roads' lanes where their ends
//! meet), and footpaths round its corners.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glam::{DVec2, DVec3};
use omsi_geometry::SplineCurve;
use omsi_map::MapObject;

use crate::assets::{Assets, SplineModel};
use crate::doc::{tile_of, tile_origin, Document, TileKey};
use crate::roads::{self, Seg};

/// One road running into the junction: the spline whose end (or start) is at its edge.
#[derive(Debug, Clone, Copy)]
pub struct Arm {
    pub key: TileKey,
    pub id: i64,
    /// The spline's end lies at the junction (else its start).
    pub at_end: bool,
}

/// A lane end at the junction's edge (world).
#[derive(Debug, Clone, Copy)]
struct LaneEnd {
    arm: usize,
    pos: DVec3,
    /// The heading traffic moves with there (degrees).
    flow: f64,
    kind: i32,
    width: f64,
    /// Lateral place, right of the arm's way into the junction (m).
    side: f64,
    /// Traffic comes into the junction here (else it leaves; footways: both).
    incoming: bool,
    /// This end belongs to a lane added by the bus pocket tool for this taper.
    pocket: bool,
}

/// Room a junction needs round its centre for these roads (m): the widest of them plus
/// a margin for the curves.
pub fn radius(half_widths: &[f64]) -> f64 {
    half_widths.iter().cloned().fold(3.0, f64::max) * 1.15 + 4.0
}

/// Room a junction needs when its roads meet at `angle` degrees (the sharper of the two
/// angles between them): at a sharp corner the kerbs meet far out from the centre, and the
/// arms must reach past that by enough for the kerb to be rounded off wider than the
/// footway beside it.
pub fn radius_at(half_widths: &[f64], angle: f64) -> f64 {
    let hw = half_widths.iter().cloned().fold(3.0, f64::max);
    let half = (angle.abs().rem_euclid(180.0).min(180.0 - angle.abs().rem_euclid(180.0)).max(20.0) * 0.5).to_radians();
    let base = radius(half_widths);
    base.max((hw + 2.5) / half.tan()).min(base * 3.0)
}

/// The arms' geometry: where each ends, which way it goes in, its half width and lanes.
fn arm_ends(doc: &Document, assets: &mut Assets, arms: &[Arm], pocket_paths: Option<&std::ops::Range<usize>>) -> Option<(Vec<(DVec3, f64, f64, String)>, Vec<LaneEnd>)> {
    let mut ends = Vec::new();
    let mut lanes = Vec::new();
    for (ai, a) in arms.iter().enumerate() {
        let s = doc.spline(a.key, a.id)?;
        let model = assets.spline(&s.file)?;
        let c = SplineCurve::from_map(s, tile_origin(a.key));
        let at = if a.at_end { s.length } else { 0.0 };
        let into = if a.at_end { c.heading_at(at) } else { c.heading_at(at) + 180.0 };
        let p = c.point_at(at);
        ends.push((p, roads::norm_deg(into), model.half_width as f64, s.file.clone()));
        let paths = pocket_paths.filter(|_| ai == 1);
        lanes.extend(lanes_at(&model, s.mirror, ai, a.at_end, |x, z| c.offset_point(at, x, z), c.heading_at(at), paths));
    }
    Some((ends, lanes))
}

/// The lanes of a road type where it meets a junction: `place(x, z)` is where a lane at
/// lateral place `x` (and height `z`) ends, `along` the heading of the road's own way there.
fn lanes_at(model: &SplineModel, mirrored: bool, ai: usize, at_end: bool, place: impl Fn(f64, f64) -> DVec3, along: f64, pocket_paths: Option<&std::ops::Range<usize>>) -> Vec<LaneEnd> {
    let mut lanes = Vec::new();
    let lateral_sign = if mirrored { -1.0 } else { 1.0 };
    for (index, path) in model.def.paths.iter().enumerate() {
        let x = path.start[0] as f64 * lateral_sign;
        let z = path.start[2] as f64;
        let pos = place(x, z);
        // A mirrored spline moves each lane across the road and reverses its flow.
        let direction = match (path.direction, mirrored) {
            (0, true) => 1,
            (1, true) => 0,
            (d, _) => d,
        };
        // along the spline: 0 forwards, 1 backwards, 2 both ways; (incoming, flow)
        let ways: Vec<(bool, f64)> = match direction {
            0 => vec![(at_end, along)],
            1 => vec![(!at_end, along + 180.0)],
            _ => vec![(true, if at_end { along } else { along + 180.0 }), (false, if at_end { along + 180.0 } else { along })],
        };
        // lateral place seen from the arm's way into the junction
        let side = if at_end { x } else { -x };
        let pocket = path.kind == 0 && pocket_paths.is_some_and(|paths| paths.contains(&index));
        for (incoming, flow) in ways {
            lanes.push(LaneEnd { arm: ai, pos, flow: roads::norm_deg(flow), kind: path.kind, width: path.width as f64, side, incoming, pocket });
        }
    }
    lanes
}

/// One `[path]` record (object frame = world less the junction's centre).
struct PathRec {
    seg: Seg,
    z0: f64,
    z1: f64,
    kind: i32,
    width: f64,
    direction: i32,
    blinker: i32,
    /// The light its traffic waits at (`[use_traffic_light]`), if any.
    light: Option<usize>,
    /// This path is part of a connection into or out of a bus pocket lane.
    pocket: bool,
}

/// What a junction has besides its roads: traffic lights (a plan of two phases, the main
/// road's and the side roads'), and zebra crossings on some of its arms. Kept in the
/// junction's own `.sco` as a comment line, so it is made again with them.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub lights: bool,
    /// Also place physical signal heads and poles (the traffic light program runs without them).
    pub light_objects: bool,
    /// Green for the main road and for the side roads (s).
    pub main_green: f64,
    pub side_green: f64,
    /// The arms with a zebra crossing, by their heading out of the junction (degrees).
    pub crossings: Vec<f64>,
    /// Paint white stripes on the selected zebra crossings; their AI paths are independent.
    pub zebra_texture: bool,
    /// Whether the junction generates AI footway paths around its corners.
    pub corner_walks: bool,
    /// Empty to use the map's default combination.
    pub signal_set: String,
}

impl Default for Options {
    fn default() -> Self {
        Options { lights: false, light_objects: false, main_green: 30.0, side_green: 20.0, crossings: Vec::new(), zebra_texture: true, corner_walks: true, signal_set: String::new() }
    }
}

const OPTIONS_TAG: &str = "openMapper junction:";

impl Options {
    fn line(&self) -> String {
        let c: Vec<String> = self.crossings.iter().map(|h| format!("{h:.1}")).collect();
        // (helpers=omsi: its spline helpers are in OMSI's order, x height forward)
        format!("{OPTIONS_TAG} lights={} heads={} main={:.0} side={:.0} crossings={} zebra_tex={} walks={} helpers=omsi sig={}", self.lights as i32, self.light_objects as i32, self.main_green, self.side_green, c.join(","), self.zebra_texture as i32, self.corner_walks as i32, self.signal_set)
    }

    /// The options written in a junction's `.sco` text (defaults when none are).
    pub fn parse(sco: &str) -> Options {
        let mut o = Options::default();
        let Some(line) = sco.lines().find(|l| l.trim_start().starts_with(OPTIONS_TAG)) else { return o };
        let mut heads = None;
        for kv in line.trim_start()[OPTIONS_TAG.len()..].split_whitespace() {
            let Some((k, v)) = kv.split_once('=') else { continue };
            match k {
                "lights" => o.lights = v == "1",
                "heads" => heads = match v { "0" => Some(false), "1" => Some(true), _ => heads },
                "main" => o.main_green = v.parse().unwrap_or(o.main_green),
                "side" => o.side_green = v.parse().unwrap_or(o.side_green),
                "crossings" => o.crossings = v.split(',').filter_map(|x| x.parse().ok()).collect(),
                "zebra_tex" => o.zebra_texture = match v { "0" => false, "1" => true, _ => o.zebra_texture },
                "walks" => o.corner_walks = match v { "0" => false, "1" => true, _ => o.corner_walks },
                "sig" => o.signal_set = v.to_owned(),
                _ => {}
            }
        }
        // Saved integration junctions placed signal models whenever lights were enabled.
        o.light_objects = heads.unwrap_or(o.lights);
        o
    }

    /// Read them from a junction's `.sco` file.
    pub fn read(path: &Path) -> Options {
        std::fs::read(path).ok().map(|b| Options::parse(&omsi_cfg::decode_text(&b))).unwrap_or_default()
    }

    /// Whether the arm going out along `out` has a crossing.
    pub fn crossing_at(&self, out: f64) -> bool {
        self.crossings.iter().any(|h| (roads::norm_deg(h - out + 180.0) - 180.0).abs() < 20.0)
    }
}

/// The two arms most nearly in line (the main road through the junction).
fn main_road(outs: &[f64]) -> (usize, usize) {
    let mut best = (0, 1, -1.0);
    for i in 0..outs.len() {
        for j in i + 1..outs.len() {
            let d = (roads::norm_deg(outs[i] - outs[j] + 180.0) - 180.0).abs();
            if d > best.2 {
                best = (i, j, d);
            }
        }
    }
    (best.0, best.1)
}

/// The light program: light 0 the main road's, light 1 the side roads': red-yellow 2 s,
/// green, yellow 3 s, then red, with 2 s of all red between them.
fn light_program(o: &Options) -> String {
    let (gm, gs) = (o.main_green.max(5.0), o.side_green.max(5.0));
    let main_turn = 2.0 + gm + 3.0 + 2.0;
    let cycle = main_turn + 2.0 + gs + 3.0 + 2.0;
    let mut t = format!("[traffic_lights_group]\n{}\n\n", num(cycle));
    let _ = write!(t, "[traffic_light]\nMain road\n\n[phase]\n3\n2\n\n[phase]\n6\n{}\n\n[phase]\n9\n3\n\n[phase]\n0\n0\n\n", num(gm));
    let _ = write!(t, "[traffic_light]\nSide roads\n\n[phase]\n0\n{}\n\n[phase]\n3\n2\n\n[phase]\n6\n{}\n\n[phase]\n9\n3\n\n[phase]\n0\n0\n\n", num(main_turn), num(gs));
    t
}

fn signed_turn(from: f64, to: f64) -> f64 {
    (to - from + 540.0).rem_euclid(360.0) - 180.0
}

/// A sidewalk lane's original endpoint and its projected crossing endpoint.
#[derive(Clone, Copy)]
struct WalkJoin {
    arm: usize,
    left: bool,
    at_arm: DVec3,
    at_crossing: DVec3,
    width: f64,
}


#[derive(Clone)]
struct KerbCorner {
    /// Counter-clockwise mesh corner: the left edge of i leads to the right edge of j.
    i: usize,
    j: usize,
    segments: Vec<Seg>,
    lead_i: f64,
    lead_j: f64,
}

fn corner_leads(corners: &[KerbCorner]) -> Vec<(usize, usize, f64, f64)> {
    corners.iter().map(|c| (c.i, c.j, c.lead_i, c.lead_j)).collect()
}

/// Distance to the polyline obtained by sampling the *same* rounded kerb
/// pieces used to construct the plate (0.25 m sampling).
fn kerb_polyline(corner: &KerbCorner) -> Vec<DVec2> {
    let mut points = Vec::new();
    for seg in &corner.segments {
        let steps = (seg.length / 0.25).ceil().max(1.0) as usize;
        for n in 0..=steps {
            if !points.is_empty() && n == 0 { continue; }
            points.push(seg.point(seg.length * n as f64 / steps as f64));
        }
    }
    points
}

fn point_to_polyline(p: DVec2, line: &[DVec2]) -> f64 {
    line.windows(2).map(|q| {
        let edge = q[1] - q[0];
        let t = if edge.length_squared() > 1e-12 {
            ((p - q[0]).dot(edge) / edge.length_squared()).clamp(0.0, 1.0)
        } else { 0.0 };
        (p - (q[0] + edge * t)).length()
    }).fold(f64::INFINITY, f64::min)
}

fn path_kerb_clearance(segs: &[Seg], corner: &KerbCorner) -> (f64, DVec2) {
    let line = kerb_polyline(corner);
    let mut best = (f64::INFINITY, DVec2::ZERO);
    for seg in segs {
        let steps = (seg.length / 0.25).ceil().max(1.0) as usize;
        for n in 0..=steps {
            let p = seg.point(seg.length * n as f64 / steps as f64);
            let d = point_to_polyline(p, &line);
            if d < best.0 { best = (d, p); }
        }
    }
    best
}

/// Keep straight tangents at the road ends, then join the shortened portion
/// with a biarc. Retry three times at half the previous tangent lengths.
/// A zero-length tangent is omitted, preserving the original path verbatim.
fn led_biarc(pa: DVec2, ha: f64, pb: DVec2, hb: f64, lead_a: f64, lead_b: f64) -> Vec<Seg> {
    if lead_a <= 0.0 && lead_b <= 0.0 {
        return roads::biarc(pa, ha, pb, hb).or_else(|| roads::straight(pa, pb).map(|s| vec![s])).unwrap_or_default();
    }
    let (mut la, mut lb) = (lead_a, lead_b);
    let mut attempts = Vec::new();
    for retry in 0..=3 {
        let p = pa + roads::dir(ha) * la;
        let q = pb - roads::dir(hb) * lb;
        attempts.push(format!("#{retry} ({la:.3}, {lb:.3}): biarc {:?} -> {:?} returned None", p, q));
        if let Some(mut middle) = roads::biarc(p, ha, q, hb) {
            if retry > 0 {
                log::warn!(
                    "junction biarc recovered after reduced tangent leads {:?} -> {:?}; previous attempts: {}",
                    pa, pb, attempts[..attempts.len() - 1].join("; ")
                );
            }
            let mut segs = Vec::new();
            if la > 1e-6 {
                segs.push(Seg { start: pa, heading: ha, length: la, radius: 0.0 });
            }
            segs.append(&mut middle);
            if lb > 1e-6 {
                segs.push(Seg { start: q, heading: hb, length: lb, radius: 0.0 });
            }
            return segs;
        }
        if retry < 3 { la *= 0.5; lb *= 0.5; }
    }
    if let Some(segs) = roads::biarc(pa, ha, pb, hb) {
        log::warn!("junction biarc lead retries failed {:?} -> {:?}; attempts: {}; using original biarc",
            pa, pb, attempts.join("; "));
        return segs;
    }
    log::warn!("junction biarc lead retries and original biarc failed {:?} -> {:?}; attempts: {}; reason: roads::biarc returned None; using straight fallback",
        pa, pb, attempts.join("; "));
    roads::straight(pa, pb).map(|s| vec![s]).unwrap_or_default()
}

fn turn_blinker(turn: f64) -> i32 {
    if turn > 30.0 { 3 } else if turn < -30.0 { 2 } else { 0 }
}

#[derive(Clone, Copy)]
struct LaneRoute {
    from: usize,
    to: usize,
    turn: f64,
    blinker: i32,
}

impl LaneRoute {
    fn lane(self, incoming: bool) -> usize {
        if incoming { self.from } else { self.to }
    }

    fn with_lane(self, incoming: bool, lane: usize) -> Self {
        if incoming { Self { from: lane, ..self } } else { Self { to: lane, ..self } }
    }
}

/// Append a connection for each remaining lane, first exits, then entrances.
fn fill_vehicle_lanes(
    lanes: &[LaneEnd], incoming: &[Vec<usize>], outgoing: &[Vec<usize>],
    used: &mut [bool], routes: &mut Vec<LaneRoute>, mut write: impl FnMut(LaneRoute) -> bool,
) {
    for (entering, same, opposite) in [(false, outgoing, incoming), (true, incoming, outgoing)] {
        // Keep the nearest-lane choice independent of the order of repairs in this pass.
        let existing = routes.len();
        for (arm, group) in same.iter().enumerate() {
            for (rank, &idx) in group.iter().enumerate() {
                if used[idx] {
                    continue;
                }
                let lane = &lanes[idx];
                let nearest = routes[..existing].iter().filter_map(|&route| {
                    let neighbor = &lanes[route.lane(entering)];
                    (neighbor.arm == arm).then_some((neighbor.pos.distance_squared(lane.pos), route))
                }).min_by(|a, b| a.0.total_cmp(&b.0));

                let route = nearest.map(|(_, route)| route.with_lane(entering, idx)).or_else(|| {
                    opposite.iter().enumerate().filter(|(other, _)| *other != arm)
                        .filter_map(|(_, peers)| {
                            let peer = &lanes[*peers.first()?];
                            let turn = if entering {
                                signed_turn(lane.flow, peer.flow)
                            } else {
                                signed_turn(peer.flow, lane.flow)
                            };
                            if turn.abs() > 150.0 {
                                return None;
                            }
                            let blinker = turn_blinker(turn);
                            let other = match blinker {
                                3 => peers[0],
                                2 => peers[peers.len() - 1],
                                _ => peers[rank.min(peers.len() - 1)],
                            };
                            let (from, to) = if entering { (idx, other) } else { (other, idx) };
                            Some(LaneRoute { from, to, turn, blinker })
                        }).min_by(|a, b| a.turn.abs().total_cmp(&b.turn.abs()))
                });

                if let Some(route) = route.filter(|&route| write(route)) {
                    used[route.from] = true;
                    used[route.to] = true;
                    routes.push(route);
                    log::info!(
                        "junction connected {} lane at arm {} side {:.3}: arm {} side {:.3} {:?} -> arm {} side {:.3} {:?}, turn {:.1}°, blinker {}",
                        if entering { "incoming" } else { "outgoing" }, arm, lane.side,
                        lanes[route.from].arm, lanes[route.from].side, lanes[route.from].pos,
                        lanes[route.to].arm, lanes[route.to].side, lanes[route.to].pos,
                        signed_turn(lanes[route.from].flow, lanes[route.to].flow), route.blinker
                    );
                }
            }
        }
    }
}

/// The ways through the junction.
fn paths(
    lanes: &[LaneEnd], centre: DVec3, light_of: &[Option<usize>], walk_joins: &[WalkJoin],
    corner_walks: bool, kerb_lead: &[(usize, usize, f64, f64)],
    zebra_arms: &[bool], kerb_corners: &[KerbCorner], left_hand_traffic: bool,
) -> Vec<PathRec> {
    paths_with_lane_repair(lanes, centre, light_of, walk_joins, corner_walks, kerb_lead,
        zebra_arms, kerb_corners, left_hand_traffic, true)
}

fn paths_with_lane_repair(
    lanes: &[LaneEnd], centre: DVec3, light_of: &[Option<usize>], walk_joins: &[WalkJoin],
    corner_walks: bool, kerb_lead: &[(usize, usize, f64, f64)],
    zebra_arms: &[bool], kerb_corners: &[KerbCorner], left_hand_traffic: bool,
    repair: bool,
) -> Vec<PathRec> {
    let mut out = Vec::new();
    let c2 = centre.truncate();
    let arms = lanes.iter().map(|l| l.arm).max().map(|m| m + 1).unwrap_or(0);
    let mut incoming = vec![Vec::new(); arms];
    let mut outgoing = vec![Vec::new(); arms];
    for (idx, lane) in lanes.iter().enumerate().filter(|(_, lane)| lane.kind == 0) {
        let groups = if lane.incoming { &mut incoming } else { &mut outgoing };
        groups[lane.arm].push(idx);
    }
    for group in &mut incoming {
        group.sort_by(|&a, &b| lanes[b].side.total_cmp(&lanes[a].side));
    }
    // Seen leaving: its right is the arm's left coming in.
    for group in &mut outgoing {
        group.sort_by(|&a, &b| lanes[a].side.total_cmp(&lanes[b].side));
    }

    let mut write = |route: LaneRoute| {
        let (a, b) = (&lanes[route.from], &lanes[route.to]);
        let pocket = a.pocket || b.pocket;
        let i = a.arm;
        let blinker = route.blinker;
        let j = b.arm;
        let turn = route.turn;
        let pa = a.pos.truncate() - c2;
        let pb = b.pos.truncate() - c2;
        let turning = turn.abs() >= 30.0;
        // The outer kerb is on the right for right-hand traffic, on the
        // left for left-hand traffic. The paired outermost lane is
        // already selected above by the existing blinker logic.
        let outer = if left_hand_traffic { blinker == 2 } else { blinker == 3 };
        // The mesh's corner runs from i.left to j.right: a right turn
        // goes i -> j, and a left turn goes j -> i.
        let corner = if !turning { None } else {
            kerb_corners.iter().find(|c| if blinker == 3 {
                c.i == i && c.j == j
            } else {
                c.i == j && c.j == i
            })
        };
        let (kerb_a, kerb_b) = if outer {
            kerb_lead.iter().find_map(|&(ki, kj, li, lj)| {
                if blinker == 3 && ki == i && kj == j { Some((li, lj)) }
                else if blinker == 2 && ki == j && kj == i { Some((lj, li)) }
                else { None }
            }).unwrap_or((0.0, 0.0))
        } else { (0.0, 0.0) };
        let zebra_a = if turning && zebra_arms.get(i).copied().unwrap_or(false) { 4.0 } else { 0.0 };
        let zebra_b = if turning && zebra_arms.get(j).copied().unwrap_or(false) { 4.0 } else { 0.0 };
        let segs = led_biarc(pa, a.flow, pb, b.flow, kerb_a.max(zebra_a), kerb_b.max(zebra_b));
        if segs.is_empty() { return false; }
        if let Some(c) = corner {
            let (distance, position) = path_kerb_clearance(&segs, c);
            let required = a.width.max(2.5) * 0.5 - 0.1;
            if distance < required {
                log::warn!(
                    "junction turning lane {:?} -> {:?} approaches corner kerb: min {:.3} m at {:?}, required {:.3} m",
                    pa, pb, distance, position, required
                );
            }
        }
        let total: f64 = segs.iter().map(|s| s.length).sum::<f64>().max(0.01);
        let mut acc = 0.0;
        for (n, s) in segs.into_iter().enumerate() {
            let z0 = a.pos.z + (b.pos.z - a.pos.z) * acc / total - centre.z;
            acc += s.length;
            let z1 = a.pos.z + (b.pos.z - a.pos.z) * acc / total - centre.z;
            // (traffic waits where it comes in: the light is on the way's first piece)
            let light = if n == 0 { light_of.get(i).copied().flatten() } else { None };
            out.push(PathRec { seg: s, z0, z1, kind: 0, width: a.width.max(2.5), direction: 0, blinker, light, pocket });
        }
        true
    };
    let mut used = vec![false; lanes.len()];
    let mut routes = Vec::new();
    // traffic: every lane coming in to the lanes going out of the other arms, matched by
    // their place counted from the right (right turns from the right, left turns from the left)
    for i in 0..arms {
        let ins = &incoming[i];
        for j in (0..arms).filter(|j| *j != i) {
            let outs = &outgoing[j];
            if ins.is_empty() || outs.is_empty() {
                continue;
            }
            let turn = signed_turn(lanes[ins[0]].flow, lanes[outs[0]].flow);
            if turn.abs() > 150.0 {
                continue;
            }
            let blinker = turn_blinker(turn);
            let pairs: Vec<(usize, usize)> = if blinker == 3 {
                vec![(ins[0], outs[0])]
            } else if blinker == 2 {
                vec![(ins[ins.len() - 1], outs[outs.len() - 1])]
            } else {
                (0..ins.len().max(outs.len())).map(|k| (ins[k.min(ins.len() - 1)], outs[k.min(outs.len() - 1)])).collect()
            };
            for (a, b) in pairs {
                let route = LaneRoute { from: a, to: b, turn, blinker };
                if write(route) {
                    used[a] = true;
                    used[b] = true;
                    routes.push(route);
                }
            }
        }
    }
    if repair {
        fill_vehicle_lanes(lanes, &incoming, &outgoing, &mut used, &mut routes, write);
    }
    // footways round the corners: each arm's outermost footway lane on a side to the
    // nearest one of the next arm round the junction
    let mut order: Vec<usize> = (0..arms).collect();
    let angle = |i: usize| -> f64 {
        let l = lanes.iter().find(|l| l.arm == i);
        l.map(|l| { let d = l.pos.truncate() - c2; d.y.atan2(d.x) }).unwrap_or(0.0)
    };
    order.sort_by(|a, b| angle(*a).total_cmp(&angle(*b)));
    if corner_walks && arms >= 2 {
        for k in 0..arms {
            let (i, j) = (order[k], order[(k + 1) % arms]);
            if arms == 2 && k == 1 {
                break;
            }
            let walk_i: Vec<&LaneEnd> = lanes.iter().filter(|l| l.arm == i && l.kind == 1).collect();
            let walk_j: Vec<&LaneEnd> = lanes.iter().filter(|l| l.arm == j && l.kind == 1).collect();
            let mut best: Option<(f64, &LaneEnd, &LaneEnd)> = None;
            // In counter-clockwise order, i's left side meets j's right side.
            // If a zebra is present, select its actual sidewalk lanes.
            let on_side = |l: &LaneEnd, left: bool| {
                walk_joins.iter().find(|w| w.arm == l.arm && w.left == left)
                    .map(|w| (l.pos - w.at_arm).length() < 0.01).unwrap_or(true)
            };
            let joined = |l: &LaneEnd| {
                walk_joins.iter().find(|w| w.arm == l.arm && (l.pos - w.at_arm).length() < 0.01)
                    .map(|w| w.at_crossing).unwrap_or(l.pos)
            };
            for a in walk_i.iter().filter(|l| on_side(l, true)) {
                for b in walk_j.iter().filter(|l| on_side(l, false)) {
                    let d = (a.pos - b.pos).length();
                    if best.map(|x| d < x.0).unwrap_or(true) {
                        best = Some((d, a, b));
                    }
                }
            }
            if let Some((_, a, b)) = best {
                let (a_pos, b_pos) = (joined(a), joined(b));
                let pa = a_pos.truncate() - c2;
                let pb = b_pos.truncate() - c2;
                if let Some(s) = roads::straight(pa, pb) {
                    out.push(PathRec { seg: s, z0: a_pos.z - centre.z, z1: b_pos.z - centre.z, kind: 1, width: a.width.max(1.5), direction: 2, blinker: 0, light: None, pocket: false });
                }
            }
        }
    }
    out
}

/// One side of a road's cross-section beyond its kerb line (kerb, footway...): the
/// points in order outward, as (distance past the kerb line, height, u, v scale).
#[derive(Clone)]
struct SideProfile {
    texture: Option<PathBuf>,
    points: Vec<(f64, f64, f64, f64)>,
}

/// An arm as the junction's mesh needs it, seen from the junction looking out along it.
#[derive(Clone)]
struct ArmGeo {
    end: DVec3,
    out: f64,
    /// Roadway edges (the kerb lines) to the right and left of the way out (m).
    right_w: f64,
    left_w: f64,
    #[allow(dead_code)]
    right: Vec<SideProfile>,
    left: Vec<SideProfile>,
    /// Texture phase where the arm meets the junction, and which way it runs into it.
    v0: f64,
    vdir: f64,
    /// The roadway's own texture and where its lane (no markings) lies: (texture, u of a
    /// lane's middle, the roadway's u range, metres per repeat along the road).
    road_tex: Option<(PathBuf, f64, f64, f64, f64, f64)>,
    /// How high the roadway's surface lies over the spline's own height (m): the profile a
    /// lane runs on, where roads are drawn above the line they are laid along.
    road_z: f64,
}

fn arm_geometry(doc: &Document, assets: &mut Assets, a: &Arm) -> Option<ArmGeo> {
    let s = doc.spline(a.key, a.id)?.clone();
    let model = assets.spline(&s.file)?;
    let c = SplineCurve::from_map(&s, tile_origin(a.key));
    let at = if a.at_end { s.length } else { 0.0 };
    let end = c.point_at(at);
    let out = roads::norm_deg(if a.at_end { c.heading_at(at) + 180.0 } else { c.heading_at(at) });
    Some(arm_geo_of(&model, s.mirror, a.at_end, end, out, at + s.tex_offset))
}

/// An arm's geometry from its road type: the spline's end at the junction (`at_end`: it is
/// the spline's end, else its start), `out` the heading out along the road, `v_at` how far
/// along its texture runs there.
fn arm_geo_of(model: &SplineModel, mirrored: bool, at_end: bool, end: DVec3, out: f64, v_at: f64) -> ArmGeo {
    type Point = (f64, f64, f64, f64); // x, z, u, v scale
    let m = if mirrored { -1.0 } else { 1.0 };
    let profs: Vec<(usize, Vec<Point>)> = model.def.profiles.iter().filter(|p| p.points.len() >= 2).map(|p| (p.texture, p.points.iter().map(|q| (q.x as f64 * m, q.z as f64, q.u as f64, q.v_scale as f64)).collect())).collect();
    let lanes: Vec<(f64, f64, f64)> = model.def.paths.iter().filter(|q| q.kind == 0)
        .map(|q| (q.start[0] as f64 * m, q.width as f64, q.start[2] as f64)).collect();
    let mut heights: Vec<_> = lanes.iter().map(|&(_, _, z)| z).collect();
    heights.sort_by(f64::total_cmp);
    let lane_z = (!heights.is_empty()).then(|| {
        let mid = heights.len() / 2;
        (heights[(heights.len() - 1) / 2] + heights[mid]) * 0.5
    });

    let interpolate = |a: Point, b: Point, x: f64| -> Point {
        if x == a.0 {
            return a;
        }
        if x == b.0 {
            return b;
        }
        let t = (x - a.0) / (b.0 - a.0);
        (x, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t, a.3 + (b.3 - a.3) * t)
    };
    let span = |pts: &[Point]| {
        pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)))
    };
    let near_surface = 0.02;
    let (xl, xr) = if let Some(z) = lane_z {
        let lo = lanes.iter().map(|(x, w, _)| x - w / 2.0).fold(f64::INFINITY, f64::min);
        let hi = lanes.iter().map(|(x, w, _)| x + w / 2.0).fold(f64::NEG_INFINITY, f64::max);
        // A bridge deck and its underside can overlap the same lanes in x.
        // Use the segments closest to driving height within the lane envelope.
        let overlapping = profs.iter().flat_map(|(_, pts)| pts.windows(2)).filter_map(|segment| {
            let (a, b) = (segment[0], segment[1]);
            let (left, right) = span(segment);
            if right <= lo + 0.05 || left >= hi - 0.05 || right - left <= 1e-6 {
                return None;
            }
            let z0 = interpolate(a, b, left.max(lo)).1;
            let z1 = interpolate(a, b, right.min(hi)).1;
            let distance = (z0.min(z1) - z).max(z - z0.max(z1)).max(0.0);
            Some((left, right, distance))
        });
        let nearest = overlapping.clone().map(|(_, _, d)| d).fold(f64::INFINITY, f64::min);
        let edges = overlapping.filter(|(_, _, d)| *d < nearest + near_surface)
            .fold((lo, hi), |(lo, hi), (left, right, _)| (lo.min(left), hi.max(right)));

        let expand = |mut edge: f64, sign: f64| {
            loop {
                let mut outermost = sign * edge;
                for (_, pts) in &profs {
                    let (left, right) = span(pts);
                    let (inner, outer) = if sign > 0.0 { (left, right) } else { (right, left) };
                    if (inner - edge).abs() < 0.05 && right - left > 0.1
                        && pts.iter().all(|p| (p.1 - pts[0].1).abs() < 1e-6 && (p.1 - z).abs() < near_surface)
                    {
                        outermost = outermost.max(sign * outer);
                    }
                }
                if outermost == sign * edge {
                    return edge;
                }
                edge = sign * outermost;
            }
        };
        (expand(edges.0, -1.0), expand(edges.1, 1.0))
    } else {
        let touches_centre = |pts: &[Point]| {
            let (lo, hi) = span(pts);
            lo <= 0.05 && hi >= -0.05
        };
        let use_centre = profs.iter().any(|(_, pts)| touches_centre(pts));
        profs.iter().filter(|(_, pts)| !use_centre || touches_centre(pts))
            .flat_map(|(_, pts)| pts).fold((0.0_f64, 0.0_f64), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)))
    };

    // Clip a polyline to an x interval, keeping each continuous piece and its UVs.
    let clip = |pts: &[Point], lo: f64, hi: f64| -> Vec<Vec<Point>> {
        let mut pieces = Vec::new();
        let mut piece = Vec::new();
        for segment in pts.windows(2) {
            let (a, b) = (segment[0], segment[1]);
            let (left, right) = span(segment);
            let (first, last) = (left.max(lo), right.min(hi));
            if first > last || (first == last && left != right) {
                if piece.len() >= 2 {
                    pieces.push(std::mem::take(&mut piece));
                }
                continue;
            }
            let (x0, x1) = if a.0 <= b.0 { (first, last) } else { (last, first) };
            let (p, q) = if a.0 == b.0 { (a, b) } else { (interpolate(a, b, x0), interpolate(a, b, x1)) };
            if piece.last() != Some(&p) {
                if !piece.is_empty() {
                    pieces.push(std::mem::take(&mut piece));
                }
                piece.push(p);
            }
            piece.push(q);
        }
        if piece.len() >= 2 {
            pieces.push(piece);
        }
        pieces
    };
    let inside = |pts: &[Point]| pts.iter().all(|q| q.0 >= xl - 0.05 && q.0 <= xr + 0.05);
    let lane_mid = lanes.iter().map(|(x, _, _)| *x).find(|x| *x > xl && *x < xr).unwrap_or((xl + xr) * 0.5);
    let sample_lane = |pts: &[Point]| {
        pts.windows(2).filter_map(|w| {
            let (lo, hi) = span(w);
            (hi - lo > 1e-6 && (lo..=hi).contains(&lane_mid)).then(|| interpolate(w[0], w[1], lane_mid))
        }).reduce(|a, b| {
            if lane_z.is_none_or(|z| (b.1 - z).abs() <= (a.1 - z).abs()) { b } else { a }
        })
    };
    let central = if let Some(z) = lane_z {
        profs.iter().flat_map(|(t, pts)| clip(pts, xl, xr).into_iter().map(move |piece| (*t, piece)))
            .filter_map(|(t, pts)| sample_lane(&pts).map(|sample| (t, pts, sample)))
            .min_by(|a, b| (a.2.1 - z).abs().total_cmp(&(b.2.1 - z).abs()))
    } else {
        let mut inside_profiles = profs.iter().filter(|(_, pts)| inside(pts));
        inside_profiles.clone().find(|(_, pts)| {
            let (lo, hi) = span(pts);
            (lo..=hi).contains(&lane_mid)
        }).or_else(|| inside_profiles.next())
            .map(|(t, pts)| (*t, pts.clone(), sample_lane(pts).unwrap_or(pts[0])))
    };
    let road_z = lane_z.or_else(|| central.as_ref().map(|(_, _, sample)| sample.1)).unwrap_or(0.0);

    let side_pieces = |pts: &[Point], right: bool| {
        if lane_z.is_some() {
            let (lo, hi) = if right { (xr, f64::INFINITY) } else { (f64::NEG_INFINITY, xl) };
            clip(pts, lo, hi)
        } else if pts.iter().all(|q| if right { q.0 >= xr - 0.05 } else { q.0 <= xl + 0.05 }) {
            vec![pts.to_vec()]
        } else {
            Vec::new()
        }
    };
    let side = |right: bool| -> Vec<SideProfile> {
        let mut profiles = profs
            .iter()
            .flat_map(|(t, pts)| side_pieces(pts, right).into_iter().map(move |pts| (*t, pts)))
            .map(|(t, pts)| {
                let mut points: Vec<(f64, f64, f64, f64)> = pts.iter().map(|q| ((if right { q.0 - xr } else { xl - q.0 }).max(0.0), q.1, q.2, q.3)).collect();
                if points.first().map(|p| p.0).unwrap_or(0.0) > points.last().map(|p| p.0).unwrap_or(0.0) {
                    points.reverse();
                }
                SideProfile { texture: model.materials.get(t).and_then(|m| m.texture.clone()), points }
            })
            .collect::<Vec<_>>();
        // The .sli profile order may run outer-to-inner on the left, but the
        // junction blends strips by index. Always match inner-to-outer strips.
        profiles.sort_by(|a, b| {
            let key = |p: &SideProfile| {
                (
                    p.points.first().unwrap().0,
                    p.points.last().unwrap().0,
                    p.points.iter().map(|q| q.1).fold(f64::INFINITY, f64::min),
                )
            };
            let (a0, a1, az) = key(a);
            let (b0, b1, bz) = key(b);
            a0.total_cmp(&b0).then_with(|| a1.total_cmp(&b1)).then_with(|| az.total_cmp(&bz))
        });
        profiles
    };
    let (spline_right, spline_left) = (side(true), side(false));
    // (looking out along an arm whose end is at the junction, the spline's right is our left)
    let (right, left, right_w, left_w) = if at_end { (spline_left, spline_right, -xl, xr) } else { (spline_right, spline_left, xr, -xl) };
    let vs = central.as_ref().map(|(_, pts, _)| pts[0].3).unwrap_or(0.2).abs().max(1e-3);
    let v0 = vs * v_at * m;
    let vdir = if at_end { m } else { -m };
    let road_tex = central.and_then(|(t, pts, sample)| {
        let tex = model.materials.get(t)?.texture.clone()?;
        let range = (pts.iter().map(|q| q.2).fold(f64::MIN, f64::max) - pts.iter().map(|q| q.2).fold(f64::MAX, f64::min)).abs();
        let (lo, hi) = span(&pts);
        let width = (hi - lo).max(0.5);
        // (a lane's width: the street lanes' own, else half the roadway)
        let lane = lanes.iter().map(|(_, width, _)| *width).fold(0.0, f64::max);
        let lane = if lane > 1.5 { lane / 0.9 } else { width * 0.5 };
        Some((tex, sample.2, range, 1.0 / vs, width, lane))
    });
    ArmGeo { end, out, right_w, left_w, right, left, v0, vdir, road_tex, road_z }
}

/// The junction's mesh: vertices (relative to the centre: position, normal, uv), triangles
/// with their material, and the materials' textures.
#[derive(Default)]
struct Mesh {
    verts: Vec<([f64; 3], [f64; 3], [f64; 2])>,
    tris: Vec<([usize; 3], usize)>,
    textures: Vec<Option<PathBuf>>,
}

impl Mesh {
    fn material(&mut self, tex: Option<PathBuf>) -> usize {
        match self.textures.iter().position(|t| *t == tex) {
            Some(i) => i,
            None => {
                self.textures.push(tex);
                self.textures.len() - 1
            }
        }
    }

    /// A triangle facing `n` (the vertices are turned round if they face the other way).
    fn tri(&mut self, p: [DVec3; 3], uv: [[f64; 2]; 3], n: DVec3, mat: usize) {
        let face = (p[1] - p[0]).cross(p[2] - p[0]);
        if face.length() < 1e-9 {
            return;
        }
        let order = if face.dot(n) >= 0.0 { [0, 1, 2] } else { [0, 2, 1] };
        let base = self.verts.len();
        let n = n.normalize_or_zero();
        for i in order {
            self.verts.push((p[i].to_array(), n.to_array(), uv[i]));
        }
        self.tris.push(([base, base + 1, base + 2], mat));
    }
}

/// The widest arm with a roadway texture supplies the existing UV direction.
fn asphalt_heading(arms: &[ArmGeo]) -> f64 {
    arms.iter().filter(|g| g.road_tex.is_some())
        .max_by(|a, b| (a.left_w + a.right_w).total_cmp(&(b.left_w + b.right_w)))
        .map(|g| g.out).unwrap_or(0.0)
}

/// Build the mesh: the roadway in the middle, and round each corner the kerb and footway
/// of the roads, swept along a smooth curve from one arm's kerb line to the next's.
/// `along`: the heading the roadway's texture runs (the widest road's), so its slabs and
/// patches lie as the road's do, not slanted across it.
fn junction_mesh(arms: &[ArmGeo], centre: DVec3, core_tex: Option<PathBuf>, period: f64, along: f64) -> (Mesh, Vec<KerbCorner>) {
    let mut mesh = Mesh::default();
    let core = mesh.material(core_tex);
    let rel = |p: DVec3| p - centre;
    // the arms round the junction, counter-clockwise
    let mut order: Vec<usize> = (0..arms.len()).collect();
    let ang = |i: usize| { let d = roads::dir(arms[i].out); d.y.atan2(d.x) };
    order.sort_by(|a, b| ang(*a).total_cmp(&ang(*b)));
    let mut outline: Vec<DVec3> = Vec::new();
    let mut kerb_corners = Vec::new();
    for k in 0..order.len() {
        let a = &arms[order[k]];
        let b = &arms[order[(k + 1) % order.len()]];
        let r_pt = a.end.truncate() + roads::right(a.out) * a.right_w;
        let l_pt = a.end.truncate() + roads::right(a.out) * -a.left_w;
        outline.push(DVec3::new(r_pt.x, r_pt.y, a.end.z));
        outline.push(DVec3::new(l_pt.x, l_pt.y, a.end.z));
        // the kerb from this arm's left kerb line round to the next arm's right one
        let from = l_pt;
        let to = b.end.truncate() + roads::right(b.out) * b.right_w;
        let segs = kerb_round(from, a.out, to, b.out);
        let total: f64 = segs.iter().map(|s| s.length).sum();
        // Only terminal straight pieces of a genuinely rounded kerb are
        // tangent leads; a straight-only kerb is not a lead-in.
        let rounded = segs.iter().any(|s| s.radius != 0.0);
        let lead_i = if rounded { segs.first().filter(|s| s.radius == 0.0).map(|s| s.length).unwrap_or(0.0) } else { 0.0 };
        let lead_j = if rounded { segs.last().filter(|s| s.radius == 0.0).map(|s| s.length).unwrap_or(0.0) } else { 0.0 };
        kerb_corners.push(KerbCorner {
            i: order[k], j: order[(k + 1) % order.len()],
            segments: segs.clone(), lead_i, lead_j,
        });
        if total < 0.05 {
            continue;
        }
        let n = (total / 0.75).ceil().max(4.0) as usize;
        // stations along the kerb: (point, heading, distance)
        let mut st: Vec<(DVec2, f64, f64)> = Vec::new();
        for i in 0..=n {
            let mut d = total * i as f64 / n as f64;
            let mut seg = &segs[0];
            for sgm in &segs {
                seg = sgm;
                if d <= sgm.length + 1e-9 {
                    break;
                }
                d -= sgm.length;
            }
            let d = d.min(seg.length);
            st.push((seg.point(d), seg.heading_at(d), total * i as f64 / n as f64));
        }
        for p in st.iter().skip(1).take(n.saturating_sub(1)) {
            let t = p.2 / total;
            outline.push(DVec3::new(p.0.x, p.0.y, a.end.z + (b.end.z - a.end.z) * t));
        }
        // the corner's kerb and footway: this arm's left side or the next arm's right one,
        // whichever reaches further out (the narrower one then runs into it, as a footway
        // meets a verge), swept along the curve (it lies to the right of the way round)
        let reach = |v: &[SideProfile]| v.iter().flat_map(|p| p.points.iter()).map(|q| q.0).fold(0.0, f64::max);
        let (sides, v0, vdir, back) = if reach(&b.right) > reach(&a.left) + 0.5 { (&b.right, b.v0, b.vdir, true) } else { (&a.left, a.v0, a.vdir, false) };
        // (the strip as it is at the arm it starts from and at the one it ends at: between two
        // roads of different width it grows from the one's footway to the other's)
        let (from_arm, to_arm) = (&a.left, &b.right);
        let blend = arms.len() == 2 && !from_arm.is_empty() && from_arm.len() == to_arm.len();
        let strips: Vec<(&SideProfile, &Vec<(f64, f64, f64, f64)>, &Vec<(f64, f64, f64, f64)>)> = if blend {
            from_arm.iter().zip(to_arm.iter()).map(|(f, t)| (if back { t } else { f }, &f.points, &t.points)).collect()
        } else {
            sides.iter().map(|p| (p, &p.points, &p.points)).collect()
        };
        for (prof, p_from, p_to) in strips {
            let mat = mesh.material(prof.texture.clone());
            let n = p_from.len().max(p_to.len());
            // (the profile at a place along the kerb, the shorter one's last point repeated)
            let at = |t: f64| -> Vec<(f64, f64, f64, f64)> {
                (0..n)
                    .map(|i| {
                        let (f, g) = (p_from[i.min(p_from.len() - 1)], p_to[i.min(p_to.len() - 1)]);
                        (f.0 + (g.0 - f.0) * t, f.1 + (g.1 - f.1) * t, f.2 + (g.2 - f.2) * t, f.3 + (g.3 - f.3) * t)
                    })
                    .collect()
            };
            for q in st.windows(2) {
                let (p0, h0, s0) = q[0];
                let (p1, h1, s1) = q[1];
                // (between two roads the strip's cross-sections are turned from the one's
                // across to the other's, so they meet the roads' ends square, with no wedge
                // of ground between)
                let turn = |h: f64, s: f64| if blend { let from = roads::norm_deg(a.out + 180.0); from + signed_turn(from, b.out) * (s / total) } else { h };
                let (h0, h1) = (turn(h0, s0), turn(h1, s1));
                let (w0, w1) = (at(s0 / total), at(s1 / total));
                for i in 0..n.saturating_sub(1) {
                    let (d0, z0, u0, vs0) = w0[i];
                    let (d1, z1, u1, vs1) = w0[i + 1];
                    let (e0, y0, _, _) = w1[i];
                    let (e1, y1, _, _) = w1[i + 1];
                    // the face's normal in the cross-section: up for a flat part, towards the road
                    // for the face of a kerb
                    let (dd, dz) = (d1 - d0, z1 - z0);
                    let base = |p: DVec2, h: f64, s: f64, d: f64, z: f64| {
                        let xy = p + roads::right(h) * d;
                        let zz = a.end.z + (b.end.z - a.end.z) * (s / total) + z;
                        rel(DVec3::new(xy.x, xy.y, zz))
                    };
                    // (the texture runs on from the arm it is taken from)
                    let v = |s: f64, vs: f64| if back { v0 - vdir * vs * (total - s) } else { v0 + vdir * vs * s };
                    let pa = base(p0, h0, s0, d0, z0);
                    let pb = base(p0, h0, s0, d1, z1);
                    let pc = base(p1, h1, s1, e0, y0);
                    let pd = base(p1, h1, s1, e1, y1);
                    let r = roads::right((h0 + h1) * 0.5 + if (h1 - h0).abs() > 180.0 { 180.0 } else { 0.0 });
                    // (in the cross-section, the left of the way outward: up, or towards the road)
                    let nrm = DVec3::new(-r.x * dz, -r.y * dz, dd);
                    let nrm = if nrm.length() < 1e-9 { DVec3::Z } else { nrm };
                    let (ua, ub) = ([u0, v(s0, vs0)], [u1, v(s0, vs1)]);
                    let (uc, ud) = ([u0, v(s1, vs0)], [u1, v(s1, vs1)]);
                    mesh.tri([pa, pb, pc], [ua, ub, uc], nrm, mat);
                    mesh.tri([pb, pd, pc], [ub, ud, uc], nrm, mat);
                }
            }
        }
    }
    log::debug!("junction outline: {:?}", outline.iter().map(|p| { let q = *p - centre; (q.x.round() as i64, q.y.round() as i64) }).collect::<Vec<_>>());
    for (i, g) in order.iter().map(|i| &arms[*i]).enumerate() {
        log::debug!("arm {i}: end ({:.1}, {:.1}) out {:.0} right {:.1} left {:.1}", g.end.x - centre.x, g.end.y - centre.y, g.out, g.right_w, g.left_w);
    }
    // the roadway: a fan round the centre, its texture laid flat on the map
    // (at the height the roads' surface lies, not the line they are laid along)
    let road_z = arms.iter().map(|a| a.road_z).sum::<f64>() / arms.len().max(1) as f64;
    let mid = DVec3::new(0.0, 0.0, outline.iter().map(|p| p.z - centre.z).sum::<f64>() / outline.len().max(1) as f64 + road_z);
    let (fwd, side) = (roads::dir(along), roads::right(along));
    let uv = |p: DVec3| [(p.x * side.x + p.y * side.y) / period, -(p.x * fwd.x + p.y * fwd.y) / period];
    for i in 0..outline.len() {
        let p0 = rel(outline[i]) + DVec3::Z * road_z;
        let p1 = rel(outline[(i + 1) % outline.len()]) + DVec3::Z * road_z;
        mesh.tri([mid, p0, p1], [uv(mid), uv(p0), uv(p1)], DVec3::Z, core);
    }
    (mesh, kerb_corners)
}

/// The kerb round a corner: from `a` (on a kerb line running out along `out_a`) to `b` (on
/// one running out along `out_b`). The two lines are carried on into the junction to where
/// they meet, and a circle touching both rounds the corner there, as large as the junction
/// has room for (a fillet); kerb lines in one line, or meeting outside, are joined straight.
fn kerb_round(a: DVec2, out_a: f64, b: DVec2, out_b: f64) -> Vec<Seg> {
    let (da, db) = (roads::dir(out_a), roads::dir(out_b));
    let straight = || roads::straight(a, b).map(|s| vec![s]).unwrap_or_default();
    // (the outside of a bend: the kerb lines part instead of meeting, and a smooth curve
    // carries the old edge round - a straight cut there left the ground showing)
    let smooth = || roads::biarc(a, roads::norm_deg(out_a + 180.0), b, out_b).unwrap_or_else(straight);
    let denom = da.perp_dot(db);
    if denom.abs() < 0.05 {
        // in one line (the straight side of a T): straight across; side by side: round
        return if da.dot(db) < 0.0 { straight() } else { smooth() };
    }
    // a + da * u = b + db * v
    let u = (b - a).perp_dot(db) / denom;
    let v = (b - a).perp_dot(da) / denom;
    if u > 0.01 || v > 0.01 {
        return smooth();
    }
    let x = a + da * u;
    let t = (-u).min(-v);
    let (ta, tb) = (x + da * t, x + db * t);
    let mut segs = Vec::new();
    if (ta - a).length() > 0.02 {
        segs.push(Seg { start: a, heading: roads::norm_deg(out_a + 180.0), length: (ta - a).length(), radius: 0.0 });
    }
    // the circle touching both lines at ta and tb (equally far from where they meet)
    let side = (tb - ta).dot(roads::right(out_a + 180.0));
    let chord = (tb - ta).length();
    let fwd = (tb - ta).dot(roads::dir(out_a + 180.0));
    if chord > 0.02 && side.abs() > 1e-6 {
        let r = (fwd * fwd + side * side) / (2.0 * side);
        let sweep = 2.0 * side.atan2(fwd);
        segs.push(Seg { start: ta, heading: roads::norm_deg(out_a + 180.0), length: (r * sweep).abs(), radius: r });
    }
    if (b - tb).length() > 0.02 {
        segs.push(Seg { start: tb, heading: roads::norm_deg(out_b), length: (b - tb).length(), radius: 0.0 });
    }
    if segs.is_empty() { straight() } else { segs }
}

/// The mesh as a DirectX `.x` text file in OMSI's frame (x right, y up, z forward).
/// (Seen from the side a triangle faces, OMSI's front faces turn clockwise.) Every triangle
/// is there twice, the second one facing the other way: a game that culls back faces by a
/// rule of its own (the winding the version reads, the normals it trusts) shows the plate
/// from above either way, as OMSI's own crossings are modelled.
fn mesh_x(m: &Mesh, names: &[String]) -> String {
    let mut x = String::from("xof 0302txt 0032\n\nMesh {\n");
    let nv = m.verts.len();
    let nf = m.tris.len();
    let _ = writeln!(x, " {};", nv * 2);
    for pass in 0..2 {
        for (i, (p, _, _)) in m.verts.iter().enumerate() {
            let _ = writeln!(x, " {:.4};{:.4};{:.4};{}", p[0], p[2], p[1], if pass == 1 && i + 1 == nv { ";" } else { "," });
        }
    }
    let _ = writeln!(x, " {};", nf * 2);
    // (front: the order turned for the swapped axes; back: the order as it stands, on its own vertices)
    let faces = |x: &mut String, last: bool| {
        for (i, (f, _)) in m.tris.iter().enumerate() {
            let _ = writeln!(x, " 3;{},{},{};,", f[0], f[2], f[1]);
            let _ = writeln!(x, " 3;{},{},{};{}", nv + f[0], nv + f[1], nv + f[2], if last && i + 1 == nf { ";" } else { "," });
        }
    };
    faces(&mut x, true);
    let _ = writeln!(x, " MeshNormals {{\n  {};", nv * 2);
    for pass in 0..2 {
        let sign = if pass == 0 { 1.0 } else { -1.0 };
        for (i, (_, n, _)) in m.verts.iter().enumerate() {
            let _ = writeln!(x, "  {:.4};{:.4};{:.4};{}", n[0] * sign, n[2] * sign, n[1] * sign, if pass == 1 && i + 1 == nv { ";" } else { "," });
        }
    }
    let _ = writeln!(x, "  {};", nf * 2);
    faces(&mut x, true);
    x.push_str(" }\n");
    let _ = writeln!(x, " MeshTextureCoords {{\n  {};", nv * 2);
    for pass in 0..2 {
        for (i, (_, _, t)) in m.verts.iter().enumerate() {
            let _ = writeln!(x, "  {:.5};{:.5};{}", t[0], t[1], if pass == 1 && i + 1 == nv { ";" } else { "," });
        }
    }
    x.push_str(" }\n");
    let _ = writeln!(x, " MeshMaterialList {{\n  {};\n  {};", names.len().max(1), nf * 2);
    for (i, (_, mat)) in m.tris.iter().enumerate() {
        let _ = writeln!(x, "  {mat},\n  {mat}{}", if i + 1 == nf { ";" } else { "," });
    }
    for n in names {
        let _ = writeln!(x, "  Material {{\n   1.0;1.0;1.0;1.0;;\n   10.0;\n   0.0;0.0;0.0;;\n   0.0;0.0;0.0;;");
        if !n.is_empty() {
            let _ = writeln!(x, "   TextureFilename {{\n    \"{n}\";\n   }}");
        }
        x.push_str("  }\n");
    }
    x.push_str(" }\n}\n");
    x
}

/// The files that go with a texture (`name_wintersnow.dds`, `name.bmp.cfg`, `name.surf`…):
/// everything beside it that starts with its name and then an underscore or a dot.
pub fn companions(tex: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem), Some(own)) = (tex.parent(), tex.file_stem().map(|s| s.to_string_lossy().to_lowercase()), tex.file_name().map(|s| s.to_string_lossy().to_lowercase())) else { return Vec::new() };
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let f = p.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
            p.is_file() && f != own && (f.starts_with(&format!("{stem}_")) || f.starts_with(&format!("{stem}.")))
        })
        .collect();
    out.sort();
    out
}

/// A texture copied into `to_dir` with everything that goes with it (its seasonal pictures,
/// its .cfg and .surf), under its own name.
pub fn copy_texture(tex: &Path, to_dir: &Path) {
    let Some(name) = tex.file_name() else { return };
    copy_texture_as(tex, to_dir, name);
}

fn copy_texture_as(tex: &Path, to_dir: &Path, name: &std::ffi::OsStr) {
    let to = to_dir.join(name);
    if !to.exists() {
        let _ = std::fs::copy(tex, &to);
    }
    for c in companions(tex) {
        let Some(n) = c.file_name() else { continue };
        let t = to_dir.join(n);
        if !t.exists() {
            let _ = std::fs::copy(&c, &t);
        }
    }
}

/// Write map overrides under the same names regardless of the map files' spelling.
fn export_texture(tex: &Path, to_dir: &Path) -> String {
    let Some(name) = tex.file_name().map(|name| name.to_string_lossy()) else { return String::new() };
    let canonical = [MAP_ASPHALT, MAP_ZEBRA].into_iter().find(|n| name.eq_ignore_ascii_case(n));
    if let Some(file) = canonical.filter(|n| name != *n) {
        copy_texture_as(tex, to_dir, file.as_ref());
        return file.to_owned();
    }
    if tex.parent() != Some(to_dir) {
        copy_texture(tex, to_dir);
    }
    name.into_owned()
}

fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.to_string() }
}

/// How finely a plate's own shape is kept: arms that differ by less make the same file. Far under
/// what shows where a road meets its plate, and well over a float's noise.
const PLATE_GRID_M: f64 = 0.01;
const PLATE_GRID_DEG: f64 = 0.25;

fn snapped(v: f64, grid: f64) -> f64 {
    (v / grid).round() * grid
}

/// The junction as its object file holds it. Roads meeting the same way must make the same file,
/// wherever they lie on the map, whichever way round, and whichever was drawn first:
/// `order` is the arms counter-clockwise from the one every such junction picks, `turn` the
/// object's heading on the map (0 when an option names arms by their heading on the map), and the
/// arms and lanes are in the object's frame, kept to the plate's grid, with the texture along the
/// kerbs starting afresh at each arm rather than running on from that road's own length.
struct PlateFrame {
    order: Vec<usize>,
    turn: f64,
    arms: Vec<ArmGeo>,
    lanes: Vec<LaneEnd>,
}

fn plate_frame(geo: &[ArmGeo], files: &[String], lanes: &[LaneEnd], centre: DVec3, keep_heading: bool) -> PlateFrame {
    let ccw = |g: &ArmGeo| roads::norm_deg(-g.out).rem_euclid(360.0);
    let mut round: Vec<usize> = (0..geo.len()).collect();
    round.sort_by(|a, b| ccw(&geo[*a]).total_cmp(&ccw(&geo[*b])));
    // (each arm as it looks from the junction, and the turn on to the next round)
    let looks: Vec<String> = (0..round.len())
        .map(|k| {
            let (i, next) = (round[k], round[(k + 1) % round.len()]);
            let g = &geo[i];
            let to_next = (ccw(&geo[next]) - ccw(g)).rem_euclid(360.0);
            let d = g.end - centre;
            format!("{}|{}|{:.2}|{:.2}|{:.2}|{:.2}|{:.2}|{:.2}", files[i].to_lowercase(), g.vdir, g.right_w, g.left_w, snapped(d.truncate().length(), PLATE_GRID_M), snapped(d.z, PLATE_GRID_M), snapped(to_next, PLATE_GRID_DEG), g.road_z)
        })
        .collect();
    let from_arm = |k: usize| -> Vec<&String> { (0..looks.len()).map(|n| &looks[(k + n) % looks.len()]).collect() };
    let first = (0..looks.len()).min_by_key(|k| from_arm(*k)).unwrap_or(0);
    let order: Vec<usize> = (0..round.len()).map(|n| round[(first + n) % round.len()]).collect();
    let turn = if keep_heading { 0.0 } else { order.first().map(|i| geo[*i].out).unwrap_or(0.0) };
    let frame = |p: DVec3| -> DVec3 {
        let q = crate::selection::turn_point((p - centre).truncate(), DVec2::ZERO, -turn);
        DVec3::new(snapped(q.x, PLATE_GRID_M), snapped(q.y, PLATE_GRID_M), snapped(p.z - centre.z, PLATE_GRID_M))
    };
    let heading = |h: f64| roads::norm_deg(snapped(roads::norm_deg(h - turn), PLATE_GRID_DEG));
    let arms = order
        .iter()
        .map(|i| {
            let g = &geo[*i];
            ArmGeo { end: frame(g.end), out: heading(g.out), v0: 0.0, ..g.clone() }
        })
        .collect();
    let lanes = order
        .iter()
        .enumerate()
        .flat_map(|(n, i)| lanes.iter().filter(move |l| l.arm == *i).map(move |l| (n, l)))
        .map(|(n, l)| LaneEnd { arm: n, pos: frame(l.pos), flow: heading(l.flow), ..*l })
        .collect();
    PlateFrame { order, turn, arms, lanes }
}

/// How far from `centre` the plate reaches: out to the corners of the arms' ends, with the
/// kerb and footway beyond the roadway (the ground under it is brought to its height out to
/// there, not only to the roads' own width).
pub fn reach(doc: &Document, assets: &mut Assets, centre: DVec3, arms: &[Arm]) -> f64 {
    arms.iter()
        .filter_map(|a| arm_geometry(doc, assets, a))
        .map(|g| {
            let beyond = g.right.iter().chain(g.left.iter()).flat_map(|p| p.points.iter()).map(|q| q.0).fold(0.0, f64::max);
            let wide = g.right_w.max(g.left_w) + beyond;
            ((g.end.truncate() - centre.truncate()).length().powi(2) + wide * wide).sqrt()
        })
        .fold(0.0, f64::max)
}

/// Place the white stripe triangles at the supplied height, in the road frame.
fn paint_zebra_stripes(mesh: &mut Mesh, zebras: &[(DVec2, f64, f64, f64)], z: f64, white: PathBuf) {
    let mat = mesh.material(Some(white));
    for (mid, out, lw, rw) in zebras {
        let (d, r) = (roads::dir(*out), roads::right(*out));
        let mut x = -lw + 0.25;
        while x < rw - 0.25 {
            let c = *mid + r * x;
            let q = [c - r * 0.25 - d * 1.5, c + r * 0.25 - d * 1.5, c + r * 0.25 + d * 1.5, c - r * 0.25 + d * 1.5].map(|p| DVec3::new(p.x, p.y, z));
            mesh.tri([q[0], q[1], q[2]], [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], DVec3::Z, mat);
            mesh.tri([q[0], q[2], q[3]], [[0.0, 0.0], [1.0, 1.0], [0.0, 1.0]], DVec3::Z, mat);
            x += 1.0;
        }
    }
}

#[derive(Default)]
pub(crate) struct MakeEdit<'a> {
    pub history: Option<&'a mut crate::history::History>,
    pub pocket_paths: Option<std::ops::Range<usize>>,
    /// Conform the terrain before placing lights, extending at least this far from the centre.
    pub conform_min_reach: Option<f64>,
}

fn write_junction_paths(body: &mut String, recs: &[PathRec]) {
    for p in recs {
        let s = &p.seg;
        let grad = if s.length > 0.01 { (p.z1 - p.z0) / s.length } else { 0.0 };
        let _ = write!(body, "[path]\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n", num(s.start.x), num(s.start.y), num(p.z0), num(s.heading), num(s.radius), num(s.length), num(grad), num(grad), p.kind, num(p.width), p.direction, p.blinker);
        if let Some(l) = p.light {
            let _ = write!(body, "[use_traffic_light]\n{l}\n\n");
        }
    }
}

/// The same with lights and crossings as `opts` says.
pub(crate) fn make_with(doc: &mut Document, assets: &mut Assets, content: &Path, asphalt: Option<&Path>, centre: DVec3, arms: &[Arm], opts: &Options, mut edit: MakeEdit<'_>) -> anyhow::Result<i64> {
    let (_, lanes) = arm_ends(doc, assets, arms, edit.pocket_paths.as_ref()).ok_or_else(|| anyhow::anyhow!("a road of the junction is not loaded"))?;
    let geo: Vec<ArmGeo> = arms.iter().map(|a| arm_geometry(doc, assets, a)).collect::<Option<_>>().ok_or_else(|| anyhow::anyhow!("a road of the junction is not loaded"))?;
    // (from here on, what goes in the file: in the object's frame, its centre the origin)
    let files: Vec<String> = arms.iter().map(|a| doc.spline(a.key, a.id).map(|s| s.file.clone()).unwrap_or_default()).collect();
    let plate = plate_frame(&geo, &files, &lanes, centre, !opts.crossings.is_empty());
    let plate_lights: Vec<Option<usize>> = if opts.lights && arms.len() >= 3 {
        let outs: Vec<f64> = plate.arms.iter().map(|g| g.out).collect();
        let (a, b) = main_road(&outs);
        (0..arms.len()).map(|i| Some(if i == a || i == b { 0 } else { 1 })).collect()
    } else {
        vec![None; arms.len()]
    };
    // (the signal heads stand on the map, arm by arm as the roads came)
    let mut light_of = vec![None; arms.len()];
    for (n, i) in plate.order.iter().enumerate() {
        light_of[*i] = plate_lights[n];
    }
    // Project the two sidewalk lanes on each marked arm to its mid section.
    let mut walk_joins = Vec::<WalkJoin>::new();
    let mut zebras: Vec<(DVec2, f64, f64, f64)> = Vec::new();
    let mut crossing_arms = Vec::new();
    for (i, g) in plate.arms.iter().enumerate().filter(|(_, g)| opts.crossing_at(g.out)) {
        let r = roads::right(g.out);
        let mid = g.end.truncate() - roads::dir(g.out) * 2.0;
        zebras.push((mid, g.out, g.left_w, g.right_w));
        crossing_arms.push((i, mid));
        for left in [true, false] {
            let lane = plate.lanes.iter().filter(|l| l.arm == i && l.kind == 1)
                .filter_map(|l| {
                    let side = (l.pos.truncate() - g.end.truncate()).dot(r);
                    if (left && side < -0.01) || (!left && side > 0.01) { Some((side.abs(), l)) } else { None }
                })
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, l)| l);
            if let Some(lane) = lane {
                let xy = lane.pos.truncate() + mid - g.end.truncate();
                walk_joins.push(WalkJoin {
                    arm: i, left, at_arm: lane.pos,
                    at_crossing: DVec3::new(xy.x, xy.y, lane.pos.z),
                    width: lane.width,
                });
            }
        }
    }
    let id = doc.new_id();
    let folder: String = doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "map".into()).chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let sco_root = std::fs::read_dir(content)?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case("sceneryobjects")).unwrap_or(false)).unwrap_or_else(|| content.join("Sceneryobjects"));
    let dir = sco_root.join("openMapper").join(&folder);
    let tex_dir = dir.join("texture");
    std::fs::create_dir_all(dir.join("model"))?;
    std::fs::create_dir_all(&tex_dir)?;
    let core_tex = junction_asphalt(Some(&doc.dir), asphalt);
    let along = asphalt_heading(&plate.arms);
    let (mut mesh, kerb_corners) = junction_mesh(&plate.arms, DVec3::ZERO, core_tex, ASPHALT_PERIOD, along);
    let kerb_lead = corner_leads(&kerb_corners);
    let zebra_arms: Vec<bool> = plate.arms.iter().map(|g| opts.crossing_at(g.out)).collect();
    let mut recs = paths(
        &plate.lanes, DVec3::ZERO, &plate_lights, &walk_joins, opts.corner_walks,
        &kerb_lead, &zebra_arms, &kerb_corners, doc.global.left_hand_traffic,
    );
    // Maintain the asphalt height across the roadway, and join it to raised
    // sidewalks with short graded paths at either end.
    for (i, mid) in crossing_arms {
        let g = &plate.arms[i];
        let r = roads::right(g.out);
        let (a, b) = (mid - r * (g.left_w + 0.5), mid + r * (g.right_w + 0.5));
        let z = g.end.z + g.road_z;
        if let Some(seg) = roads::straight(a, b) {
            recs.push(PathRec { seg, z0: z, z1: z, kind: 1, width: 3.0, direction: 2, blinker: 0, light: None, pocket: false });
            for (left, edge) in [(true, a), (false, b)] {
                if let Some(join) = walk_joins.iter().find(|w| w.arm == i && w.left == left) {
                    if let Some(seg) = roads::straight(edge, join.at_crossing.truncate()) {
                        recs.push(PathRec {
                            seg, z0: z, z1: join.at_crossing.z, kind: 1,
                            width: join.width, direction: 2, blinker: 0, light: None, pocket: false,
                        });
                    }
                    // The corner path starts at at_crossing; this additional
                    // two-metre stub links it back to the road's original
                    // sidewalk lane at the spline end, even when corners are off.
                    if let Some(seg) = roads::straight(join.at_arm.truncate(), join.at_crossing.truncate()) {
                        recs.push(PathRec {
                            seg, z0: join.at_arm.z, z1: join.at_crossing.z,
                            kind: 1, width: join.width, direction: 2,
                            blinker: 0, light: None, pocket: false,
                        });
                    }
                }
            }
        }
    }

    if opts.zebra_texture && !zebras.is_empty() {
        let stripe_tex = if let Some(texture) = map_texture(&doc.dir, MAP_ZEBRA) {
            log::info!("junction zebra texture: {texture:?} (map override)");
            texture
        } else {
            let white = tex_dir.join("om_zebra_white.png");
            if !white.exists() {
                let _ = image::RgbaImage::from_pixel(8, 8, image::Rgba([235, 235, 230, 255])).save(&white);
            }
            log::info!("junction zebra texture: {white:?} (default; map override unavailable)");
            white
        };
        let mean_end_z = plate.arms.iter().map(|g| g.end.z).sum::<f64>() / plate.arms.len().max(1) as f64;
        let mean_road_z = plate.arms.iter().map(|g| g.road_z).sum::<f64>() / plate.arms.len().max(1) as f64;
        paint_zebra_stripes(&mut mesh, &zebras, mean_end_z + mean_road_z + 0.03, stripe_tex);
    }
    // every texture beside the object
    let mut names = Vec::new();
    for t in &mesh.textures {
        let Some(p) = t else {
            names.push(String::new());
            continue;
        };
        names.push(export_texture(p, &tex_dir));
    }
    // (the plate is written from what it is made of, in its own frame: junctions that come out
    // the same share one object file, one model and one set of textures, so the game loads and
    // draws it once. Named by what is in it)
    let x_text = mesh_x(&mesh, &names);
    let mut body = String::new();
    if plate_lights.iter().any(|l| l.is_some()) {
        body.push_str(&light_program(opts));
    }
    // where roads join it (the editors snap to these)
    for (g, i) in plate.arms.iter().zip(&plate.order) {
        let file = &files[*i];
        let p = g.end;
        // (OMSI's order: x, height, forward)
        let _ = write!(body, "[splinehelper]\n{file}\n{}\n{}\n{}\n{}\n0.1\n0\n\n", num(p.x), num(p.z), num(p.y), num(roads::norm_deg(g.out + 180.0)));
    }
    write_junction_paths(&mut body, &recs);
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&x_text, &body, opts.line()).hash(&mut h);
        h.finish()
    };
    let name = format!("junction_{hash:016x}");
    let model_path = dir.join("model").join(format!("{name}.x"));
    let sco_path = dir.join(format!("{name}.sco"));
    if !(model_path.is_file() && sco_path.is_file()) {
        std::fs::write(&model_path, &x_text)?;
        let mut sco = String::new();
        let _ = write!(sco, "{}\n\n[friendlyname]\nJunction {}\n\n[groups]\n1\nopenMapper\n\n[surface]\n\n[absheight]\n\n[mesh]\n{name}.x\n\n", opts.line(), &format!("{hash:016x}")[..6]);
        sco.push_str(&body);
        std::fs::write(&sco_path, omsi_cfg::encode_text(&sco.replace('\n', "\r\n")))?;
    }
    crate::hot::own_change();
    let k = tile_of(centre.truncate());
    let k = if doc.tiles.contains_key(&k) { k } else { arms[0].key };
    let org = tile_origin(k);
    if let Some(history) = edit.history.as_deref_mut() {
        history.touch_tile(doc, k);
    }
    let rel = format!("Sceneryobjects\\openMapper\\{folder}\\{name}.sco");
    let t = doc.tiles.get_mut(&k).ok_or_else(|| anyhow::anyhow!("no tile"))?;
    let rules = recs.iter().enumerate().filter(|(_, p)| p.pocket).flat_map(|(index, _)| {
        ["bus", "no_cars"].map(|kind| omsi_map::MapRule {
            path_index: index as i32, kind: kind.into(), ..Default::default()
        })
    }).collect();
    t.tile.objects.push(MapObject { file: rel, id, pos: [centre.x - org.x, centre.y - org.y, centre.z], rot: [roads::norm_deg(plate.turn), 0.0, 0.0], rules, ..Default::default() });
    t.touch_content();
    // Establish the final terrain before storing the heights of ground-relative objects.
    if let Some(min_reach) = edit.conform_min_reach {
        let ends: Vec<_> = geo.iter().map(|g| g.end).collect();
        let reach = reach(doc, assets, centre, arms).max(min_reach);
        crate::editor::conform_disc(doc, centre, reach, &ends);
    }
    if opts.light_objects && light_of.iter().any(Option::is_some) {
        let (set, reason) = doc.signal_sets.resolve(&opts.signal_set);
        log::info!("junction {id} signal set: requested={:?}, used={:?}, reason={reason}", opts.signal_set, set.id);
        let set = set.clone();
        let height_modes = set.height_modes(assets);
        let left_hand = doc.global.left_hand_traffic;
        for (i, g) in geo.iter().enumerate() {
            let Some(light) = light_of[i] else { continue };
            if !lanes.iter().any(|l| l.arm == i && l.kind == 0 && l.incoming) {
                continue;
            }
            // (traffic coming in keeps to the left of the way out where it keeps right)
            let side = if left_hand { roads::right(g.out) * (g.right_w + 1.2) } else { -roads::right(g.out) * (g.left_w + 1.2) };
            let p = g.end.truncate() + side - roads::dir(g.out) * 1.0;
            let head = p.extend(g.end.z + g.road_z + set.head_height);
            let at = crate::signal_sets::Placement { junction: id, light, heading: g.out, head };
            set.place(doc, edit.history.as_deref_mut(), at, height_modes);
        }
    }
    Ok(id)
}

/// The folder a map's junction objects are written in (`Sceneryobjects\openMapper\<folder>`
/// in the content folder), made with its `model` and `texture` folders.
pub fn object_dir(content: &Path, folder: &str) -> std::io::Result<PathBuf> {
    let sco_root = std::fs::read_dir(content)?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case("sceneryobjects")).unwrap_or(false)).unwrap_or_else(|| content.join("Sceneryobjects"));
    let dir = sco_root.join("openMapper").join(folder);
    std::fs::create_dir_all(dir.join("model"))?;
    std::fs::create_dir_all(dir.join("texture"))?;
    Ok(dir)
}

/// The name of a map's folder as its junction objects' folder is named.
pub fn folder_name(doc: &Document) -> String {
    doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "map".into()).chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// Snapshot a generated plate into the OMSI installation with its own model and textures.
/// The SCO bytes carry the original paths, light program, and spline-helper convention.
pub fn persist(roots: &crate::install::Roots, file: &str, map: &str, id: i64) -> anyhow::Result<String> {
    use anyhow::Context as _;
    use crate::doc::safe::own_path;

    anyhow::ensure!(!map.contains(['/', '\\', ':']), "Invalid map folder name: {map}");
    let source = roots.resolve(file);
    let read = |path: &Path| omsi_cfg::vfs::read(path).with_context(|| path.display().to_string());
    let sco_bytes = read(&source)?;
    let source_dir = source.parent().context("junction source directory")?;
    let sco = omsi_scenery::SceneryObject::parse(&omsi_cfg::CfgFile::from_bytes(&source, &sco_bytes));
    let [mesh] = sco.model.meshes.as_slice() else {
        anyhow::bail!("Expected one generated junction mesh: {}", source.display());
    };
    anyhow::ensure!(!mesh.file.contains(['/', '\\', ':']) && mesh.file != "." && mesh.file != "..", "Invalid junction mesh name: {}", mesh.file);
    let model_dir = omsi_cfg::resolve_path(source_dir, "model");
    let model = omsi_cfg::resolve_path(&model_dir, &mesh.file);
    let model_bytes = read(&model)?;
    let parsed = omsi_o3d::load_mesh(&model).with_context(|| model.display().to_string())?;

    let dirs = crate::assets::texture_dirs(&roots.omsi, &roots.content, source_dir);
    let dirs: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
    let mut textures = std::collections::BTreeMap::new();
    let seasons = [None, Some("Spring"), Some("Fall"), Some("Winter"), Some("WinterSnow"), Some("WinterSnowfall"), Some("SummerDry")];
    for mat in parsed.materials.iter().filter(|m| !m.texture.trim().is_empty()) {
        let mut found = false;
        for season in seasons {
            let Some(tex) = omsi_texture::find_texture_in_look(&mat.texture, &dirs, season) else { continue };
            found = true;
            let mut dest = PathBuf::from("texture");
            if let Some(sub) = tex.parent().and_then(Path::file_name).and_then(|f| f.to_str()) {
                if seasons.iter().flatten().any(|s| s.eq_ignore_ascii_case(sub)) {
                    dest.push(sub);
                }
            }
            for path in std::iter::once(tex.clone()).chain(companions(&tex)) {
                textures.entry(dest.join(path.file_name().unwrap())).or_insert(path);
            }
        }
        anyhow::ensure!(found, "{}: {}", crate::i18n::tr("file not found", "файл не найден"), mat.texture);
    }

    // Reserve a folder per instance. A previous save or another editor can already own
    // that ID's folder, including a copy in the higher-priority content root.
    let base = format!("Sceneryobjects\\openMapper-Persisted\\{map}");
    std::fs::create_dir_all(own_path(&roots.omsi, &base))?;
    let mut serial = 1;
    let (rel_dir, dir) = loop {
        let suffix = if serial == 1 { String::new() } else { format!("-{serial}") };
        let rel = format!("{base}\\Junction-{id}{suffix}");
        serial += 1;
        if omsi_cfg::vfs::exists(&roots.resolve(&rel)) {
            continue;
        }
        let dir = own_path(&roots.omsi, &rel);
        match std::fs::create_dir(&dir) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            result => result.with_context(|| dir.display().to_string())?,
        }
        break (rel, dir);
    };

    let name = format!("Junction-{id}.sco");
    let result = (|| -> anyhow::Result<()> {
        let model_dir = dir.join("model");
        std::fs::create_dir(&model_dir)?;
        std::fs::write(model_dir.join(&mesh.file), &model_bytes)?;
        for (rel, source) in textures {
            let dest = dir.join(rel);
            std::fs::create_dir_all(dest.parent().unwrap())?;
            std::fs::write(&dest, read(&source)?).with_context(|| dest.display().to_string())?;
        }
        crate::doc::safe::write_atomic(&dir.join(&name), &sco_bytes)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e).with_context(|| dir.display().to_string());
    }
    crate::hot::own_change();
    Ok(format!("{rel_dir}\\{name}"))
}

/// A road coming into a drawn junction: its road type, where it ends (in the drawing's frame)
/// and the heading out along it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateArm {
    pub file: String,
    pub pos: DVec2,
    pub out: f64,
    /// The road's spline runs into the junction here (its end is at it); else it runs away
    /// from it (its start is). The road type's left and right and the lanes' ways are the
    /// spline's own.
    pub at_end: bool,
}

/// The plate of a junction drawn in the complex junction editor: the roadway, the kerbs round
/// each corner and the footways, as the Roads tool makes them, and the lanes through it.
pub struct Plate {
    mesh: Mesh,
    /// Its `[path]` records (drawing's frame).
    pub paths: String,
    /// The same lanes as lines (drawing's frame): the way along it, the path's kind and its
    /// direction (0 along the line, 1 against, 2 both).
    pub lanes: Vec<(Seg, i32, i32)>,
}

/// The plate between `arms` using the caller's asphalt texture. None when fewer than
/// two arms, or a road type is not loaded.
pub fn layout_plate(assets: &mut Assets, arms: &[PlateArm], asphalt: Option<&Path>) -> Option<Plate> {
    if arms.len() < 2 {
        return None;
    }
    let (mut geo, mut lanes) = (Vec::new(), Vec::new());
    for (i, a) in arms.iter().enumerate() {
        let model = assets.spline(&a.file)?;
        let end = DVec3::new(a.pos.x, a.pos.y, 0.0);
        // (the spline's own way there: into the junction, or away from it)
        let along = if a.at_end { roads::norm_deg(a.out + 180.0) } else { a.out };
        let r = roads::right(along);
        geo.push(arm_geo_of(&model, false, a.at_end, end, a.out, 0.0));
        lanes.extend(lanes_at(&model, false, i, a.at_end, |x, z| end + DVec3::new(r.x * x, r.y * x, z), along, None));
    }
    // (made round the junction's own middle, then moved to the drawing's frame)
    let mid = arms.iter().map(|a| a.pos).sum::<DVec2>() / arms.len() as f64;
    let centre = DVec3::new(mid.x, mid.y, 0.0);
    // The complex plate editor has no automatic zebra crossings.
    // Build the kerbs first, and reuse their exact tangent lengths for AI paths.
    let core_tex = asphalt.map(Path::to_path_buf);
    let along = asphalt_heading(&geo);
    let (mut mesh, kerb_corners) = junction_mesh(&geo, centre, core_tex, ASPHALT_PERIOD, along);
    let kerb_lead = corner_leads(&kerb_corners);
    let recs = paths(&lanes, centre, &vec![None; arms.len()], &[], true,
        &kerb_lead, &vec![false; arms.len()], &kerb_corners, false);
    for v in &mut mesh.verts {
        v.0[0] += mid.x;
        v.0[1] += mid.y;
    }
    let mut paths = String::new();
    for p in &recs {
        let s = &p.seg;
        let start = s.start + mid;
        let grad = if s.length > 0.01 { (p.z1 - p.z0) / s.length } else { 0.0 };
        let _ = write!(paths, "[path]\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n", num(start.x), num(start.y), num(p.z0), num(s.heading), num(s.radius), num(s.length), num(grad), num(grad), p.kind, num(p.width), p.direction, p.blinker);
    }
    let lanes = recs.iter().map(|p| (Seg { start: p.seg.start + mid, ..p.seg }, p.kind, p.direction)).collect();
    Some(Plate { mesh, paths, lanes })
}

impl Plate {
    /// The plate's triangles seen from above for drawing them with their pictures: each with
    /// its texture (None: the roadway, which the drawing paints flat), and the uv at its corners.
    pub fn flat_textured(&self) -> Vec<([DVec2; 3], Option<PathBuf>, [[f32; 2]; 3])> {
        self.mesh
            .tris
            .iter()
            .map(|(t, mat)| {
                let v = |i: usize| &self.mesh.verts[t[i]];
                let tex = self.mesh.textures.get(*mat).cloned().flatten();
                ([0, 1, 2].map(|i| DVec2::new(v(i).0[0], v(i).0[1])), tex, [0, 1, 2].map(|i| [v(i).2[0] as f32, v(i).2[1] as f32]))
            })
            .collect()
    }

    /// The plate as a mesh of an editable object: its materials added to `materials`, its
    /// textures (name to file) to `textures`.
    pub fn model_mesh(&self, materials: &mut Vec<openmapper_mesh::Material>, textures: &mut std::collections::HashMap<String, PathBuf>) -> openmapper_mesh::Mesh {
        let slot: Vec<u32> = self
            .mesh
            .textures
            .iter()
            .map(|t| {
                let name = t.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string());
                if let (Some(n), Some(p)) = (&name, t) {
                    textures.entry(n.clone()).or_insert_with(|| p.clone());
                }
                let mat = openmapper_mesh::Material { name: name.clone().unwrap_or_else(|| "Plate".into()), color: [1.0, 1.0, 1.0, 1.0], texture: name, ..Default::default() };
                let i = materials.iter().position(|x| x.texture == mat.texture && x.color == mat.color && x.alpha == mat.alpha).unwrap_or_else(|| {
                    materials.push(mat);
                    materials.len() - 1
                });
                i as u32
            })
            .collect();
        let mut out = openmapper_mesh::Mesh::default();
        for (t, mat) in &self.mesh.tris {
            let ids: Vec<u32> = t.iter().map(|&i| out.add_vertex(glam::Vec3::new(self.mesh.verts[i].0[0] as f32, self.mesh.verts[i].0[1] as f32, self.mesh.verts[i].0[2] as f32))).collect();
            let uvs: Vec<glam::Vec2> = t.iter().map(|&i| glam::Vec2::new(self.mesh.verts[i].2[0] as f32, self.mesh.verts[i].2[1] as f32)).collect();
            out.add_face(ids, uvs, slot.get(*mat).copied().unwrap_or(0));
        }
        out.weld(1e-5);
        out
    }

    /// Written as a scenery object of its own (`Sceneryobjects\openMapper\<folder>\junction_<hash>.sco`)
    /// for a drawing put on the map: its relative path.
    pub fn write_object(&self, content: &Path, folder: &str) -> anyhow::Result<String> {
        let dir = object_dir(content, folder)?;
        let tex_dir = dir.join("texture");
        let mut names = Vec::new();
        for t in &self.mesh.textures {
            let Some(p) = t else {
                names.push(String::new());
                continue;
            };
            names.push(export_texture(p, &tex_dir));
        }
        let x_text = mesh_x(&self.mesh, &names);
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (&x_text, &self.paths).hash(&mut h);
            h.finish()
        };
        let name = format!("junction_{hash:016x}");
        let (model_path, sco_path) = (dir.join("model").join(format!("{name}.x")), dir.join(format!("{name}.sco")));
        if !(model_path.is_file() && sco_path.is_file()) {
            std::fs::write(&model_path, &x_text)?;
            let sco = format!("{}\n\n[friendlyname]\nJunction {}\n\n[groups]\n1\nopenMapper\n\n[surface]\n\n[absheight]\n\n[mesh]\n{name}.x\n\n{}", Options::default().line(), &format!("{hash:016x}")[..6], self.paths);
            std::fs::write(&sco_path, omsi_cfg::encode_text(&sco.replace('\n', "\r\n")))?;
        }
        crate::hot::own_change();
        Ok(format!("Sceneryobjects\\openMapper\\{folder}\\{name}.sco"))
    }
}

/// Delete the junction files (`junction_<id>.sco` and its `.x`) in `dir` whose object file
/// (`Sceneryobjects\\openMapper\\<folder>\\junction_<id>.sco`, lower case) is not in `used`,
/// leaving those numbered from `keep_from` on (made in this session: undo may need them).
pub fn remove_unused(dir: &Path, folder: &str, used: &std::collections::HashSet<String>, keep_from: i64) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let lower = name.to_lowercase();
        if !(lower.starts_with("junction_") && lower.ends_with(".sco")) {
            continue;
        }
        let rel = format!("sceneryobjects\\openmapper\\{}\\{lower}", folder.to_lowercase());
        let num: Option<i64> = lower["junction_".len()..lower.len() - 4].parse().ok();
        // (one named by its contents is shared; a recent one may be wanted by undo)
        let recent = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs() < 86_400).unwrap_or(true);
        if used.contains(&rel) || num.map(|n| n >= keep_from).unwrap_or(recent) {
            continue;
        }
        let stem = &name[..name.len() - 4];
        let _ = std::fs::remove_file(e.path());
        let _ = std::fs::remove_file(dir.join("model").join(format!("{stem}.x")));
        n += 1;
    }
    if n > 0 {
        crate::hot::own_change();
    }
    n
}

/// The objects that belong to junction `id` (its signal heads and poles).
pub fn children(doc: &Document, id: i64) -> Vec<(TileKey, i64)> {
    doc.tiles.iter().flat_map(|(k, t)| t.tile.objects.iter().filter(|o| o.var_parent == Some(id)).map(move |o| (*k, o.id))).collect()
}

/// Where two polylines cross: (point, distance along a, distance along b).
pub fn crossings(a: &[(DVec2, f64)], b: &[(DVec2, f64)]) -> Vec<(DVec2, f64, f64)> {
    let mut out = Vec::new();
    for wa in a.windows(2) {
        for wb in b.windows(2) {
            let (p, r) = (wa[0].0, wa[1].0 - wa[0].0);
            let (q, s) = (wb[0].0, wb[1].0 - wb[0].0);
            let rxs = r.perp_dot(s);
            if rxs.abs() < 1e-9 {
                continue;
            }
            let t = (q - p).perp_dot(s) / rxs;
            let u = (q - p).perp_dot(r) / rxs;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                let pt = p + r * t;
                // (a crossing right on a joint of the lines is found on both sides of it)
                if !out.iter().any(|o: &(DVec2, f64, f64)| (o.0 - pt).length() < 0.01) {
                    out.push((pt, wa[0].1 + (wa[1].1 - wa[0].1) * t, wb[0].1 + (wb[1].1 - wb[0].1) * u));
                }
            }
        }
    }
    out
}

/// A spline's centre line every 2 m: (point, distance along it).
pub fn polyline(c: &SplineCurve) -> Vec<(DVec2, f64)> {
    let n = (c.length / 2.0).ceil().max(1.0) as usize;
    (0..=n).map(|i| {
        let s = c.length * i as f64 / n as f64;
        (c.point_at(s).truncate(), s)
    }).collect()
}

const MAP_ASPHALT: &str = "plain_asphalt_texture.bmp";
const MAP_ZEBRA: &str = "zebra_texture.bmp";
const ASPHALT_PERIOD: f64 = 8.0;

/// Find a file in the map's texture folder, ignoring case in both names.
pub fn map_texture(map_dir: &Path, name: &str) -> Option<PathBuf> {
    let dir = std::fs::read_dir(map_dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.is_dir() && p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case("texture"))
    })?;
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.is_file() && p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name))
    })
}

pub fn map_asphalt(map_dir: &Path) -> Option<PathBuf> {
    map_texture(map_dir, MAP_ASPHALT)
}

/// Prefer the map's asphalt to the installation's junction texture.
pub fn junction_asphalt(map_dir: Option<&Path>, installed: Option<&Path>) -> Option<PathBuf> {
    let map = map_dir.and_then(map_asphalt);
    let reason = if map.is_some() {
        "map override"
    } else if installed.is_some() {
        "installation fallback; map override unavailable"
    } else {
        "map override and installation texture unavailable"
    };
    let chosen = map.or_else(|| installed.map(Path::to_path_buf));
    log::info!("junction asphalt texture: {chosen:?} ({reason})");
    chosen
}

/// The asphalt texture junction plates are drawn with: the installation's own.
pub fn asphalt_texture(omsi: &Path) -> Option<PathBuf> {
    let dir = std::fs::read_dir(omsi).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case("texture")).unwrap_or(false))?;
    for name in ["str_asphdrk.bmp", "str_asphalt.bmp", "asphalt.bmp"] {
        if let Some(p) = std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false)) {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn options_are_read_back_from_the_line_written() {
        let o = Options { lights: true, light_objects: true, main_green: 42.0, side_green: 17.0, crossings: vec![90.0, 270.5], zebra_texture: false, corner_walks: false, signal_set: "mc-small_2".into() };
        let text = format!("{}\r\n\r\n[friendlyname]\r\nJunction 1\r\n", o.line());
        let back = Options::parse(&text);
        println!("Options before writing: {o:?}\nSCO:\n{text}Parsed: {back:?}");
        assert_eq!(back, o, "all junction options must round-trip through .sco");
        assert_eq!((back.main_green, back.side_green), (42.0, 17.0));
        assert!(back.crossing_at(91.0) && back.crossing_at(270.0) && !back.crossing_at(0.0));
        assert!(back.light_objects);
        assert!(!back.zebra_texture);
        assert!(!Options::parse(&text).corner_walks);
        let legacy = "openMapper junction: lights=1 main=42 side=17 crossings=90.0,270.5 helpers=omsi";
        let legacy_options = Options::parse(legacy);
        assert!(legacy_options.corner_walks, "old .sco without walks= must retain corner paths");
        assert!(legacy_options.light_objects, "older integration junctions with lights=1 placed signal models");
        assert!(legacy_options.zebra_texture, "old .sco without zebra_tex= must keep zebra stripes");
        assert!(!Options::default().light_objects);
        assert!(Options::default().zebra_texture);
        assert_eq!(Options::parse("[friendlyname]\nx"), Options::default());
        println!("OPTIONS ROUND TRIP: {} => {:?}", o.line(), back);
        println!("LEGACY: {legacy} => corner_walks={}", Options::parse(legacy).corner_walks);
    }

    #[test]
    fn the_main_road_is_the_pair_most_in_line_and_its_light_comes_first() {
        assert_eq!(main_road(&[0.5, 90.0, 270.0]), (1, 2));
        let p = light_program(&Options { lights: true, main_green: 30.0, side_green: 20.0, crossings: vec![], ..Default::default() });
        // a cycle of both greens, their yellows, red-yellows and all-red times
        assert!(p.starts_with("[traffic_lights_group]\n64\n"), "{p}");
        assert_eq!(p.matches("[traffic_light]").count(), 2);
    }

    #[test]
    fn only_junction_files_nothing_uses_from_before_are_removed() {
        let dir = std::env::temp_dir().join(format!("om-gc-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("model")).unwrap();
        for n in [5, 6, 50] {
            std::fs::write(dir.join(format!("junction_{n}.sco")), "x").unwrap();
            std::fs::write(dir.join("model").join(format!("junction_{n}.x")), "x").unwrap();
        }
        std::fs::write(dir.join("other.sco"), "x").unwrap();
        let used: std::collections::HashSet<String> = ["sceneryobjects\\openmapper\\m\\junction_6.sco".to_string()].into();
        // 5 unused and old: removed; 6 used; 50 made this session (from 40 on)
        assert_eq!(remove_unused(&dir, "m", &used, 40), 1);
        assert!(!dir.join("junction_5.sco").exists() && !dir.join("model/junction_5.x").exists());
        assert!(dir.join("junction_6.sco").exists() && dir.join("junction_50.sco").exists() && dir.join("other.sco").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    #[test]
    fn two_roads_crossing_meet_once() {
        let a: Vec<(DVec2, f64)> = (0..=10).map(|i| (DVec2::new(i as f64 * 10.0 - 50.0, 0.0), i as f64 * 10.0)).collect();
        let b: Vec<(DVec2, f64)> = (0..=10).map(|i| (DVec2::new(5.0, i as f64 * 10.0 - 50.0), i as f64 * 10.0)).collect();
        let x = crossings(&a, &b);
        assert_eq!(x.len(), 1);
        assert!((x[0].0 - DVec2::new(5.0, 0.0)).length() < 1e-9);
        assert!((x[0].1 - 55.0).abs() < 1e-9 && (x[0].2 - 50.0).abs() < 1e-9);
    }

    #[test]
    fn a_kerb_rounds_a_corner_smoothly() {
        // an arm running out east with its left kerb 4 m north of the centre line, ending
        // 12 m out; the next arm running out north with its right kerb 4 m east
        let a = DVec2::new(12.0, 4.0);
        let b = DVec2::new(4.0, 12.0);
        let segs = kerb_round(a, 90.0, b, 0.0);
        assert!(!segs.is_empty(), "{segs:?}");
        let end = segs.last().unwrap();
        assert!((end.end() - b).length() < 1e-6, "ends at {:?}", end.end());
        for w in segs.windows(2) {
            assert!((w[0].end() - w[1].start).length() < 1e-6);
            assert!((roads::norm_deg(w[0].end_heading() - w[1].heading + 180.0) - 180.0).abs() < 1e-6);
        }
        // it rounds the corner through its inside
        let mid = segs[segs.len() / 2].point(segs[segs.len() / 2].length * 0.5);
        assert!(mid.x > 4.0 && mid.y > 4.0 && mid.x < 12.0 && mid.y < 12.0, "{mid:?}");
    }

    #[test]
    fn mirrored_junction_lanes_follow_the_placed_spline() {
        let roots = crate::doc::tests::scratch("junction-mirrored-lanes");
        let mut doc = crate::selection::tests::flat_map(1);
        let key = (0, 0);
        let id = 1;
        crate::selection::tests::add_spline(&mut doc, key, id, 100.0, 100.0, 0.0, 20.0, 0, 0);
        let mut road = test_road();
        road.def.paths.push(omsi_scenery::PathDef {
            kind: 1,
            start: [3.0, 0.0, 0.25],
            width: 1.0,
            direction: 2,
            ..Default::default()
        });
        let mut assets = Assets::new(roots.omsi, roots.content);
        assets.splines.insert("splines\\road.sli".into(), crate::assets::Slot::Ready(std::sync::Arc::new(road)));

        // Columns: world x, world y, height, heading, and whether the lane enters the junction.
        let cases = [
            (false, false, vec![
                (102.0, 100.0, 0.0, 0.0, false),
                (98.0, 100.0, 0.0, 180.0, true),
                (103.0, 100.0, 0.25, 180.0, true),
                (103.0, 100.0, 0.25, 0.0, false),
            ]),
            (false, true, vec![
                (102.0, 120.0, 0.0, 0.0, true),
                (98.0, 120.0, 0.0, 180.0, false),
                (103.0, 120.0, 0.25, 0.0, true),
                (103.0, 120.0, 0.25, 180.0, false),
            ]),
            (true, false, vec![
                (98.0, 100.0, 0.0, 180.0, true),
                (102.0, 100.0, 0.0, 0.0, false),
                (97.0, 100.0, 0.25, 180.0, true),
                (97.0, 100.0, 0.25, 0.0, false),
            ]),
            (true, true, vec![
                (98.0, 120.0, 0.0, 180.0, false),
                (102.0, 120.0, 0.0, 0.0, true),
                (97.0, 120.0, 0.25, 0.0, true),
                (97.0, 120.0, 0.25, 180.0, false),
            ]),
        ];
        for left_hand in [false, true] {
            doc.global.left_hand_traffic = left_hand;
            for (mirrored, at_end, expected) in &cases {
                doc.tiles.get_mut(&key).unwrap().tile.splines[0].mirror = *mirrored;
                let (_, lanes) = arm_ends(&doc, &mut assets, &[Arm { key, id, at_end: *at_end }], None).unwrap();
                let actual: Vec<_> = lanes.iter().map(|l| (l.pos.x, l.pos.y, l.pos.z, l.flow, l.incoming)).collect();
                println!("left_hand={left_hand}, mirror={mirrored}, at_end={at_end}\n  actual: {actual:?}\n  expected: {expected:?}");
                assert_eq!(&actual, expected);
            }
        }
    }

    fn lane_connectivity_records(geo: &[ArmGeo], lanes: &[LaneEnd]) -> (Vec<PathRec>, Vec<PathRec>) {
        let lights: Vec<_> = (0..geo.len()).map(Some).collect();
        let (_, corners) = junction_mesh(geo, DVec3::ZERO, None, 8.0, 0.0);
        let leads = corner_leads(&corners);
        let zebras = vec![true; geo.len()];
        (
            paths_with_lane_repair(lanes, DVec3::ZERO, &lights, &[], true,
                &leads, &zebras, &corners, false, false),
            paths(lanes, DVec3::ZERO, &lights, &[], true, &leads, &zebras, &corners, false),
        )
    }

    fn lane_connectivity_blocks(records: &[PathRec]) -> String {
        let mut body = String::new();
        write_junction_paths(&mut body, records);
        body
    }

    fn lane_connectivity_arm(
        arm: usize, out: f64, at_end: bool, profile: &[(f32, f32, i32)],
    ) -> (ArmGeo, Vec<LaneEnd>) {
        let mut model = test_road();
        model.def.paths = profile.iter().map(|&(x, width, direction)| omsi_scenery::PathDef {
            kind: 0, start: [x, 0.0, 0.0], width, direction, ..Default::default()
        }).collect();
        let half_width = if profile.len() == 4 { 12.0 } else { 4.0 };
        model.half_width = half_width;
        for (point, x) in model.def.profiles[0].points.iter_mut().zip([-half_width, half_width]) {
            point.x = x;
        }
        let end = (roads::dir(out) * if profile.len() == 4 { 24.0 } else { 20.0 }).extend(0.0);
        let along = if at_end { roads::norm_deg(out + 180.0) } else { out };
        let lanes = lanes_at(&model, false, arm, at_end,
            |x, z| end + (roads::right(along) * x).extend(z), along, None);
        (arm_geo_of(&model, false, at_end, end, out, 0.0), lanes)
    }

    fn lane_connectivity_fixture(t_junction: bool, wide: bool) -> (Vec<ArmGeo>, Vec<LaneEnd>) {
        let two_way = [(-2.0, 2.5, 1), (2.0, 2.5, 0)];
        let one_way = [(-2.0, 2.5, 0), (2.0, 2.5, 0)];
        let three_spur = [
            (-9.625, 3.03, 0), (-6.375, 3.03, 1), (0.0, 4.04, 1), (7.0, 4.04, 1),
        ];
        let mut geo = Vec::new();
        let mut lanes = Vec::new();
        for out in [0.0, 90.0, 180.0, 270.0] {
            if t_junction && out == 180.0 {
                continue;
            }
            let profile: &[_] = if wide && out % 180.0 == 0.0 {
                &three_spur
            } else if t_junction && out != 0.0 {
                &one_way
            } else {
                &two_way
            };
            let (arm, ends) = lane_connectivity_arm(geo.len(), out, !t_junction || out != 270.0, profile);
            geo.push(arm);
            lanes.extend(ends);
        }
        (geo, lanes)
    }

    // Generated segments are emitted consecutively for each lane-to-lane connection.
    // Match their ends back to the original directed lane ends.
    fn lane_connectivity_report(label: &str, lanes: &[LaneEnd], records: &[PathRec]) -> Vec<Vec<usize>> {
        let mut links = vec![Vec::new(); lanes.len()];
        let mut start = None;
        let mut previous: Option<DVec3> = None;
        for record in records.iter().filter(|record| record.kind == 0) {
            let seg = &record.seg;
            let matches = |lane: &LaneEnd, p: DVec2, z: f64| {
                lane.kind == 0 && lane.pos.distance_squared(p.extend(z)) < 1e-6
            };
            if let Some(pos) = previous {
                assert!(pos.distance_squared(seg.start.extend(record.z0)) < 1e-6);
            }
            let source = *start.get_or_insert_with(|| lanes.iter()
                .position(|lane| lane.incoming && matches(lane, seg.start, record.z0)).unwrap());
            if let Some(target) = lanes.iter().position(|lane| !lane.incoming && matches(lane, seg.end(), record.z1)) {
                links[source].push(target);
                links[target].push(source);
                start = None;
                previous = None;
            } else {
                previous = Some(seg.end().extend(record.z1));
            }
        }
        assert!(start.is_none(), "unfinished route in {label}");
        println!("{label}");
        for (idx, lane) in lanes.iter().enumerate() {
            let peers: Vec<_> = links[idx].iter().map(|&i| (lanes[i].arm, lanes[i].pos)).collect();
            println!("  arm {} lane {idx} {:?} {} {peers:?}", lane.arm, lane.pos,
                if lane.incoming { "out to" } else { "in from" });
        }
        links
    }

    #[test]
    fn lane_connectivity_fills_t_junction_exits() {
        let (geo, lanes) = lane_connectivity_fixture(true, true);
        let (before, after) = lane_connectivity_records(&geo, &lanes);
        let old = lane_connectivity_report("T junction before", &lanes, &before);
        let new = lane_connectivity_report("T junction after", &lanes, &after);
        for x in [0.0, -7.0] {
            let idx = lanes.iter().position(|lane| lane.arm == 0 && !lane.incoming && lane.pos.x == x).unwrap();
            assert!(old[idx].is_empty());
            assert_eq!(new[idx].len(), 1);
            let source = &lanes[new[idx][0]];
            assert_eq!(source.arm, 1);
            assert!(after[before.len()..].iter().any(|record| {
                record.seg.start.distance(source.pos.truncate()) < 1e-6
                    && record.blinker == 3 && record.light == Some(source.arm)
            }));
        }
        assert!(new.iter().all(|links| !links.is_empty()));
        let (old_sco, new_sco) = (lane_connectivity_blocks(&before), lane_connectivity_blocks(&after));
        println!("T junction before [path]\n{old_sco}T junction after [path]\n{new_sco}");
        assert!(new_sco.starts_with(&old_sco));
    }

    #[test]
    fn lane_connectivity_fills_t_junction_entrances() {
        let (geo, mut lanes) = lane_connectivity_fixture(true, true);
        for lane in &mut lanes {
            lane.incoming = !lane.incoming;
            lane.side = -lane.side;
            lane.flow = roads::norm_deg(lane.flow + 180.0);
        }
        let (before, after) = lane_connectivity_records(&geo, &lanes);
        let old = lane_connectivity_report("Reversed T junction before", &lanes, &before);
        let new = lane_connectivity_report("Reversed T junction after", &lanes, &after);
        assert!(old.iter().any(Vec::is_empty));
        assert!(new.iter().all(|links| !links.is_empty()));
        for (idx, lane) in lanes.iter().enumerate().filter(|(idx, _)| old[*idx].is_empty()) {
            assert!(lane.incoming);
            assert_eq!(new[idx].len(), 1);
            assert_eq!(lanes[new[idx][0]].arm, 1);
        }
    }

    #[test]
    fn lane_connectivity_keeps_existing_crossroad_paths_byte_identical() {
        for wide in [false, true] {
            let (geo, lanes) = lane_connectivity_fixture(false, wide);
            let (before, after) = lane_connectivity_records(&geo, &lanes);
            let old = lane_connectivity_report(&format!("Crossroads wide={wide} before"), &lanes, &before);
            let new = lane_connectivity_report(&format!("Crossroads wide={wide} after"), &lanes, &after);
            assert!(old.iter().all(|links| !links.is_empty()));
            assert_eq!(old, new);
            let (old_sco, new_sco) = (lane_connectivity_blocks(&before), lane_connectivity_blocks(&after));
            println!("Crossroads wide={wide} before [path]\n{old_sco}Crossroads wide={wide} after [path]\n{new_sco}");
            assert_eq!(old_sco.as_bytes(), new_sco.as_bytes());
        }
    }

    #[test]
    fn lane_connectivity_falls_back_to_the_most_in_line_arm() {
        for entering in [false, true] {
            let headings = if entering { [0.0, 175.0, 20.0, 50.0, 50.0] } else { [0.0, 185.0, 340.0, 310.0, 310.0] };
            let lanes: Vec<_> = headings.into_iter().enumerate().map(|(idx, flow)| LaneEnd {
                arm: idx.min(3), pos: DVec3::ZERO, flow, kind: 0, width: 2.5,
                side: idx as f64, incoming: if idx == 0 { entering } else { !entering }, pocket: false,
            }).collect();
            let same = vec![vec![0], vec![], vec![], vec![]];
            let opposite = vec![vec![], vec![1], vec![2], vec![3, 4]];
            let (incoming, outgoing) = if entering { (&same, &opposite) } else { (&opposite, &same) };
            let mut routes = Vec::new();
            let mut used = vec![true; lanes.len()];
            used[0] = false;
            fill_vehicle_lanes(&lanes, incoming, outgoing, &mut used, &mut routes, |_| true);
            let expected = if entering { (0, 2) } else { (2, 0) };
            let chosen = routes.iter().find(|route| route.lane(entering) == 0).unwrap();
            println!("Fallback incoming={entering}, input={lanes:?}, before: no connections; after: {} -> {}, turn={}, blinker={}",
                chosen.from, chosen.to, chosen.turn, chosen.blinker);
            assert_eq!((chosen.from, chosen.to, chosen.blinker), (expected.0, expected.1, 0));
        }
    }

    fn test_road() -> SplineModel {
        let lane = |x: f32, direction: i32| omsi_scenery::PathDef { kind: 0, start: [x, 0.0, 0.0], width: 3.0, direction, ..Default::default() };
        SplineModel {
            def: omsi_scenery::Spline {
                profiles: vec![omsi_scenery::SplineProfile {
                    texture: 0,
                    points: vec![omsi_scenery::SplineProfilePoint { x: -4.0, z: 0.0, ..Default::default() }, omsi_scenery::SplineProfilePoint { x: 4.0, z: 0.0, ..Default::default() }],
                }],
                paths: vec![lane(2.0, 0), lane(-2.0, 1)],
                ..Default::default()
            },
            materials: Vec::new(),
            half_width: 4.0,
        }
    }

    fn arm_profile_road(profiles: &[[(f32, f32); 2]], lanes: &[f32]) -> SplineModel {
        SplineModel {
            def: omsi_scenery::Spline {
                profiles: profiles.iter().enumerate().map(|(texture, points)| {
                    omsi_scenery::SplineProfile {
                        texture,
                        points: points.iter().enumerate().map(|(i, &(x, z))| omsi_scenery::SplineProfilePoint {
                            x, z, u: i as f32, v_scale: 0.2 + i as f32 * 0.2, ..Default::default()
                        }).collect(),
                    }
                }).collect(),
                paths: lanes.iter().map(|&x| omsi_scenery::PathDef {
                    kind: 0, start: [x, 0.0, 0.1], width: 2.5, ..Default::default()
                }).collect(),
                ..Default::default()
            },
            materials: (0..profiles.len()).map(|i| crate::assets::Material {
                texture: Some(PathBuf::from(format!("profile-{i}.bmp"))),
                color: [1.0; 4],
                alpha: crate::assets::Alpha::Opaque,
                glow: None,
            }).collect(),
            half_width: profiles.iter().flatten().map(|p| p.0.abs()).fold(0.0, f32::max),
        }
    }

    fn log_arm_profiles(name: &str, model: &SplineModel, after: &ArmGeo) {
        // Reproduce the old x-only overlap, whole-profile side filters, and
        // first central profile for these two-point fixtures.
        let profiles: Vec<_> = model.def.profiles.iter().map(|p| {
            let (a, b) = (&p.points[0], &p.points[1]);
            let (a, b) = if a.x <= b.x { (a, b) } else { (b, a) };
            (p.texture, (a.x as f64, a.z as f64), (b.x as f64, b.z as f64))
        }).collect();
        let lanes: Vec<_> = model.def.paths.iter().filter(|p| p.kind == 0)
            .map(|p| (p.start[0] as f64, p.width as f64, p.start[2] as f64)).collect();
        let (lo, hi) = lanes.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &(x, w, _)| {
            (lo.min(x - w / 2.0), hi.max(x + w / 2.0))
        });
        let (xl, xr) = profiles.iter().filter(|(_, a, b)| b.0 > lo + 0.05 && a.0 < hi - 0.05)
            .fold((lo, hi), |(xl, xr), (_, a, b)| (xl.min(a.0), xr.max(b.0)));
        let lane_mid = lanes[0].0;
        let z = profiles.iter().find(|(_, a, b)| a.0 >= xl - 0.05 && b.0 <= xr + 0.05 && a.0 <= lane_mid && b.0 >= lane_mid)
            .map(|(_, a, b)| a.1 + (b.1 - a.1) * (lane_mid - a.0) / (b.0 - a.0)).unwrap_or(0.0);
        let old_side = |right: bool| -> Vec<_> {
            profiles.iter().filter(|(_, a, b)| if right { a.0 >= xr - 0.05 } else { b.0 <= xl + 0.05 })
                .map(|(t, a, b)| (*t, a.0, b.0)).collect()
        };
        let new_side = |profiles: &[SideProfile], right: bool| -> Vec<_> {
            profiles.iter().map(|p| {
                let xs = p.points.iter().map(|q| if right { after.right_w + q.0 } else { -after.left_w - q.0 });
                let (lo, hi) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(x), hi.max(x)));
                (p.texture.clone(), lo, hi)
            }).collect()
        };
        println!("{name} input profiles (slot, (x, z), (x, z)) = {profiles:?}, lanes (x, width, z) = {lanes:?}");
        println!("{name} BEFORE xl={xl:.3} xr={xr:.3} road_z={z:.3} SideProfile x ranges: left={:?} right={:?}",
            old_side(false), old_side(true));
        println!("{name} AFTER  xl={:.3} xr={:.3} road_z={:.3} SideProfile x ranges: left={:?} right={:?}",
            -after.left_w, after.right_w, after.road_z, new_side(&after.left, false), new_side(&after.right, true));
    }

    fn check_arm_profile(name: &str, road: &SplineModel, xl: f64, xr: f64, z: f64, texture: &str) -> ArmGeo {
        let arm = arm_geo_of(road, false, false, DVec3::ZERO, 0.0, 0.0);
        log_arm_profiles(name, road, &arm);
        assert!((-arm.left_w - xl).abs() < 1e-5, "{name}: xl");
        assert!((arm.right_w - xr).abs() < 1e-5, "{name}: xr");
        assert!((arm.road_z - z).abs() < 1e-5, "{name}: road_z");
        assert_eq!(arm.road_tex.as_ref().unwrap().0, PathBuf::from(texture));
        arm
    }

    #[test]
    fn altonaer_arm_extends_asphalt_to_both_kerbs() {
        let road = arm_profile_road(&[
            [(-9.0, 0.25), (-5.65, 0.25)], [(-5.5, 0.25), (-5.5, 0.1)],
            [(-5.5, 0.1), (-4.0, 0.1)], [(-4.0, 0.1), (4.0, 0.1)],
            [(4.0, 0.1), (5.5, 0.1)], [(5.5, 0.1), (5.5, 0.25)],
            [(5.65, 0.25), (9.0, 0.25)],
        ], &[-2.0, 2.0]);
        let arm = check_arm_profile("2spur_11m_altonaer_c", &road, -5.5, 5.5, 0.1, "profile-3.bmp");
        for side in [&arm.left, &arm.right] {
            assert_eq!(side.len(), 2);
            assert!(side.iter().any(|p| p.points.iter().all(|point| point.0 == 0.0)));
        }
    }

    #[test]
    fn half_road_arm_includes_the_lane_without_a_path() {
        let road = arm_profile_road(&[
            [(0.0, 0.1), (3.5, 0.1)], [(3.5, 0.1), (7.0, 0.1)],
            [(7.0, 0.1), (10.5, 0.1)], [(10.5, 0.1), (10.5, 0.4)],
            [(10.5, 0.4), (11.25, 0.4)],
        ], &[5.25, 8.75]);
        let arm = check_arm_profile("3lanes_wbarriernoped_half_2pathsonly", &road, 0.0, 10.5, 0.1, "profile-1.bmp");
        assert!(arm.left.is_empty());
        assert_eq!(arm.right.len(), 2);
    }

    #[test]
    fn bridge_arm_uses_deck_height_and_its_left_edge() {
        let road = arm_profile_road(&[
            [(0.3, -4.0), (12.0, -4.0)], [(0.0, -4.0), (0.3, -0.3)],
            [(-3.6, -0.3), (0.0, -0.3)], [(-3.9, -0.3), (-3.6, 0.4)],
            [(-3.7, 0.5), (-3.5, 0.1)], [(-3.5, 0.1), (0.0, 0.1)],
            [(0.0, 0.1), (4.0, 0.1)],
        ], &[2.0, 6.0, 10.0, 14.0, 24.0, 28.0, 32.0]);
        let arm = check_arm_profile("4lanes_oneway_bridge", &road, -3.5, 33.25, 0.1, "profile-6.bmp");
        let cut = arm.left.iter().find(|p| p.texture.as_deref() == Some(Path::new("profile-2.bmp"))).unwrap();
        let (offset, z, u, v_scale) = cut.points[0];
        assert_eq!(offset, 0.0);
        assert!((z + 0.3).abs() < 1e-5);
        assert!((u - 0.1 / 3.6).abs() < 1e-5);
        assert!((v_scale - (0.2 + 0.2 * 0.1 / 3.6)).abs() < 1e-5);
    }

    #[test]
    fn flyover_arm_keeps_left_asphalt_and_clips_crossing_underside() {
        let road = arm_profile_road(&[
            [(-2.0, 0.1), (-1.0, 0.1)], [(-1.0, 0.1), (2.5, 0.1)],
            [(-1.5, -1.5), (15.15, -1.5)],
        ], &[0.75, 4.25, 9.4, 12.9]);
        let arm = check_arm_profile("4lanes_chathamroad_flyover_2+2", &road, -2.0, 14.15, 0.1, "profile-1.bmp");
        assert!(arm.left.is_empty());
        let cut = arm.right.iter().find(|p| p.texture.as_deref() == Some(Path::new("profile-2.bmp"))).unwrap();
        let (offset, z, u, _) = cut.points[0];
        assert_eq!(offset, 0.0);
        assert!((z + 1.5).abs() < 1e-5);
        assert!((u - 15.65 / 16.65).abs() < 1e-5);
        assert!((cut.points[1].0 - 1.0).abs() < 1e-5);
    }

    #[test]
    fn arm_profile_height_uses_the_middle_two_vehicle_paths() {
        let mut road = arm_profile_road(&[
            [(-4.0, -1.0), (4.0, -1.0)], [(-4.0, 0.2), (4.0, 0.2)],
        ], &[-2.0, -1.0, 1.0, 2.0]);
        for (path, z) in road.def.paths.iter_mut().zip([0.3, 9.0, -8.0, 0.1]) {
            path.start[2] = z;
        }
        road.def.paths.push(omsi_scenery::PathDef { kind: 1, start: [0.0, 0.0, 10.0], width: 1.5, ..Default::default() });
        check_arm_profile("even vehicle path median", &road, -4.0, 4.0, 0.2, "profile-1.bmp");
        road.def.paths.clear();
        let legacy = arm_geo_of(&road, false, false, DVec3::ZERO, 0.0, 0.0);
        println!("no vehicle paths: xl={} xr={} road_z={} texture={:?}", -legacy.left_w, legacy.right_w, legacy.road_z, legacy.road_tex);
        assert_eq!((-legacy.left_w, legacy.right_w, legacy.road_z), (-4.0, 4.0, -1.0));
        assert_eq!(legacy.road_tex.unwrap().0, PathBuf::from("profile-0.bmp"));
    }

    /// Three roads out of `centre`, turned by `turn`, made into a junction: its object.
    fn t_junction(doc: &mut Document, assets: &mut Assets, content: &Path, centre: DVec2, turn: f64, ids: [i64; 3], lengths: [f64; 3]) -> MapObject {
        let mut arms = Vec::new();
        for ((rel, id), length) in [0.0, 90.0, 270.0].into_iter().zip(ids).zip(lengths) {
            let out = roads::norm_deg(turn + rel);
            let p = centre + roads::dir(out) * 10.0;
            crate::selection::tests::add_spline(doc, (0, 0), id, p.x, p.y, out, length, 0, 0);
            arms.push(Arm { key: (0, 0), id, at_end: false });
        }
        let id = make_with(doc, assets, content, None, DVec3::new(centre.x, centre.y, 0.0), &arms, &Options::default(), MakeEdit::default()).unwrap();
        doc.tiles[&(0, 0)].tile.objects.iter().find(|o| o.id == id).unwrap().clone()
    }

    #[test]
    fn persisting_a_junction_keeps_its_sco_and_signal_links() {
        use crate::doc::{Sel, safe::own_path, tile_text};
        use crate::editor::{Editor, is_own_junction};

        let roots = crate::doc::tests::scratch("persist-junction");
        let mut assets = Assets::new(roots.omsi.clone(), roots.content.clone());
        assets.splines.insert("splines\\road.sli".into(), crate::assets::Slot::Ready(std::sync::Arc::new(test_road())));
        let mut doc = crate::selection::tests::flat_map(2);
        doc.dir = roots.content.join("maps").join("Test Junction");
        let k = (0, 0);
        let signal_tile = (1, 0);
        let centre = DVec3::new(294.0, 100.0, 1.0);
        let asphalt = roots.content.join("asphalt.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([70, 70, 70, 255])).save(&asphalt).unwrap();
        std::fs::write(asphalt.with_extension("surf"), b"surface").unwrap();
        let arms: Vec<_> = [0.0, 90.0, 270.0].into_iter().zip(1..).map(|(out, id)| {
            let p = centre.truncate() + roads::dir(out) * 10.0;
            crate::selection::tests::add_spline(&mut doc, k, id, p.x, p.y, out, 20.0, 0, 0);
            Arm { key: k, id, at_end: false }
        }).collect();
        let opts = Options { lights: true, light_objects: true, crossings: vec![0.0], ..Default::default() };
        let id = make_with(&mut doc, &mut assets, &roots.content, Some(&asphalt), centre, &arms, &opts, MakeEdit::default()).unwrap();
        let original = doc.object(k, id).unwrap().clone();
        let shared_id = doc.new_id();
        doc.tiles.get_mut(&k).unwrap().tile.objects.push(MapObject { id: shared_id, pos: [100.0, 200.0, 1.0], ..original.clone() });
        let before = [k, signal_tile].map(|key| tile_text(&doc.tiles[&key].tile));
        let next_id = doc.global.next_id_code;
        let signals = children(&doc, id);
        assert_eq!(signals.len(), 6);
        assert!(signals.iter().any(|(key, _)| *key == signal_tile));
        assert!(signals.iter().any(|&(key, id)| doc.object(key, id).unwrap().extra == ["1"]));

        let source = roots.resolve(&original.file);
        let sco_before = std::fs::read(&source).unwrap();
        let model_name = source.with_extension("x").file_name().unwrap().to_owned();
        let model = source.parent().unwrap().join("model").join(&model_name);
        let old_model = model.with_extension("x.missing");
        let textures = source.parent().unwrap().join("texture");
        let winter = textures.join("WinterSnow");
        std::fs::create_dir(&winter).unwrap();
        std::fs::copy(&asphalt, winter.join("asphalt.png")).unwrap();
        std::fs::write(textures.join("unrelated.png"), b"unrelated").unwrap();
        let old = omsi_cfg::decode_text(&sco_before);
        for tag in ["[path]", "[use_traffic_light]", "[traffic_lights_group]", "[splinehelper]"] {
            assert!(old.contains(tag), "Missing {tag}");
        }
        let mut ed = Editor::default();
        ed.open(doc);
        ed.select(Some(Sel::Object(k, id)));

        std::fs::rename(&model, &old_model).unwrap();
        let error = ed.persist_junction(&roots, k, id).unwrap_err();
        println!("Missing model result: {error:#}");
        assert_eq!(ed.doc.as_ref().unwrap().object(k, id).unwrap(), &original);
        assert!(!roots.omsi.join("Sceneryobjects").exists());
        std::fs::rename(&old_model, &model).unwrap();

        assert!(ed.remake_junction(id)); // A pending rebuild must not replace the saved plate.
        let rel = ed.persist_junction(&roots, k, id).unwrap();
        assert_eq!(rel, format!("Sceneryobjects\\openMapper-Persisted\\Test Junction\\Junction-{id}\\Junction-{id}.sco"));
        let target = own_path(&roots.omsi, &rel);
        let target_dir = target.parent().unwrap();
        let sco_after = std::fs::read(&target).unwrap();
        println!("Before SCO ({})\n{old}\nAfter SCO ({})\n{}", source.display(), target.display(), omsi_cfg::decode_text(&sco_after));
        assert_eq!(sco_before, sco_after);
        assert_eq!(std::fs::read(&model).unwrap(), std::fs::read(target_dir.join("model").join(model_name)).unwrap());
        for file in ["asphalt.png", "asphalt.surf", "om_zebra_white.png", "WinterSnow/asphalt.png"] {
            assert_eq!(std::fs::read(textures.join(file)).unwrap(), std::fs::read(target_dir.join("texture").join(file)).unwrap());
        }
        assert!(!target_dir.join("texture/unrelated.png").exists());

        ed.tick(&mut assets, 0.0, false);
        assert_eq!(ed.selected, Some(Sel::Object(k, id)));
        assert!(!is_own_junction(&rel) && !ed.remake_junction(id));
        let doc = ed.doc.as_mut().unwrap();
        let expected = MapObject { file: rel.clone(), ..original.clone() };
        assert_eq!(doc.object(k, id).unwrap(), &expected);
        assert_eq!(doc.object(k, shared_id).unwrap().file, original.file);
        assert_eq!(doc.global.next_id_code, next_id);
        assert_eq!(children(doc, id), signals);
        let after = [k, signal_tile].map(|key| tile_text(&doc.tiles[&key].tile));
        println!("Before tile 0\n{}\nAfter tile 0\n{}\nBefore tile 1\n{}\nAfter tile 1\n{}", before[0], after[0], before[1], after[1]);
        assert_eq!(after[0], before[0].replacen(&original.file, &rel, 1));
        assert_eq!(after[1], before[1]);
        ed.history.undo(doc);
        assert_eq!(doc.object(k, id).unwrap(), &original);
        ed.history.redo(doc);
        assert_eq!(doc.object(k, id).unwrap(), &expected);
        crate::doc::save(doc).unwrap();
        let reloaded = Document::load(&roots, &doc.dir.join("global.cfg"), |_| {}).unwrap();
        assert_eq!(reloaded.object(k, id).unwrap(), &expected);
        assert_eq!(children(&reloaded, id), signals);

        let stock = crate::assets::load_object(&target, &roots.omsi, &roots.content).unwrap();
        assert!(!stock.parts.is_empty() && !stock.helpers.is_empty());
        assert!(stock.parts.iter().flat_map(|p| &p.materials).all(|m| m.texture.as_ref().is_some_and(|p| p.starts_with(target_dir))));
        ed.open(reloaded);
        assert_eq!(ed.remake_all_junctions(), 1); // Only the other instance remains automatic.
        std::fs::remove_dir_all(&roots.content).unwrap();
    }

    #[test]
    fn junction_signal_placement_uses_the_arm_road_height_after_conforming_ground() {
        let roots = crate::doc::tests::scratch("junction-signal-road-height");
        let content = &roots.content;
        let mut doc = crate::selection::tests::flat_map(1);
        doc.dir = content.join("maps").join("signals");
        let sco_dir = content.join("Sceneryobjects").join("Fixture");
        std::fs::create_dir_all(&sco_dir).unwrap();
        std::fs::write(sco_dir.join("Head.sco"), "[trafficlight]\n").unwrap();
        std::fs::write(sco_dir.join("Pole.sco"), "[friendlyname]\nPole\n").unwrap();
        let config_dir = doc.dir.join("openMapper");
        std::fs::create_dir_all(&config_dir).unwrap();
        let config = r#"{"version":1,"default":"fixture","sets":[{"id":"fixture","name":"Fixture","head":"Sceneryobjects/Fixture/Head.sco","pole":"Sceneryobjects/Fixture/Pole.sco","head_height":2.55,"pole_offset":[0.0,-0.47,-2.55]}]}"#;
        std::fs::write(config_dir.join("signal_sets.json"), config).unwrap();
        doc.signal_sets.reload(&doc.dir, content);
        let mut assets = Assets::new(roots.omsi.clone(), content.clone());
        let mut road = test_road();
        for point in &mut road.def.profiles[0].points {
            point.z = 0.25;
        }
        for path in &mut road.def.paths {
            path.start[2] = 0.25;
        }
        assets.splines.insert("splines\\road.sli".into(), crate::assets::Slot::Ready(std::sync::Arc::new(road)));

        let centre = DVec3::new(100.0, 100.0, 10.0);
        let arms: Vec<_> = [(0.0, 11.0), (90.0, 12.0), (270.0, 13.0)].into_iter().enumerate().map(|(i, (heading, z))| {
            let p = centre.truncate() + roads::dir(heading) * 10.0;
            let id = i as i64 + 1;
            doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.push(omsi_map::MapSpline {
                id, file: "Splines\\road.sli".into(), pos: p.extend(z).to_array(), heading, length: 20.0,
                ..Default::default()
            });
            Arm { key: (0, 0), id, at_end: false }
        }).collect();
        let opts = Options::parse("openMapper junction: lights=1 main=30 side=20 crossings= helpers=omsi");
        let edit = MakeEdit { conform_min_reach: Some(20.0), ..Default::default() };
        let junction = make_with(&mut doc, &mut assets, content, None, centre, &arms, &opts, edit).unwrap();
        println!("Config: {config}\nOptions: {}", opts.line());
        let children = children(&doc, junction);
        assert_eq!(children.len(), 6);
        for (key, id) in children {
            let child = doc.object(key, id).unwrap();
            let world = doc.object_world(key, child, false);
            let heading = child.rot[0];
            let arm_z = match heading as i32 { 0 => 11.0, 90 => 12.0, 270 => 13.0, _ => panic!("{heading}") };
            let ground = doc.ground(world.truncate());
            let is_head = child.file.ends_with("Head.sco");
            let head_xy = match heading as i32 {
                0 => DVec2::new(94.8, 109.0),
                90 => DVec2::new(109.0, 105.2),
                270 => DVec2::new(91.0, 94.8),
                _ => panic!("{heading}"),
            };
            let road_z = arm_z + 0.25;
            let (before, expected) = if is_head {
                let head = head_xy.extend(road_z + 2.55);
                (head, head)
            } else {
                let forward = roads::dir(heading) * 0.47;
                ((head_xy + forward).extend(road_z), (head_xy - forward).extend(road_z))
            };
            println!("heading={heading} arm_z={arm_z} profile_z=0.25 ground={ground} before={before:?} after={world:?} expected={expected:?} tile={key:?} object={child:?}");
            assert!((world - expected).length() < 1e-9);
        }
        let _ = std::fs::remove_dir_all(content);
    }

    /// The same roads meeting at the same angles make one object, wherever the junction lies, whichever
    /// way it faces, whichever road came first and however long the roads run on.
    #[test]
    fn junctions_of_the_same_shape_share_one_object() {
        let roots = crate::doc::tests::scratch("junction-shared");
        let content = roots.content.clone();
        std::fs::create_dir_all(content.join("Sceneryobjects")).unwrap();
        let mut doc = crate::selection::tests::flat_map(1);
        doc.dir = content.join("maps").join("shared");
        let mut assets = Assets::new(roots.omsi.clone(), content.clone());
        assets.splines.insert("splines\\road.sli".into(), crate::assets::Slot::Ready(std::sync::Arc::new(test_road())));
        let a = t_junction(&mut doc, &mut assets, &content, DVec2::new(100.0, 100.0), 0.0, [1, 2, 3], [20.0, 20.0, 20.0]);
        let b = t_junction(&mut doc, &mut assets, &content, DVec2::new(200.0, 200.0), 37.0, [6, 5, 4], [13.5, 41.2, 27.3]);
        assert_eq!(a.file, b.file);
        assert!((roads::norm_deg(b.rot[0] - a.rot[0]) - 37.0).abs() < 1e-6, "{:?} {:?}", a.rot, b.rot);
        let dir = omsi_cfg::resolve_path(&content, &a.file);
        let dir = dir.parent().unwrap();
        let scos = std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "sco")).count();
        assert_eq!(scos, 1);
        // each junction's lanes meet its roads' lanes on the map, however it is turned
        assets.object_wait(&a.file, 5000).expect("the object reads back");
        for centre in [DVec2::new(100.0, 100.0), DVec2::new(200.0, 200.0)] {
            let (_, ends) = crate::lanes::near(&doc, &mut assets, centre, 14.0);
            let inner: Vec<_> = ends.iter().filter(|e| (e.pos.truncate() - centre).length() < 12.5).collect();
            assert!(inner.len() >= 12, "{}", inner.len());
            assert!(inner.iter().all(|e| e.linked), "{centre:?} {inner:?}");
        }
        let _ = std::fs::remove_dir_all(&content);
    }

    /// Synthetic .sli sections in their real file order: left side outer-to-inner,
    /// road surface, then right side inner-to-outer. Texture slot 3 is the kerb,
    /// slot 2 the footway, and slot 0 the road surface.
    fn profile_order_road(bus_bay: bool) -> SplineModel {
        let mut profiles = Vec::new();
        let mut add = |texture: usize, coords: &[(f32, f32)]| {
            profiles.push(omsi_scenery::SplineProfile {
                texture,
                points: coords.iter().map(|&(x, z)| omsi_scenery::SplineProfilePoint {
                    x, z, u: x * 0.1, v_scale: 0.2, ..Default::default()
                }).collect(),
            });
        };
        let (kerb, edge, outer, walk) = if bus_bay { (5.5, 5.65, 9.5, 7.5) } else { (3.5, 3.65, 7.5, 5.5) };
        add(2, &[(-7.5, 0.25), (-3.65, 0.25)]);
        add(3, &[(-3.65, 0.25), (-3.5, 0.25)]);
        add(3, &[(-3.5, 0.25), (-3.5, 0.1)]);
        add(0, &[(-3.5, 0.1), (0.0, 0.1)]);
        add(0, &[(0.0, 0.1), (3.5, 0.1)]);
        if bus_bay {
            add(0, &[(3.5, 0.1), (5.5, 0.1)]);
        }
        add(3, &[(kerb, 0.1), (kerb, 0.25)]);
        add(3, &[(kerb, 0.25), (edge, 0.25)]);
        add(2, &[(edge, 0.25), (outer, 0.25)]);
        let mut paths = Vec::new();
        let mut path = |kind: i32, x: f32, width: f32, direction: i32| {
            paths.push(omsi_scenery::PathDef {
                kind, start: [x, 0.0, 0.0], width, direction, ..Default::default()
            });
        };
        path(1, -5.5, 1.5, 2);
        path(0, -1.75, 2.5, if bus_bay { 1 } else { 0 });
        path(0, 1.75, 2.5, if bus_bay { 1 } else { 0 });
        if bus_bay {
            path(0, 4.5, 1.8, 1);
        }
        path(1, walk, 1.5, 2);
        let material = |name: &str| crate::assets::Material {
            texture: Some(PathBuf::from(name)),
            color: [1.0; 4],
            alpha: crate::assets::Alpha::Opaque,
            glow: None,
        };
        SplineModel {
            def: omsi_scenery::Spline { profiles, paths, ..Default::default() },
            materials: vec![
                material("road.bmp"), material("unused.bmp"),
                material("str_side2.bmp"), material("str_side1.bmp"),
            ],
            half_width: outer,
        }
    }

    fn profile_order_log(side: &[SideProfile]) -> Vec<(String, f64, f64)> {
        side.iter().map(|p| (
            p.texture.as_ref().and_then(|t| t.file_name()).unwrap().to_string_lossy().into_owned(),
            p.points.first().unwrap().0, p.points.last().unwrap().0,
        )).collect()
    }

    /// Width of one texture's triangles at a given arm's actual road interface:
    /// only vertices at that interface, on the specified outward side of the kerb.
    fn profile_order_width(mesh: &Mesh, arm: &ArmGeo, right: bool, texture: &str) -> Option<f64> {
        let axis = roads::dir(arm.out);
        let lateral = roads::right(arm.out);
        let sign = if right { 1.0 } else { -1.0 };
        let kerb = arm.end.truncate() + lateral * if right { arm.right_w } else { -arm.left_w };
        let mut values = Vec::new();
        for (tri, mat) in &mesh.tris {
            if mesh.textures[*mat].as_ref().and_then(|p| p.file_name()).and_then(|n| n.to_str()) != Some(texture) {
                continue;
            }
            for idx in tri {
                let v = mesh.verts[*idx].0;
                let delta = DVec2::new(v[0], v[1]) - kerb;
                let d = delta.dot(lateral) * sign;
                if delta.dot(axis).abs() < 1e-4 && (-1e-4..=5.0).contains(&d) {
                    values.push(d);
                }
            }
        }
        if values.is_empty() {
            None
        } else {
            Some(values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                - values.iter().copied().fold(f64::INFINITY, f64::min))
        }
    }

    #[test]
    fn two_arm_profile_strips_match_by_kerb_distance_in_both_directions() {
        let a = profile_order_road(false);
        let b = profile_order_road(true);
        let roots = crate::doc::tests::scratch("junction-profile-order");
        let mut assets = Assets::new(roots.omsi.clone(), roots.content.clone());
        assets.splines.insert("splines\\profile-a.sli".into(), crate::assets::Slot::Ready(std::sync::Arc::new(profile_order_road(false))));
        assets.splines.insert("splines\\profile-b.sli".into(), crate::assets::Slot::Ready(std::sync::Arc::new(profile_order_road(true))));
        let names = |s: &[SideProfile]| -> Vec<String> {
            s.iter().map(|p| p.texture.as_ref().unwrap().file_name().unwrap().to_string_lossy().into_owned()).collect()
        };
        let expected = vec!["str_side1.bmp", "str_side1.bmp", "str_side2.bmp"];
        for b_at_end in [false, true] {
            let case = if b_at_end { "A end -> B end (B reversed)" } else { "A end -> B start (same spline direction)" };
            let g_a = arm_geo_of(&a, false, true, DVec3::new(0.0, -10.0, 0.0), 180.0, 0.0);
            let g_b = arm_geo_of(&b, false, b_at_end, DVec3::new(0.0, 10.0, 0.0), 0.0, 0.0);
            let corrected = [g_a, g_b];
            for side in [&corrected[0].left, &corrected[0].right] {
                assert_eq!(names(side), expected);
            }
            let mut original = corrected.clone();
            // The left-hand .sli profiles were written outer-to-inner; their
            // pre-fix side() order is exactly the reverse of the sorted strips.
            for (g, at_end) in original.iter_mut().zip([true, b_at_end]) {
                if at_end { g.right.reverse(); } else { g.left.reverse(); }
            }
            for (i, (old, new)) in original.iter().zip(corrected.iter()).enumerate() {
                for (label, before, after) in [("left", &old.left, &new.left), ("right", &old.right, &new.right)] {
                    println!("{case}, arm {i} {label} BEFORE (texture,inner,outer) = {:?}", profile_order_log(before));
                    println!("{case}, arm {i} {label} AFTER  (texture,inner,outer) = {:?}", profile_order_log(after));
                }
            }
            let arms = [
                PlateArm { file: "splines\\profile-a.sli".into(), pos: DVec2::new(0.0, -10.0), out: 180.0, at_end: true },
                PlateArm { file: "splines\\profile-b.sli".into(), pos: DVec2::new(0.0, 10.0), out: 0.0, at_end: b_at_end },
            ];
            let plate = layout_plate(&mut assets, &arms, None).expect("both arms produce a plate");
            assert!(!plate.mesh.tris.is_empty(), "{case}: plate has triangles");
            let previous = junction_mesh(&original, DVec3::ZERO, None, 8.0, 0.0).0;
            let mut old_wedge = false;
            for (i, arm) in corrected.iter().enumerate() {
                for (label, right) in [("left", false), ("right", true)] {
                    let old_kerb = profile_order_width(&previous, arm, right, "str_side1.bmp");
                    let old_walk = profile_order_width(&previous, arm, right, "str_side2.bmp");
                    let kerb = profile_order_width(&plate.mesh, arm, right, "str_side1.bmp").expect("kerb triangles reach the interface");
                    let walk = profile_order_width(&plate.mesh, arm, right, "str_side2.bmp").expect("footway triangles reach the interface");
                    println!("{case}, arm {i} {label} interface: BEFORE side1={old_kerb:?} side2={old_walk:?}; AFTER side1={kerb:.3}m side2={walk:.3}m");
                    old_wedge |= old_kerb.is_some_and(|v| v > 0.2)
                        || old_walk.is_some_and(|v| (v - 3.85).abs() > 0.05);
                    assert!(kerb <= 0.2, "{case} arm {i} {label}: kerb width {kerb}");
                    assert!((walk - 3.85).abs() <= 0.05, "{case} arm {i} {label}: footway width {walk}");
                }
            }
            if b_at_end {
                assert!(old_wedge, "reversed B should reproduce the original material wedge");
            }
        }
    }

    #[test]
    fn a_junction_mesh_reads_back_through_openomsi() {
        let arm = |end: DVec3, out: f64| ArmGeo {
            end,
            out,
            right_w: 3.5,
            left_w: 3.5,
            right: vec![SideProfile { texture: None, points: vec![(0.0, 0.0, 0.0, 0.2), (0.0, 0.15, 0.1, 0.2), (2.0, 0.15, 0.5, 0.2)] }],
            left: vec![SideProfile { texture: None, points: vec![(0.0, 0.0, 0.0, 0.2), (0.0, 0.15, 0.1, 0.2), (2.0, 0.15, 0.5, 0.2)] }],
            v0: 0.0,
            vdir: 1.0,
            road_tex: None,
            road_z: 0.0,
        };
        let arms = vec![arm(DVec3::new(10.0, 0.0, 0.0), 90.0), arm(DVec3::new(-10.0, 0.0, 0.0), 270.0), arm(DVec3::new(0.0, 10.0, 0.0), 0.0)];
        let mesh = junction_mesh(&arms, DVec3::ZERO, None, 8.0, 0.0).0;
        assert!(mesh.tris.len() > 20);
        // every roadway triangle faces up
        for (f, mat) in &mesh.tris {
            if *mat == 0 {
                let p: Vec<DVec3> = f.iter().map(|i| DVec3::from(mesh.verts[*i].0)).collect();
                assert!((p[1] - p[0]).cross(p[2] - p[0]).z >= 0.0);
            }
        }
        let x = mesh_x(&mesh, &[String::new()]);
        let dir = std::env::temp_dir().join(format!("om-junction-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("j.x"), x).unwrap();
        let m = omsi_o3d::load_mesh(&dir.join("j.x")).unwrap();
        assert_eq!(m.triangles.len(), mesh.tris.len() * 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// Read vertex heights for one texture from the *written* DirectX .x,
    /// respecting MeshMaterialList rather than assuming a vertex belongs to asphalt.
    fn x_texture_heights(x: &str, texture: &str) -> Vec<f64> {
        let mut lines = x.lines().skip_while(|l| l.trim() != "Mesh {");
        assert_eq!(lines.next().map(str::trim), Some("Mesh {"));
        let nverts: usize = lines.next().unwrap().trim().trim_end_matches(';').parse().unwrap();
        let heights: Vec<f64> = (0..nverts).map(|_| {
            lines.next().unwrap().trim().split(';').nth(1).unwrap().parse().unwrap()
        }).collect();
        let nfaces: usize = lines.next().unwrap().trim().trim_end_matches(';').parse().unwrap();
        let faces: Vec<Vec<usize>> = (0..nfaces).map(|_| {
            let line = lines.next().unwrap().trim();
            line.strip_prefix("3;").unwrap().split(';').next().unwrap()
                .split(',').map(|v| v.parse::<usize>().unwrap()).collect()
        }).collect();

        let mut mats = x.lines().skip_while(|l| l.trim() != "MeshMaterialList {");
        assert_eq!(mats.next().map(str::trim), Some("MeshMaterialList {"));
        let _nmaterials: usize = mats.next().unwrap().trim().trim_end_matches(';').parse().unwrap();
        let material_faces: usize = mats.next().unwrap().trim().trim_end_matches(';').parse().unwrap();
        assert_eq!(material_faces, nfaces);
        let face_mats: Vec<usize> = (0..nfaces).map(|_| {
            mats.next().unwrap().trim().trim_end_matches(|c: char| c == ',' || c == ';').parse().unwrap()
        }).collect();
        let mat = x.split("  Material {").skip(1)
            .position(|m| m.split("\n  }").next().unwrap().contains(&format!("\"{texture}\"")))
            .expect("texture has a Material in the written .x");
        faces.iter().zip(face_mats).filter(|(_, m)| *m == mat)
            .flat_map(|(face, _)| face.iter().map(|i| heights[*i])).collect()
    }

    #[derive(Clone)]
    struct WrittenPath {
        a: DVec3,
        b: DVec3,
        kind: i32,
        width: f64,
    }

    fn written_paths(sco: &str) -> Vec<WrittenPath> {
        let normalized = sco.replace("\r\n", "\n");
        normalized.split("[path]\n").skip(1).map(|section| {
            let vals: Vec<&str> = section.lines().take(12).collect();
            assert_eq!(vals.len(), 12);
            let n = |i: usize| vals[i].parse::<f64>().unwrap();
            let seg = Seg { start: DVec2::new(n(0), n(1)), heading: n(3), radius: n(4), length: n(5) };
            let end = seg.end();
            WrittenPath {
                a: DVec3::new(n(0), n(1), n(2)),
                b: DVec3::new(end.x, end.y, n(2) + n(6) * seg.length),
                kind: vals[8].parse().unwrap(),
                width: n(9),
            }
        }).collect()
    }

    /// Identify a crossing by its real geometry, not its width: the path begins
    /// on the zebra's mid-plane and runs across (perpendicular to) that arm.
    fn roadway_crossing_paths(paths: &[WrittenPath], arms: &[ArmGeo]) -> Vec<(usize, usize)> {
        paths.iter().enumerate().filter_map(|(pi, p)| {
            if p.kind != 1 { return None }
            let delta = p.b.truncate() - p.a.truncate();
            if delta.length() < 0.01 { return None }
            arms.iter().enumerate().find(|(_, g)| {
                let mid = g.end.truncate() - roads::dir(g.out) * 2.0;
                let d = roads::dir(g.out);
                let cross = (g.left_w + g.right_w + 1.0).max(1.0);
                (p.a.truncate() - mid).dot(d).abs() < 0.05
                    && (p.b.truncate() - mid).dot(d).abs() < 0.05
                    && (delta.normalize().dot(d)).abs() < 0.01
                    && (delta.length() - cross).abs() < 0.05
            }).map(|(arm, _)| (pi, arm))
        }).collect()
    }

    fn record_paths(recs: &[PathRec]) -> String {
        let mut body = String::new();
        for p in recs {
            let s = &p.seg;
            let grad = if s.length > 0.01 { (p.z1 - p.z0) / s.length } else { 0.0 };
            let _ = write!(body, "[path]\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n",
                num(s.start.x), num(s.start.y), num(p.z0), num(s.heading), num(s.radius),
                num(s.length), num(grad), num(grad), p.kind, num(p.width), p.direction, p.blinker);
        }
        body
    }

    fn zebra_road_with_raised_sidewalks() -> SplineModel {
        let mut road = test_road();
        let point = |x: f32, z: f32| omsi_scenery::SplineProfilePoint {
            x, z, ..Default::default()
        };
        road.def.profiles = vec![
            omsi_scenery::SplineProfile {
                texture: 0, points: vec![point(-4.0, 0.20), point(4.0, 0.20)],
            },
            omsi_scenery::SplineProfile {
                texture: 0, points: vec![point(-6.5, 0.35), point(-5.5, 0.35), point(-4.0, 0.20)],
            },
            omsi_scenery::SplineProfile {
                texture: 0, points: vec![point(4.0, 0.20), point(5.5, 0.35), point(6.5, 0.35)],
            },
        ];
        for lane in &mut road.def.paths {
            lane.start[2] = 0.20;
        }
        for x in [-5.5_f32, 5.5] {
            road.def.paths.push(omsi_scenery::PathDef {
                kind: 1, start: [x, 0.0, 0.35], width: 2.0, direction: 2,
                ..Default::default()
            });
        }
        road.half_width = 6.5;
        road
    }

    /// Four equally high arms, all with raised footways and marked crossings.
    /// Write both versions of the .x/.sco, print *all* [path] blocks and stripe
    /// vertex heights, and assert the new geometry is connected and above asphalt.
    #[test]
    fn junction_zebra_crossings_join_sidewalks_and_clear_asphalt() {
        let roots = crate::doc::tests::scratch("junction-zebra-regression");
        let content = roots.content.clone();
        std::fs::create_dir_all(content.join("Sceneryobjects")).unwrap();
        let mut doc = crate::selection::tests::flat_map(1);
        doc.dir = content.join("maps").join("zebra-four");
        let mut assets = Assets::new(roots.omsi.clone(), content.clone());
        assets.splines.insert("splines\\road.sli".into(),
            crate::assets::Slot::Ready(std::sync::Arc::new(zebra_road_with_raised_sidewalks())));
        let centre = DVec3::new(100.0, 100.0, 0.0);
        let mut arms = Vec::new();
        for (i, out) in [0.0_f64, 90.0, 180.0, 270.0].into_iter().enumerate() {
            let pos = centre.truncate() + roads::dir(out) * 10.0;
            let id = i as i64 + 100;
            crate::selection::tests::add_spline(&mut doc, (0, 0), id, pos.x, pos.y, out, 20.0, 0, 0);
            arms.push(Arm { key: (0, 0), id, at_end: false });
        }
        let opts = Options {
            lights: false, main_green: 30.0, side_green: 20.0,
            crossings: vec![0.0, 90.0, 180.0, 270.0],
            ..Default::default()
        };
        let asphalt = content.join("om_test_asphalt.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([70, 70, 70, 255]))
            .save(&asphalt).unwrap();

        // Reconstruct the original make_with() output, preserving its original
        // lane routing and old z formula. This is the before-fix reference.
        let (_, lanes) = arm_ends(&doc, &mut assets, &arms, None).unwrap();
        let geo: Vec<ArmGeo> = arms.iter().map(|a| arm_geometry(&doc, &mut assets, a).unwrap()).collect();
        let files: Vec<String> = arms.iter().map(|a| doc.spline(a.key, a.id).unwrap().file.clone()).collect();
        let plate = plate_frame(&geo, &files, &lanes, centre, true);
        let mut before_recs = paths(&plate.lanes, DVec3::ZERO, &[None; 4], &[], true,
            &[], &[], &[], false);
        let mut zebras = Vec::new();
        for g in &plate.arms {
            let mid = g.end.truncate() - roads::dir(g.out) * 2.0;
            let r = roads::right(g.out);
            let (a, b) = (mid - r * (g.left_w + 0.5), mid + r * (g.right_w + 0.5));
            let seg = roads::straight(a, b).unwrap();
            before_recs.push(PathRec {
                seg, z0: g.end.z, z1: g.end.z, kind: 1,
                width: 3.0, direction: 2, blinker: 0, light: None, pocket: false,
            });
            zebras.push((mid, g.out, g.left_w, g.right_w));
        }
        assert_eq!(zebras.len(), 4);
        let before_sco = format!("[friendlyname]\nBefore fix\n\n[mesh]\nbefore.x\n\n{}", record_paths(&before_recs));
        let mut before_mesh = junction_mesh(&plate.arms, DVec3::ZERO, Some(asphalt.clone()), 8.0, 0.0).0;
        let old_z = plate.arms.iter().map(|g| g.end.z).sum::<f64>() / 4.0 + 0.03;
        paint_zebra_stripes(&mut before_mesh, &zebras, old_z, content.join("om_zebra_white.png"));
        let before_names: Vec<String> = before_mesh.textures.iter()
            .map(|t| t.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        let before_x = mesh_x(&before_mesh, &before_names);

        // Exercise production make_with(), and read the actual written files.
        let id = make_with(&mut doc, &mut assets, &content, Some(&asphalt), centre, &arms, &opts, MakeEdit::default()).unwrap();
        let obj = doc.tiles[&(0, 0)].tile.objects.iter().find(|o| o.id == id).unwrap();
        let sco_path = omsi_cfg::resolve_path(&content, &obj.file);
        let after_sco = omsi_cfg::decode_text(&std::fs::read(&sco_path).unwrap());
        let after_x = std::fs::read_to_string(sco_path.parent().unwrap().join("model").join(format!(
            "{}.x", sco_path.file_stem().unwrap().to_string_lossy()
        ))).unwrap();

        let before_z = x_texture_heights(&before_x, "om_zebra_white.png");
        let before_road = x_texture_heights(&before_x, "om_test_asphalt.png");
        let after_z = x_texture_heights(&after_x, "om_zebra_white.png");
        let after_road = x_texture_heights(&after_x, "om_test_asphalt.png");
        assert!(!before_z.is_empty() && !before_road.is_empty());
        assert_eq!(before_z.len(), after_z.len());
        println!("===== BEFORE .sco ALL [path] BLOCKS =====\n{}", &before_sco[before_sco.find("[path]").unwrap()..]);
        println!("===== BEFORE .x ZEBRA VERTEX HEIGHTS ({}) =====\n{:?}", before_z.len(), before_z);
        println!("===== AFTER .sco ALL [path] BLOCKS =====\n{}", { let normalized = after_sco.replace("\r\n", "\n"); normalized[normalized.find("[path]").unwrap()..].to_string() });
        println!("===== AFTER .x ZEBRA VERTEX HEIGHTS ({}) =====\n{:?}", after_z.len(), after_z);
        println!("===== ASPHALT VERTEX HEIGHTS BEFORE / AFTER =====\n{:?}\n{:?}", before_road, after_road);

        // Old stripes are buried inside elevated asphalt; new stripes must
        // clear every vertex belonging to the asphalt material.
        assert!(before_z.iter().any(|z| before_road.iter().any(|r| z <= r)));
        assert!(after_z.iter().all(|z| after_road.iter().all(|r| z > r)),
            "zebra {:?} must clear asphalt {:?}", after_z, after_road);
        let before_paths = written_paths(&before_sco);
        let after_paths = written_paths(&after_sco);
        let marked = roadway_crossing_paths(&after_paths, &plate.arms);
        assert_eq!(marked.len(), 4, "one geometrically identified roadway crossing per arm");
        let connected = |paths: &[WrittenPath], i: usize, at: DVec3| {
            paths.iter().enumerate().any(|(j, q)| j != i && q.kind == 1 &&
                [q.a, q.b].iter().any(|p| (p.truncate() - at.truncate()).length() < 0.05 && (p.z - at.z).abs() < 0.05))
        };
        // Assert both ends of every roadway / connector path, not only its midpoint.
        for (i, _) in &marked {
            let path = &after_paths[*i];
            for endpoint in [path.a, path.b] {
                assert!(connected(&after_paths, *i, endpoint),
                    "crossing path #{i} endpoint {endpoint:?} has no coincident footway endpoint");
            }
        }
        // Each raised sidewalk ramp must connect to a two-metre stub, regardless
        // of its lane width. Corner walks remain optional.
        let ramps: Vec<_> = after_paths.iter().enumerate()
            .filter(|(_, p)| p.kind == 1 && (p.a.z - p.b.z).abs() > 0.05).collect();
        assert_eq!(ramps.len(), 8, "two ramps on each of four crossings");
        for (i, p) in ramps {
            let top = if p.a.z > p.b.z { p.a } else { p.b };
            assert!(connected(&after_paths, i, top), "ramp top {top:?} must meet a sidewalk stub");
        }
        let old_cross = roadway_crossing_paths(&before_paths, &plate.arms);
        println!("===== BEFORE / AFTER COMPARISON =====");
        println!("all [path] blocks: {} -> {}", before_paths.len(), after_paths.len());
        println!("roadway crossings: {} -> {}; kind=1 paths (including stubs): {} -> {}",
            old_cross.len(), marked.len(),
            before_paths.iter().filter(|p| p.kind == 1).count(),
            after_paths.iter().filter(|p| p.kind == 1).count());
        println!("asphalt z: {:.4} m; zebra z: {:.4} -> {:.4} m",
            after_road[0], before_z[0], after_z[0]);
        assert_eq!(old_cross.len(), 4);
        assert!(old_cross.iter().any(|(i, _)| !connected(&before_paths, *i, before_paths[*i].a)));

        for g in &plate.arms {
            let mid = g.end.truncate() - roads::dir(g.out) * 2.0;
            for lane in plate.lanes.iter().filter(|l| l.kind == 1) {
                let is_arm = (lane.pos.truncate() - g.end.truncate()).length() < 10.0
                    && (lane.pos.truncate() - g.end.truncate()).dot(roads::dir(g.out)).abs() < 0.05;
                if is_arm {
                    let crossing = lane.pos.truncate() + mid - g.end.truncate();
                    println!("at_arm={:?}; at_crossing={:?}",
                        lane.pos, DVec3::new(crossing.x, crossing.y, lane.pos.z));
                }
            }
        }

        // Keep both generated artifacts for byte-for-byte comparison in CI.
        let artifacts = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/junction-zebra-regression");
        std::fs::create_dir_all(&artifacts).unwrap();
        for (name, bytes) in [
            ("before.sco", before_sco.as_bytes()), ("before.x", before_x.as_bytes()),
            ("after.sco", after_sco.as_bytes()), ("after.x", after_x.as_bytes()),
        ] {
            std::fs::write(artifacts.join(name), bytes).unwrap();
        }
        let _ = std::fs::remove_dir_all(&content);
    }


    #[test]
    fn junction_uses_map_asphalt_and_zebra_textures_independently() {
        // Run every case in its own map/output directory, so a stale file cannot
        // satisfy a copy assertion. The spline's own BMP is available throughout.
        let cases = [
            ("map", "Texture", Some(MAP_ASPHALT), true, Some(MAP_ZEBRA), true),
            ("mixed-case", "tEXture", Some("PLAIN_Asphalt_Texture.BMP"), true, Some("Zebra_TEXTURE.BMP"), true),
            ("installed", "Texture", None, true, Some(MAP_ZEBRA), true),
            ("untextured", "Texture", None, false, Some(MAP_ZEBRA), true),
            ("white-stripes", "Texture", Some(MAP_ASPHALT), false, None, true),
            ("stripes-disabled", "Texture", Some(MAP_ASPHALT), true, Some(MAP_ZEBRA), false),
        ];
        let save_bmp = |path: &Path, rgb| {
            image::RgbImage::from_pixel(2, 2, image::Rgb(rgb))
                .save_with_format(path, image::ImageFormat::Bmp).unwrap();
        };
        for (name, folder, asphalt_name, have_stock, zebra_name, stripes) in cases {
            let roots = crate::doc::tests::scratch(&format!("junction-texture-{name}"));
            let mut doc = crate::selection::tests::flat_map(1);
            doc.dir = roots.content.join("maps").join(name);
            let map_tex_dir = doc.dir.join(folder);
            std::fs::create_dir_all(&map_tex_dir).unwrap();
            let map_asphalt = asphalt_name.map(|file| {
                let p = map_tex_dir.join(file);
                save_bmp(&p, [61, 62, 63]);
                p
            });
            let map_zebra = zebra_name.map(|file| {
                let p = map_tex_dir.join(file);
                save_bmp(&p, [205, 206, 207]);
                p
            });
            if have_stock {
                let stock_dir = roots.omsi.join("Texture");
                std::fs::create_dir_all(&stock_dir).unwrap();
                save_bmp(&stock_dir.join("str_asphdrk.bmp"), [76, 77, 78]);
            }
            let installed = asphalt_texture(&roots.omsi);

            let spline_tex = roots.content.join("spline-road.bmp");
            save_bmp(&spline_tex, [88, 89, 90]);
            let mut assets = Assets::new(roots.omsi.clone(), roots.content.clone());
            let centre = DVec3::new(100.0, 100.0, 0.0);
            let arms: Vec<_> = [0.0, 90.0, 180.0, 270.0].into_iter().enumerate().map(|(i, out)| {
                let id = 100 + i as i64;
                let p = centre.truncate() + roads::dir(out) * 10.0;
                let width = if i == 0 { 4.0 } else { 3.5 };
                let mut road = arm_profile_road(&[[(-width, 0.1), (width, 0.1)]], &[-1.5, 1.5]);
                road.materials[0].texture = Some(spline_tex.clone());
                let points = &mut road.def.profiles[0].points;
                points[0].v_scale = 0.25;
                points[1].v_scale = 0.25;
                points[1].u = 1.0;
                let file = format!("Splines\\road-{i}.sli");
                assets.splines.insert(file.to_lowercase(), crate::assets::Slot::Ready(std::sync::Arc::new(road)));
                crate::selection::tests::add_spline(&mut doc, (0, 0), id, p.x, p.y, out, 20.0, 0, 0);
                doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.last_mut().unwrap().file = file;
                Arm { key: (0, 0), id, at_end: false }
            }).collect();
            let opts = Options { crossings: vec![0.0], zebra_texture: stripes, ..Default::default() };
            let id = make_with(&mut doc, &mut assets, &roots.content, installed.as_deref(), centre, &arms, &opts, MakeEdit::default()).unwrap();
            let obj = doc.tiles[&(0, 0)].tile.objects.iter().find(|o| o.id == id).unwrap();
            let sco = omsi_cfg::resolve_path(&roots.content, &obj.file);
            let model = std::fs::read_to_string(sco.parent().unwrap().join("model").join(sco.with_extension("x").file_name().unwrap())).unwrap();
            let tex_dir = sco.parent().unwrap().join("texture");

            let mut textures: Vec<_> = std::fs::read_dir(&tex_dir).unwrap().map(|e| e.unwrap().file_name()).collect();
            textures.sort();
            let materials: Vec<_> = model.split("  Material {").skip(1).map(|m| {
                m.split("\n  }").next().unwrap().lines().skip_while(|l| l.trim() != "TextureFilename {")
                    .nth(1).map(|l| l.trim().trim_end_matches(';').trim_matches('"'))
            }).collect();
            let asphalt = map_asphalt.as_ref().or(installed.as_ref());
            let asphalt_ref = asphalt.map(|p| if map_asphalt.is_some() { MAP_ASPHALT } else { p.file_name().unwrap().to_str().unwrap() });
            let zebra_ref = stripes.then_some(if map_zebra.is_some() { MAP_ZEBRA } else { "om_zebra_white.png" });
            println!(
                "{name}: spline asphalt before={spline_tex:?}, period=4.0 m; map asphalt={map_asphalt:?}; installed={installed:?}; map stripes={map_zebra:?}; zebra_tex={stripes}; selected asphalt={asphalt:?}; selected zebra={:?}; .x TextureFilename={materials:?}; texture directory {tex_dir:?}={textures:?}",
                zebra_ref.map(|file| map_zebra.clone().unwrap_or_else(|| tex_dir.join(file))),
            );
            assert_eq!(junction_asphalt(Some(&doc.dir), installed.as_deref()).as_ref(), asphalt);
            assert_eq!(materials[0], asphalt_ref);
            assert_eq!(materials.iter().flatten().copied().collect::<Vec<_>>(),
                asphalt_ref.into_iter().chain(zebra_ref).collect::<Vec<_>>());
            let x_files: std::collections::BTreeSet<_> = materials.into_iter().flatten().map(std::ffi::OsStr::new).collect();
            assert_eq!(textures.iter().map(|f| f.as_os_str()).collect::<std::collections::BTreeSet<_>>(), x_files);
            if let (Some(source), Some(file)) = (asphalt, asphalt_ref) {
                assert_eq!(std::fs::read(tex_dir.join(file)).unwrap(), std::fs::read(source).unwrap());
            }
            if let Some(file) = zebra_ref {
                if let Some(source) = map_zebra {
                    assert_eq!(std::fs::read(tex_dir.join(file)).unwrap(), std::fs::read(source).unwrap());
                } else {
                    let pixels = image::open(tex_dir.join(file)).unwrap().into_rgba8();
                    assert_eq!(pixels.dimensions(), (8, 8));
                    assert!(pixels.pixels().all(|p| p.0 == [235, 235, 230, 255]));
                }
            }

            // The widest arm points north. Its right pavement edge (4, 10, 0.1)
            // is written in .x order (x, height, north), with U=4/8 and V=-10/8.
            let vertex = model.lines().skip_while(|l| l.trim() != "Mesh {").skip(2)
                .position(|l| l.trim().starts_with("4.0000;0.1000;10.0000;")).unwrap();
            let uv = model.lines().skip_while(|l| l.trim() != "MeshTextureCoords {")
                .nth(vertex + 2).unwrap().trim();
            println!("{name}: .x vertex (4, 0.1, 10) UV={uv}; expected=(0.5, -1.25), period=8.0 m");
            assert_eq!(uv, "0.50000;-1.25000;,");
            std::fs::remove_dir_all(&roots.content).unwrap();
        }
    }

    /// Signal phases and pedestrian paths must work without any generated signal
    /// objects or white stripe polygons. The two switches are independent.
    #[test]
    fn junction_light_objects_and_zebra_texture_are_independent() {
        let roots = crate::doc::tests::scratch("junction-optional-visuals");
        let content = roots.content.clone();
        std::fs::create_dir_all(content.join("Sceneryobjects")).unwrap();
        let mut doc = crate::selection::tests::flat_map(1);
        doc.dir = content.join("maps").join("optional-visuals");
        let mut assets = Assets::new(roots.omsi.clone(), content.clone());
        assets.splines.insert("splines\\road.sli".into(),
            crate::assets::Slot::Ready(std::sync::Arc::new(zebra_road_with_raised_sidewalks())));
        let centre = DVec3::new(100.0, 100.0, 0.0);
        let mut arms = Vec::new();
        for (i, out) in [0.0_f64, 90.0, 180.0, 270.0].into_iter().enumerate() {
            let p = centre.truncate() + roads::dir(out) * 10.0;
            let id = 300 + i as i64;
            crate::selection::tests::add_spline(&mut doc, (0, 0), id,
                p.x, p.y, out, 20.0, 0, 0);
            arms.push(Arm { key: (0, 0), id, at_end: false });
        }
        let mut reference_paths: Option<String> = None;
        for (light_objects, zebra_texture) in [(false, true), (true, true), (false, false), (true, false)] {
            let opts = Options {
                lights: true, light_objects, zebra_texture,
                crossings: vec![0.0, 180.0], ..Default::default()
            };
            let id = make_with(&mut doc, &mut assets, &content, None, centre, &arms, &opts, MakeEdit::default()).unwrap();
            let junction = doc.tiles[&(0, 0)].tile.objects.iter().find(|o| o.id == id).unwrap();
            let sco_path = omsi_cfg::resolve_path(&content, &junction.file);
            let sco = omsi_cfg::decode_text(&std::fs::read(&sco_path).unwrap());
            assert_eq!(Options::parse(&sco), opts);
            assert!(sco.contains("[traffic_lights_group]"));
            assert!(sco.contains("[use_traffic_light]"), "AI must still stop at red");
            let paths = written_paths(&sco);
            assert!(paths.iter().any(|p| p.kind == 1), "footway crossings remain present");
            let suffix = sco.replace("\r\n", "\n").split_once("[path]\n").unwrap().1.to_owned();
            if let Some(reference) = &reference_paths {
                assert_eq!(&suffix, reference, "visual switches must not change AI paths");
            } else {
                reference_paths = Some(suffix);
            }
            let kids = children(&doc, id);
            assert_eq!(kids.len(), if light_objects { 8 } else { 0 });
            if light_objects {
                let mut heads = 0;
                let mut poles = 0;
                for (key, child_id) in kids {
                    let o = doc.object(key, child_id).unwrap();
                    if o.file.eq_ignore_ascii_case(r"Sceneryobjects\Verkehrszeichen_MC\Ampel_Kfz_1.sco") {
                        heads += 1;
                        assert_eq!(o.flag, 1);
                    } else if o.file.eq_ignore_ascii_case(r"Sceneryobjects\Verkehrszeichen_MC\Mast_Ampel_klein_1.sco") {
                        poles += 1;
                        assert_eq!(o.flag, 0);
                    } else {
                        panic!("unexpected junction child: {}", o.file);
                    }
                    assert_eq!(o.var_parent, Some(id));
                }
                assert_eq!((heads, poles), (4, 4));
            }
            let model_path = sco_path.parent().unwrap().join("model")
                .join(format!("{}.x", sco_path.file_stem().unwrap().to_string_lossy()));
            let model = std::fs::read_to_string(model_path).unwrap();
            assert_eq!(model.contains("om_zebra_white.png"), zebra_texture,
                "stripe material must follow the texture switch");
        }
        let _ = std::fs::remove_dir_all(&content);
    }

    /// The old road_styles.json schema must continue generating corner paths.
    #[test]
    fn old_road_style_json_keeps_corner_walks_enabled() {
        let legacy = r#"{"splines\\a.sli":{"corner":2.0,"lights":true,"crossings":false,"junctions":true}}"#;
        let styles: std::collections::HashMap<String, crate::road_maker::JunctionStyle> =
            serde_json::from_str(legacy).unwrap();
        assert!(styles["splines\\a.sli"].corner_walks);
        println!("LEGACY road_styles.json: {legacy} => corner_walks={}",
            styles["splines\\a.sli"].corner_walks);
    }

    /// End-to-end test of both switch positions. Re-read actual .sco path records.
    /// For every sidewalk lane at the arm, verify the 2 m stub, followed by a
    /// ramp, and then a full-width roadway crossing. Corners are never needed
    /// for that chain.
    #[test]
    fn four_arm_crossings_connect_road_footways_with_corners_on_and_off() {
        for corner_walks in [true, false] {
            let roots = crate::doc::tests::scratch(if corner_walks { "junction-stub-on" } else { "junction-stub-off" });
            let content = roots.content.clone();
            std::fs::create_dir_all(content.join("Sceneryobjects")).unwrap();
            let mut doc = crate::selection::tests::flat_map(1);
            doc.dir = content.join("maps").join(if corner_walks { "corners-on" } else { "corners-off" });
            let mut assets = Assets::new(roots.omsi.clone(), content.clone());
            assets.splines.insert("splines\\road.sli".into(),
                crate::assets::Slot::Ready(std::sync::Arc::new(zebra_road_with_raised_sidewalks())));
            let centre = DVec3::new(100.0, 100.0, 0.0);
            let mut arms = Vec::new();
            for (i, out) in [0.0_f64, 90.0, 180.0, 270.0].into_iter().enumerate() {
                let pos = centre.truncate() + roads::dir(out) * 10.0;
                let id = 200 + i as i64;
                crate::selection::tests::add_spline(&mut doc, (0, 0), id,
                    pos.x, pos.y, out, 20.0, 0, 0);
                arms.push(Arm { key: (0, 0), id, at_end: false });
            }
            let opts = Options {
                crossings: vec![0.0, 90.0, 180.0, 270.0],
                corner_walks, ..Default::default()
            };
            let (_, lanes) = arm_ends(&doc, &mut assets, &arms, None).unwrap();
            let geo: Vec<ArmGeo> = arms.iter().map(|a| arm_geometry(&doc, &mut assets, a).unwrap()).collect();
            let files: Vec<String> = arms.iter().map(|a| doc.spline(a.key, a.id).unwrap().file.clone()).collect();
            let plate = plate_frame(&geo, &files, &lanes, centre, true);
            let id = make_with(&mut doc, &mut assets, &content, None, centre, &arms, &opts, MakeEdit::default()).unwrap();
            let obj = doc.tiles[&(0, 0)].tile.objects.iter().find(|o| o.id == id).unwrap();
            let sco_path = omsi_cfg::resolve_path(&content, &obj.file);
            let sco = omsi_cfg::decode_text(&std::fs::read(sco_path).unwrap());
            let paths = written_paths(&sco);
            assert_eq!(Options::parse(&sco).corner_walks, corner_walks);
            println!("===== corner_walks={corner_walks} .sco FULL [path] BLOCKS =====");
            println!("{}", &sco[sco.find("[path]").unwrap()..]);

            let marked = roadway_crossing_paths(&paths, &plate.arms);
            assert_eq!(marked.len(), 4);
            let mut joins = 0;
            let mut printed_arms = std::collections::HashSet::<(i32, i32, i32)>::new();
            for (i, g) in plate.arms.iter().enumerate() {
                let r = roads::right(g.out);
                let mid = g.end.truncate() - roads::dir(g.out) * 2.0;
                for left in [true, false] {
                    let l = plate.lanes.iter().filter(|l| l.arm == i && l.kind == 1)
                        .filter_map(|l| {
                            let side = (l.pos.truncate() - g.end.truncate()).dot(r);
                            if (left && side < -0.01) || (!left && side > 0.01) {
                                Some((side.abs(), l))
                            } else {
                                None
                            }
                        })
                        .max_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, l)| l).unwrap();
                    let at_arm = l.pos;
                    let xy = at_arm.truncate() + mid - g.end.truncate();
                    let at_crossing = DVec3::new(xy.x, xy.y, at_arm.z);
                    println!("corner_walks={corner_walks} arm={i} left={left} at_arm={at_arm:?} at_crossing={at_crossing:?}");
                    assert!(printed_arms.insert(((at_arm.x * 100.0).round() as i32,
                        (at_arm.y * 100.0).round() as i32, (at_arm.z * 100.0).round() as i32)),
                        "each sidewalk lane end should be distinct");
                    // At the road end, a path endpoint must coincide horizontally
                    // within 5 cm and vertically within 5 cm.
                    let close = |a: DVec3, b: DVec3| {
                        (a.truncate() - b.truncate()).length() < 0.05 && (a.z - b.z).abs() < 0.05
                    };
                    let stub = paths.iter().find(|p| p.kind == 1
                        && close(p.a, at_arm) && close(p.b, at_crossing)
                        && (p.width - l.width).abs() < 0.001).unwrap_or_else(|| {
                            panic!("missing 2 m stub: {at_arm:?} -> {at_crossing:?}, width {}", l.width)
                        });
                    assert!(((stub.b - stub.a).length() - 2.0).abs() < 0.05);
                    assert!(paths.iter().any(|p| p.kind == 1
                        && [p.a, p.b].iter().any(|x| close(*x, at_crossing))
                        && (p.a.z - p.b.z).abs() > 0.05
                        && (p.width - l.width).abs() < 0.001),
                        "stub must connect directly to a graded crossing ramp");
                    joins += 1;
                }
            }
            assert_eq!(joins, 8);
            for (index, _) in &marked {
                let crossing = &paths[*index];
                for end in [crossing.a, crossing.b] {
                    assert!(paths.iter().enumerate().any(|(k, p)| k != *index
                        && p.kind == 1 && [p.a, p.b].iter().any(|e| {
                            (end.truncate() - e.truncate()).length() < 0.05
                                && (end.z - e.z).abs() < 0.05
                        })), "roadway crossing disconnected at {end:?}");
                }
            }
            // Map a path endpoint to the nearest arm's crossing plane.
            // Only the corner paths join endpoints assigned to different arms.
            let arm_of = |pt: DVec3| {
                (0..4).min_by(|&a, &b| {
                    let ma = plate.arms[a].end.truncate() - roads::dir(plate.arms[a].out) * 2.0;
                    let mb = plate.arms[b].end.truncate() - roads::dir(plate.arms[b].out) * 2.0;
                    (ma - pt.truncate()).length_squared().total_cmp(&(mb - pt.truncate()).length_squared())
                }).unwrap()
            };
            let corner_count = paths.iter().filter(|p| p.kind == 1
                && arm_of(p.a) != arm_of(p.b)).count();
            assert_eq!(corner_count, if corner_walks { 4 } else { 0 },
                "corner AI path count must follow corner_walks switch");
            let total_walks = paths.iter().filter(|p| p.kind == 1).count();
            assert_eq!(total_walks, if corner_walks { 24 } else { 20 },
                "4 crossings + 8 ramps + 8 stubs + optional 4 corners");
            println!("mode={corner_walks}: {total_walks} kind=1 paths, {corner_count} corner paths, 8 stubs, 4 crossings");
            let _ = std::fs::remove_dir_all(&content);
        }
    }


    // The northwest corner of the reported repro: north arm kerb x=-4.5,
    // west arm kerb y=+13, north rightmost inbound lane x=-2,
    // west rightmost outbound lane y=+11.625.
    fn kerb_lead_fixture(symmetric: bool) -> (Vec<ArmGeo>, Vec<LaneEnd>) {
        let mut geo = Vec::new();
        let mut lanes = Vec::new();
        for (i, out) in [0.0, 90.0, 180.0, 270.0].into_iter().enumerate() {
            let wide = symmetric || i % 2 == 1;
            let kerb = if wide { 13.0 } else { 4.5 };
            let distance = if wide { 30.0 } else { 31.0 };
            let d = roads::dir(out) * distance;
            let end = DVec3::new(d.x, d.y, 0.0);
            geo.push(ArmGeo {
                end, out, left_w: kerb, right_w: kerb,
                right: Vec::new(), left: Vec::new(), v0: 0.0, vdir: 1.0,
                road_tex: None, road_z: 0.0,
            });
            let centres: &[f64] = if wide {
                &[-11.625, -8.125, -5.25, 5.25, 8.125, 11.625]
            } else {
                &[-2.0, 2.0]
            };
            for &offset in centres {
                let xy = end.truncate() + roads::right(out) * offset;
                let pos = DVec3::new(xy.x, xy.y, end.z);
                for (incoming, flow) in [(true, roads::norm_deg(out + 180.0)), (false, out)] {
                    lanes.push(LaneEnd {
                        arm: i, pos, flow, kind: 0, width: 2.5,
                        // Right of the *incoming* heading = minus right(out).
                        side: -offset, incoming, pocket: false,
                    });
                }
            }
        }
        (geo, lanes)
    }

    fn trace_turn<'a>(paths: &'a [PathRec], pa: DVec2, pb: DVec2, blinker: i32) -> Vec<&'a PathRec> {
        let mut current = pa;
        let mut result = Vec::new();
        let mut used = std::collections::HashSet::new();
        for _ in 0..10 {
            let (idx, rec) = paths.iter().enumerate().find(|(idx, rec)| {
                rec.kind == 0 && rec.blinker == blinker && !used.contains(idx)
                    && (rec.seg.start - current).length() < 0.02
            }).unwrap_or_else(|| panic!("missing route segment at {current:?} toward {pb:?}, blink {blinker}"));
            used.insert(idx);
            current = rec.seg.end();
            result.push(rec);
            if (current - pb).length() < 0.03 { return result; }
        }
        panic!("turn did not arrive at {pb:?}, last {current:?}");
    }

    fn turn_blocks(route: &[&PathRec]) -> String {
        let mut result = String::new();
        for p in route {
            let s = &p.seg;
            let gradient = (p.z1 - p.z0) / s.length.max(0.01);
            let _ = write!(result, "[path]\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n",
                num(s.start.x), num(s.start.y), num(p.z0),
                num(s.heading), num(s.radius), num(s.length),
                num(gradient), num(gradient), p.kind, num(p.width), p.direction, p.blinker);
        }
        result
    }

    fn chosen_turn(lanes: &[LaneEnd], i: usize, j: usize, right: bool) -> (LaneEnd, LaneEnd) {
        let mut incoming: Vec<_> = lanes.iter().filter(|l| l.arm == i && l.incoming).collect();
        let mut outgoing: Vec<_> = lanes.iter().filter(|l| l.arm == j && !l.incoming).collect();
        incoming.sort_by(|a, b| b.side.total_cmp(&a.side));
        outgoing.sort_by(|a, b| a.side.total_cmp(&b.side));
        let pick = if right { 0 } else { incoming.len() - 1 };
        let exit = if right { 0 } else { outgoing.len() - 1 };
        (*incoming[pick], *outgoing[exit])
    }

    #[test]
    fn unequal_kerbs_keep_outer_right_turns_clear_of_corner() {
        let (geo, lanes) = kerb_lead_fixture(false);
        let (_, corners) = junction_mesh(&geo, DVec3::ZERO, None, 8.0, 0.0);
        let leads = corner_leads(&corners);
        let northwest = corners.iter().find(|c| c.i == 0 && c.j == 3).unwrap();
        println!("NW kerb_round: {:?}", northwest.segments);
        assert!((northwest.lead_i - 0.0).abs() < 0.05);
        assert!((northwest.lead_j - 7.5).abs() < 0.05);

        let before = paths(&lanes, DVec3::ZERO, &[None; 4], &[], true,
            &[], &[false; 4], &[], false);
        let after = paths(&lanes, DVec3::ZERO, &[None; 4], &[], true,
            &leads, &[false; 4], &corners, false);
        for i in 0..4 {
            let j = (i + 3) % 4;
            let (a, b) = chosen_turn(&lanes, i, j, true);
            let (pa, pb) = (a.pos.truncate(), b.pos.truncate());
            let c = corners.iter().find(|c| c.i == i && c.j == j).unwrap();
            let old = trace_turn(&before, pa, pb, 3);
            let new = trace_turn(&after, pa, pb, 3);
            let old_segs: Vec<_> = old.iter().map(|r| r.seg).collect();
            let new_segs: Vec<_> = new.iter().map(|r| r.seg).collect();
            let (old_d, old_at) = path_kerb_clearance(&old_segs, c);
            let (new_d, new_at) = path_kerb_clearance(&new_segs, c);
            println!("===== RIGHT TURN {i} -> {j} BEFORE: min={old_d:.4} at {old_at:?} =====\n{}",
                turn_blocks(&old));
            println!("===== RIGHT TURN {i} -> {j} AFTER: min={new_d:.4} at {new_at:?} =====\n{}",
                turn_blocks(&new));
            assert!(new_d >= 1.25 - 1e-4,
                "outer right turn {i}->{j}: {new_d:.4} m to kerb at {new_at:?}");
            if i == 0 {
                assert!(old_d < 1.25, "NW baseline should reproduce kerb clipping");
                assert!(new_d > old_d, "NW tangent lead must improve clearance");
            }
        }

        // For left-hand traffic the outermost left turn traverses the
        // same rounded corner in the opposite direction.
        let left = paths(&lanes, DVec3::ZERO, &[None; 4], &[], true,
            &leads, &[false; 4], &corners, true);
        let (a, b) = chosen_turn(&lanes, 3, 0, false);
        let reversed = trace_turn(&left, a.pos.truncate(), b.pos.truncate(), 2);
        let reversed_segs: Vec<_> = reversed.iter().map(|r| r.seg).collect();
        let (clearance, _) = path_kerb_clearance(&reversed_segs, northwest);
        println!("===== LEFT-HAND OUTER LEFT TURN 3 -> 0: min={clearance:.4} =====\n{}",
            turn_blocks(&reversed));
        assert!(clearance >= 1.25 - 1e-4);
    }

    #[test]
    fn symmetric_kerbs_are_bit_identical_without_zebras() {
        let (geo, lanes) = kerb_lead_fixture(true);
        let (_, corners) = junction_mesh(&geo, DVec3::ZERO, None, 8.0, 0.0);
        let leads = corner_leads(&corners);
        assert!(leads.iter().all(|c| c.2.abs() < 1e-6 && c.3.abs() < 1e-6),
            "equal arms need no kerb tangent lead: {leads:?}");
        let old = paths(&lanes, DVec3::ZERO, &[None; 4], &[], true,
            &[], &[false; 4], &[], false);
        let new = paths(&lanes, DVec3::ZERO, &[None; 4], &[], true,
            &leads, &[false; 4], &corners, false);
        let before = record_paths(&old);
        let after = record_paths(&new);
        assert_eq!(after.as_bytes(), before.as_bytes(),
            "symmetric four-arm .sco path records must be byte-for-byte identical");
        println!("===== SYMMETRIC FOUR ARMS BEFORE [path] =====\n{before}");
        println!("===== SYMMETRIC FOUR ARMS AFTER [path] =====\n{after}");

        // Even on asymmetric roads, without zebra crossings a normal
        // left turn (not an outer LHT turn) is completely unchanged.
        let (geo, lanes) = kerb_lead_fixture(false);
        let (_, corners) = junction_mesh(&geo, DVec3::ZERO, None, 8.0, 0.0);
        let leads = corner_leads(&corners);
        let before = paths(&lanes, DVec3::ZERO, &[None; 4], &[], true,
            &[], &[false; 4], &[], false);
        let after = paths(&lanes, DVec3::ZERO, &[None; 4], &[], true,
            &leads, &[false; 4], &corners, false);
        let only_left = |recs: &[PathRec]| record_paths(&recs.iter().filter(|p| p.kind == 0 && p.blinker == 2)
            .map(|p| PathRec {
                seg: p.seg, z0: p.z0, z1: p.z1, kind: p.kind,
                width: p.width, direction: p.direction, blinker: p.blinker, light: p.light, pocket: p.pocket,
            }).collect::<Vec<_>>());
        assert_eq!(only_left(&before).as_bytes(), only_left(&after).as_bytes());
        println!("===== ASYMMETRIC / NO ZEBRA LEFT TURNS BEFORE [path] =====\n{}", only_left(&before));
        println!("===== ASYMMETRIC / NO ZEBRA LEFT TURNS AFTER [path] =====\n{}", only_left(&after));
    }

    #[test]
    fn all_zebra_arms_keep_four_metres_straight_at_both_ends() {
        let (geo, lanes) = kerb_lead_fixture(false);
        let (_, corners) = junction_mesh(&geo, DVec3::ZERO, None, 8.0, 0.0);
        let leads = corner_leads(&corners);
        let old = paths(&lanes, DVec3::ZERO, &[None; 4], &[], true,
            &[], &[false; 4], &[], false);
        let new = paths(&lanes, DVec3::ZERO, &[None; 4], &[], true,
            &leads, &[true; 4], &corners, false);
        for i in 0..4 {
            for j in 0..4 {
                if i == j || (i + 2) % 4 == j { continue; }
                let right = (i + 3) % 4 == j;
                let (a, b) = chosen_turn(&lanes, i, j, right);
                let (pa, pb) = (a.pos.truncate(), b.pos.truncate());
                let old_route = trace_turn(&old, pa, pb, if right { 3 } else { 2 });
                let route = trace_turn(&new, pa, pb, if right { 3 } else { 2 });
                println!("===== ZEBRA TURN {i} -> {j}, before =====\n{}", turn_blocks(&old_route));
                println!("===== ZEBRA TURN {i} -> {j}, after =====\n{}", turn_blocks(&route));
                let segs: Vec<_> = route.iter().map(|p| p.seg).collect();
                let full: f64 = segs.iter().map(|s| s.length).sum();
                assert!(full > 8.0);
                // Check each 0.25 m sample, including the boundary at 4 m,
                // against the arm's incoming or outgoing flow (< 1 degree).
                for n in 0..=16 {
                    for (d, target) in [(n as f64 * 0.25, a.flow), (full - n as f64 * 0.25, b.flow)] {
                        let mut acc = 0.0;
                        let mut heading = None;
                        for (k, seg) in segs.iter().enumerate() {
                            if d <= acc + seg.length + 1e-7 || k + 1 == segs.len() {
                                heading = Some(seg.heading_at((d - acc).clamp(0.0, seg.length)));
                                break;
                            }
                            acc += seg.length;
                        }
                        let diff = signed_turn(heading.unwrap(), target).abs();
                        assert!(diff < 1.0,
                            "turn {i}->{j} at {d:.2}/{full:.2} m deviates {diff:.3} degrees; path:\n{}",
                            turn_blocks(&route));
                    }
                }
            }
        }
    }

}
