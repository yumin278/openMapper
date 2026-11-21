use super::*;
use crate::road_maker::{Direction, Strip};

/// The path labels: their font size, the room they keep from the drawing and from the sides,
/// and the gap between two labels.
const LABEL_FONT: f32 = 9.0;
const LABEL_PAD: f32 = 3.0;
const LABEL_GAP: f32 = 4.0;

struct PreviewAxis {
    left: f32,
    min_x: f32,
    scale: f32,
}

impl PreviewAxis {
    fn new(rect: egui::Rect, road_left: f32, road_right: f32) -> Self {
        let pad = 15.0_f32.min(rect.width() * 0.2);
        let min_x = road_left.min(0.0);
        let max_x = road_right.max(0.0);
        let span = (max_x - min_x).max(0.001);
        Self {
            left: rect.left() + pad,
            min_x,
            scale: (rect.width() - 2.0 * pad).max(0.0) / span,
        }
    }

    fn pixel(&self, x: f32) -> f32 {
        self.left + (x - self.min_x) * self.scale
    }
}

fn draw_origin(painter: &egui::Painter, x: f32, top: f32, bottom: f32) {
    let mut y = top;
    while y < bottom {
        let dash = [Pos2::new(x, y), Pos2::new(x, (y + 5.0).min(bottom))];
        // A light border keeps the black dashes readable over dark asphalt.
        painter.line_segment(dash, Stroke::new(3.0, Color32::from_gray(200)));
        painter.line_segment(dash, Stroke::new(1.5, Color32::BLACK));
        y += 9.0;
    }
}

pub(super) fn draw(ui: &mut egui::Ui, s: &RoadSpec, tex: &mut TexCache, top: bool) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), if top { 170.0 } else { 100.0 }),
        Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_rgb(28, 30, 33));
    let positions = s.strip_positions();
    let lo = positions.first().map_or(0.0, |p| p.0);
    let hi = positions.last().map_or(1.0, |p| p.1);
    let axis = PreviewAxis::new(rect, s.output_x(lo), s.output_x(hi));
    let scale = axis.scale;
    let x = |m| axis.pixel(s.output_x(m));
    // Each path's x, under the strip it is written for: the value in the `.sli`, signed.
    let font = egui::FontId::monospace(LABEL_FONT);
    let (centers, labels) = path_labels(s, &positions, |m| axis.pixel(m));
    let widths: Vec<f32> = labels
        .iter()
        .map(|text| {
            painter
                .layout_no_wrap(text.clone(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .collect();
    let row_h = painter.fonts_mut(|f| f.row_height(&font)).max(LABEL_FONT) + 1.0;
    let (left, right) = (rect.left() + LABEL_PAD, rect.right() - LABEL_PAD);
    let (_, rows) = label_boxes(rect.bottom() + LABEL_GAP, &centers, &widths, left, right, row_h);
    let band = (rows > 0).then(|| {
        ui.allocate_exact_size(
            Vec2::new(rect.width(), rows as f32 * row_h + 2.0 * LABEL_PAD),
            Sense::hover(),
        )
        .0
    });
    let panel = band.map_or(rect, |band| rect.union(band));
    let panel_painter = ui.painter_at(panel);
    panel_painter.rect_filled(panel, 3.0, Color32::from_rgb(28, 30, 33));
    let bottom = rect.bottom() - 24.0;
    for (i, (strip, &(a, b))) in s.strips.iter().zip(&positions).enumerate() {
        let bus = matches!(
            strip,
            Strip::Lane { bus_only: true, .. } | Strip::BusBay { .. }
        );
        let (color, path) = match strip {
            Strip::Lane { .. } | Strip::BusBay { .. } => (
                if bus {
                    Color32::from_rgb(40, 160, 180)
                } else {
                    Color32::from_rgb(100, 105, 115)
                },
                &s.roadway_tex,
            ),
            Strip::Footway { tex, .. } => (
                Color32::from_gray(175),
                if tex.as_os_str().is_empty() {
                    &s.footway_tex
                } else {
                    tex
                },
            ),
            Strip::Median { tex, .. } => (Color32::from_rgb(100, 145, 90), tex),
        };
        let pic = tex.get(ui.ctx(), path);
        let y = if top {
            rect.top() + 8.0
        } else {
            bottom - 9.0 - ((strip.height(s.kerb) - crate::road_maker::ROAD_H) * scale * 4.0).min(18.0)
        };
        let (u0, u1) = if matches!(strip, Strip::Lane { .. }) {
            (s.roadway_u(a), s.roadway_u(b))
        } else {
            (0.0, 1.0)
        };
        quad(
            &painter,
            pic,
            [
                Pos2::new(x(a), y),
                Pos2::new(x(b), y),
                Pos2::new(x(b), bottom),
                Pos2::new(x(a), bottom),
            ],
            [
                Pos2::new(u0, 0.0),
                Pos2::new(u1, 0.0),
                Pos2::new(u1, 1.0),
                Pos2::new(u0, 1.0),
            ],
            color,
        );
        if bus {
            painter.rect_filled(
                egui::Rect::from_min_max(Pos2::new(x(a), y), Pos2::new(x(b), bottom)),
                0.0,
                Color32::from_rgba_unmultiplied(0, 200, 230, 75),
            );
        }
        let direction = match strip {
            Strip::Lane { dir, .. } => Some(*dir),
            Strip::BusBay { .. } => s.strips[..i]
                .iter()
                .rev()
                .chain(s.strips[i + 1..].iter())
                .find_map(|p| {
                    if let Strip::Lane { dir, .. } = p {
                        Some(*dir)
                    } else {
                        None
                    }
                }),
            _ => None,
        };
        if let Some(dir) = direction {
            let middle = axis.pixel(s.strip_path_x(strip, a, b));
            let cy = if top { (y + bottom) / 2.0 } else { y - 11.0 };
            let sign = if dir == Direction::Fwd { -1.0 } else { 1.0 };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(middle, cy + sign * 8.0),
                    Pos2::new(middle - 5.0, cy - sign * 5.0),
                    Pos2::new(middle + 5.0, cy - sign * 5.0),
                ],
                Color32::WHITE,
                Stroke::NONE,
            ));
            if bus {
                painter.text(
                    Pos2::new(middle, cy + 22.0),
                    egui::Align2::CENTER_CENTER,
                    tr("BUS", "АВТОБУС"),
                    egui::FontId::proportional(11.0),
                    Color32::WHITE,
                );
            }
        }
        if i + 1 < s.strips.len() && strip.height(s.kerb) != s.strips[i + 1].height(s.kerb) {
            let end = if top {
                y
            } else {
                bottom - 9.0 - ((s.strips[i + 1].height(s.kerb) - crate::road_maker::ROAD_H) * scale * 4.0).min(18.0)
            };
            painter.line_segment(
                [Pos2::new(x(b), end), Pos2::new(x(b), bottom)],
                Stroke::new(2.0, Color32::LIGHT_GRAY),
            );
        }
    }
    if top {
        let markings = if s.marked {
            s.markings.clone()
        } else {
            s.standard_markings()
        };
        for m in markings.into_iter().filter(|_| scale > 0.0) {
            for (position, dashed) in m.lines_at() {
                let mut y = rect.top() + 8.0;
                while y < bottom {
                    let end = if dashed {
                        (y + m.dash * scale).min(bottom)
                    } else {
                        bottom
                    };
                    painter.line_segment(
                        [Pos2::new(x(position), y), Pos2::new(x(position), end)],
                        Stroke::new(
                            (m.width * scale).max(1.0),
                            Color32::from_rgb(m.color[0], m.color[1], m.color[2]),
                        ),
                    );
                    if !dashed {
                        break;
                    }
                    y += (s.dash_period(&m) * scale).max(1.0);
                }
            }
        }
    }
    draw_origin(&painter, axis.pixel(0.0), rect.top() + 4.0, bottom + 4.0);
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 9.0),
        egui::Align2::CENTER_CENTER,
        format!(
            "{:.1} m · {} {:.1} m",
            s.total_width(),
            tr("roadway", "проезжая часть"),
            s.roadway_width()
        ),
        egui::FontId::proportional(11.0),
        Color32::LIGHT_GRAY,
    );
    if let Some(band) = band {
        let (boxes, _) = label_boxes(band.top() + LABEL_PAD, &centers, &widths, left, right, row_h);
        for (text, b) in labels.iter().zip(boxes) {
            panel_painter.text(
                Pos2::new(b.left(), b.top()),
                egui::Align2::LEFT_TOP,
                text,
                font.clone(),
                Color32::from_gray(200),
            );
        }
    }
}

/// The on-screen position and emitted X label for each Lane or BusBay arrow.
fn path_labels(
    s: &RoadSpec,
    positions: &[(f32, f32)],
    x: impl Fn(f32) -> f32,
) -> (Vec<f32>, Vec<String>) {
    let mut centers = Vec::new();
    let mut labels = Vec::new();
    for (strip, &(a, b)) in s.strips.iter().zip(positions) {
        if matches!(strip, Strip::Lane { .. } | Strip::BusBay { .. }) {
            let path_x = s.strip_path_x(strip, a, b);
            centers.push(x(path_x));
            labels.push(signed_x(path_x));
        }
    }
    (centers, labels)
}

/// A path's x as the `.sli` writes it, with its sign and the `X=` the preview puts before it:
/// `X=+1.639`, `X=-0.500`. The writer prints three decimals, so a negative zero it would write
/// as `-0.000` keeps that sign here too.
fn signed_x(x: f32) -> String {
    let number = format!("{x:.3}");
    if number.starts_with('-') {
        format!("X={number}")
    } else {
        format!("X=+{number}")
    }
}

/// A box for every label, under `top`, packed into as many rows as it takes so that none touches
/// another and none leaves `left`..`right`. Returns the boxes and how many rows they use.
fn label_boxes(
    top: f32,
    centers: &[f32],
    widths: &[f32],
    left: f32,
    right: f32,
    row_h: f32,
) -> (Vec<egui::Rect>, usize) {
    let mut ends: Vec<f32> = Vec::new();
    let mut boxes = Vec::with_capacity(centers.len());
    for (&center, &width) in centers.iter().zip(widths) {
        let width = width.min(right - left).max(0.0);
        let start = (center - width / 2.0).clamp(left, (right - width).max(left));
        let mut row = 0;
        while row < ends.len() && ends[row] + LABEL_GAP > start {
            row += 1;
        }
        if row == ends.len() {
            ends.push(start + width);
        } else {
            ends[row] = start + width;
        }
        boxes.push(egui::Rect::from_min_size(
            Pos2::new(start, top + row as f32 * row_h),
            Vec2::new(width, row_h - 1.0),
        ));
    }
    (boxes, ends.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn road_maker_strip_preview_path_x_is_shown_signed_to_the_millimetre() {
        for x in [-1.639, 0.0, 1.639] {
            println!("Local X {x} → {}", signed_x(x));
        }
        assert_eq!(signed_x(1.639), "X=+1.639");
        assert_eq!(signed_x(-1.639), "X=-1.639");
        assert_eq!(signed_x(0.0), "X=+0.000");
        assert_eq!(signed_x(-2.0), "X=-2.000");
        assert_eq!(signed_x(12.5), "X=+12.500");
        // The writer's three decimals keep the sign of a negative zero: so does the preview.
        assert_eq!(format!("{:.3}", -0.0f32), "-0.000");
        assert_eq!(signed_x(-0.0), "X=-0.000");
        assert_eq!(signed_x(-0.0004), "X=-0.000");
    }

    /// A footway, a bus bay, two lanes either side of a median and a footway: only the bay and
    /// the lanes get an arrow, so only they get a label.
    fn road_maker_strip_preview_sample() -> RoadSpec {
        let foot = |path| Strip::Footway {
            width: 2.0,
            tex: PathBuf::new(),
            path,
            height: None,
        };
        RoadSpec {
            name: "Sample".into(),
            left_hand: Some(false),
            strips: vec![
                foot(true),
                Strip::BusBay { width: 3.0 },
                Strip::Lane {
                    dir: Direction::Back,
                    width: 3.0,
                    bus_only: false,
                    path_offset: 0.0,
                },
                Strip::Median {
                    width: 2.0,
                    raised: true,
                    tex: PathBuf::new(),
                    height: None,
                },
                Strip::Lane {
                    dir: Direction::Fwd,
                    width: 4.0,
                    bus_only: false,
                    path_offset: 0.0,
                },
                foot(true),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn road_maker_strip_preview_labels_only_lanes_and_bus_bays() {
        let spec = road_maker_strip_preview_sample();
        let positions = spec.strip_positions();
        let (centers, labels) = path_labels(&spec, &positions, &|m| m);
        // The bay at -6 m and the lanes at -3 m and +2.5 m: no footway and no median.
        assert_eq!(labels, vec!["X=-6.000", "X=-3.000", "X=+2.500"]);
        // The label sits on the strip's own centre, as the writer centres the path.
        assert_eq!(centers, vec![-6.0, -3.0, 2.5]);
        let shown: Vec<String> = labels
            .iter()
            .zip(&centers)
            .map(|(label, center)| format!("{label} at {center} m"))
            .collect();
        println!("road maker strip preview example labels: {shown:?}");
        // A footway without an AI path would write no path at all, and still gets no label.
        let mut without = road_maker_strip_preview_sample();
        if let Some(Strip::Footway { path, .. }) = without.strips.get_mut(0) {
            *path = false;
        }
        let (_, labels) = path_labels(&without, &without.strip_positions(), &|m| m);
        assert_eq!(labels, vec!["X=-6.000", "X=-3.000", "X=+2.500"]);
        without.x_offset = 3.0;
        let (shifted_centers, shifted_labels) =
            path_labels(&without, &without.strip_positions(), &|m| m);
        println!("X offset +3.000, shifted drawing centers {shifted_centers:?}, labels {shifted_labels:?}");
        assert_eq!(shifted_centers, vec![-3.0, 0.0, 5.5]);
        assert_eq!(shifted_labels, vec!["X=-3.000", "X=+0.000", "X=+5.500"]);
    }

    /// Read the meshes, arrowheads and origin dashes actually submitted to egui.
    fn drawn_x(spec: &RoadSpec, width: f32, top: bool) -> (f32, f32, f32, Vec<f32>) {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 400.0));
        let mut textures = TexCache::default();
        let frame = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |root| {
                egui::CentralPanel::default().show(root, |ui| draw(ui, spec, &mut textures, top));
            },
        );
        let mut edges = Vec::new();
        let mut arrows = Vec::new();
        let mut dashes = Vec::new();
        for item in &frame.shapes {
            match &item.shape {
                egui::Shape::Mesh(mesh) if mesh.vertices.len() == 4 => {
                    edges.extend(mesh.vertices.iter().map(|v| v.pos.x));
                    assert!(mesh.vertices.iter().all(|v| item.clip_rect.contains(v.pos)));
                }
                egui::Shape::Path(path) if path.closed && path.points.len() == 3 => {
                    arrows.push(path.points[0].x);
                }
                egui::Shape::LineSegment { points, stroke }
                    if stroke.color == Color32::BLACK && stroke.width == 1.5 =>
                {
                    assert_eq!(points[0].x, points[1].x);
                    assert!(points.iter().all(|p| item.clip_rect.contains(*p)));
                    dashes.push((points[0].x, points[0].y, points[1].y));
                }
                _ => {}
            }
        }
        assert!(dashes.len() > 2);
        let origin = dashes[0].0;
        assert!(dashes.iter().all(|dash| dash.0 == origin));
        assert!(dashes.windows(2).all(|pair| pair[0].2 < pair[1].1));
        let left = edges.iter().copied().reduce(f32::min).unwrap();
        let right = edges.iter().copied().reduce(f32::max).unwrap();
        (origin, left, right, arrows)
    }

    fn verify_drawn_and_written(spec: &RoadSpec, range: (f32, f32), paths: &[f32]) {
        let sli = spec.sli(false);
        let emitted = omsi_scenery::Spline::parse(&omsi_cfg::CfgFile::from_str("preview.sli", &sli));
        let vertices = emitted.profiles.iter().flat_map(|p| &p.points);
        let lo = vertices.clone().map(|p| p.x).reduce(f32::min).unwrap();
        let hi = vertices.map(|p| p.x).reduce(f32::max).unwrap();
        assert_eq!((lo, hi), range);
        let written: Vec<_> = emitted.paths.iter().map(|p| p.start[0]).collect();
        assert_eq!(written, paths);
        for width in [72.0, 218.0, 540.0] {
            let top = drawn_x(spec, width, true);
            let side = drawn_x(spec, width, false);
            assert_eq!(top, side);
            let (origin, left, right, arrows) = top;
            let scale = (right - left) / (hi - lo);
            assert!(scale > 0.0);
            assert!((left - (origin + scale * lo)).abs() < 0.02);
            assert!((right - (origin + scale * hi)).abs() < 0.02);
            assert_eq!(arrows.len(), paths.len());
            for (&pixel, &path) in arrows.iter().zip(paths) {
                assert!((pixel - (origin + scale * path)).abs() < 0.02);
            }
            println!("canvas {width:.0}: SLI [{lo:.3}, {hi:.3}] paths {paths:?}; drawn origin={origin:.2}, edges={left:.2}..{right:.2}, arrows={arrows:?} (top and side)");
        }
    }

    #[test]
    fn road_maker_preview_draws_the_emitted_x_in_both_views() {
        let mut spec = RoadSpec {
            advanced: true,
            strips: vec![Strip::Lane {
                dir: Direction::Fwd,
                width: 3.5,
                bus_only: false,
                path_offset: 0.25,
            }],
            ..Default::default()
        };
        for offset in [0.0, 1.75, -1.75, 5.0, -5.0] {
            spec.x_offset = offset;
            verify_drawn_and_written(&spec, (offset - 1.75, offset + 1.75), &[offset + 0.25]);
        }
    }

    #[test]
    fn road_maker_preview_places_imported_wilhelm2_at_the_origin_edge() {
        let spec = crate::road_maker::sli_import::wilhelm2_test_spec();
        verify_drawn_and_written(&spec, (0.0, 8.9), &[1.45, 4.4, 7.4]);
    }

    #[test]
    fn road_maker_strip_preview_labels_stay_under_the_drawing_and_clear_of_each_other() {
        let body = egui::Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(400.0, 170.0));
        let top = body.bottom() + LABEL_GAP;
        let mut checked = 0;
        for (centers, width) in [
            (vec![60.0, 140.0, 220.0, 300.0, 380.0], 34.0),
            // Narrow lanes: the labels need more than one row.
            (vec![30.0, 42.0, 54.0, 66.0, 78.0, 90.0, 102.0, 114.0], 34.0),
            // At the edges: no label leaves the picture.
            (vec![-40.0, 100.0, 200.0, 480.0], 40.0),
        ] {
            let widths = vec![width; centers.len()];
            let (boxes, rows) =
                label_boxes(top, &centers, &widths, body.left(), body.right(), 12.0);
            assert_eq!(boxes.len(), centers.len());
            assert!(rows >= 1);
            for (i, b) in boxes.iter().enumerate() {
                assert!(b.top() >= body.bottom(), "{b:?} is not under the drawing");
                assert!(b.left() >= body.left() - 0.01 && b.right() <= body.right() + 0.01, "{b:?}");
                for other in &boxes[i + 1..] {
                    assert!(
                        !b.intersects(*other),
                        "{b:?} overlaps {other:?} in {centers:?}"
                    );
                }
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn road_maker_strip_preview_labels_keep_one_per_path_in_a_narrow_drawing() {
        let body = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(40.0, 100.0));
        let centers = vec![10.0, 20.0, 30.0];
        let widths = vec![80.0, 80.0, 80.0];
        let (boxes, rows) =
            label_boxes(body.bottom() + LABEL_GAP, &centers, &widths, body.left(), body.right(), 12.0);
        assert_eq!(boxes.len(), 3);
        assert_eq!(rows, 3);
        assert!(boxes.iter().all(|b| b.width() <= body.width()));
        assert!(!boxes[0].intersects(boxes[1]) && !boxes[1].intersects(boxes[2]));
    }
}
