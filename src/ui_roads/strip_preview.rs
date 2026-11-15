use super::*;
use crate::road_maker::{Direction, Strip};

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
    let scale = (rect.width() - 30.0) / (hi - lo).max(1.0);
    let x = |m: f32| rect.center().x + (m - (lo + hi) / 2.0) * scale;
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
            bottom - 9.0 - ((strip.height(s.kerb) - 0.10) * scale * 4.0).min(18.0)
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
            let middle = x((a + b) / 2.0);
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
                bottom - 9.0 - ((s.strips[i + 1].height(s.kerb) - 0.10) * scale * 4.0).min(18.0)
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
        for m in markings {
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
                    y += s.dash_period(&m) * scale;
                }
            }
        }
    }
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
}
