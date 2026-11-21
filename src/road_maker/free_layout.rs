//! Emission for layouts not representable by the original quick parameters.
use super::*;

pub(super) fn emit(spec: &RoadSpec) -> String {
    let strips = &spec.strips;
    let positions = strips::positions(strips);
    let mut text = format!(
        "File created with openMapper\n\n[friendlyname]\n{}\n\n",
        spec.name.trim()
    );
    if spec.source_height_profile.is_empty() {
        for (strip, (lo, hi)) in strips.iter().zip(&positions) {
            let h = strip.height(spec.kerb);
            let _ = write!(
                text,
                "[heightprofile]\n{}\n{}\n{}\n{}\n\n",
                num(spec.output_x(*lo)),
                num(spec.output_x(*hi)),
                num(h),
                num(h)
            );
        }
    } else {
        // The imported spline's own collision heights, which may differ from the visible
        // surface: an asphalt shoulder can lie under a raised kerb.
        for span in collision_profile(spec, strips, &positions) {
            let _ = write!(
                text,
                "[heightprofile]\n{}\n{}\n{}\n{}\n\n",
                num(spec.output_x(span.x0)),
                num(spec.output_x(span.x1)),
                num(span.z),
                num(span.z)
            );
        }
    }
    let mut textures = vec![spec.roadway_tex_name()];
    let mut texture_index = |path: &Path| -> usize {
        if path.as_os_str().is_empty() {
            return 0;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if let Some(index) = textures.iter().position(|s| s == &name) {
            index
        } else {
            textures.push(name);
            textures.len() - 1
        }
    };
    let side_index = texture_index(&spec.footway_tex);
    let mut indices = Vec::new();
    for strip in strips {
        indices.push(match strip {
            Strip::Footway { tex, .. } => texture_index(if tex.as_os_str().is_empty() {
                &spec.footway_tex
            } else {
                tex
            }),
            Strip::Median { tex, .. } => texture_index(tex),
            _ => 0,
        });
    }
    let bay_index = textures.len();
    if strips.iter().any(|s| matches!(s, Strip::BusBay { .. })) {
        textures.push(spec.bay_tex_name());
    }
    for name in textures {
        let _ = write!(text, "[texture]\n{name}\n\n");
    }
    let profile = |s: &mut String, index, a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| {
        let _ = write!(s, "[profile]\n{index}\n\n");
        for (x, h, u, v) in [a, b] {
            let _ = write!(
                s,
                "[profilepnt]\n{}\n{}\n{}\n{}\n\n",
                num(spec.output_x(x)),
                num(h),
                num(u),
                num(v)
            );
        }
    };
    for (i, (strip, (lo, hi))) in strips.iter().zip(&positions).enumerate() {
        let h = strip.height(spec.kerb);
        let (index, u0, u1, v) = match strip {
            Strip::Lane { .. } => (
                0,
                spec.roadway_u(*lo),
                spec.roadway_u(*hi),
                spec.roadway_v(),
            ),
            Strip::BusBay { .. } => (bay_index, 0.0, 1.0, spec.roadway_v()),
            Strip::Footway { .. } if i == 0 => (indices[i], 0.187, 0.953, 0.2),
            Strip::Footway { .. } => (indices[i], 0.953, 0.187, 0.2),
            Strip::Median { width, tex, .. } if !tex.as_os_str().is_empty() => {
                (indices[i], 0.0, 1.0, *width)
            }
            Strip::Median { raised: true, .. } if !spec.footway_tex.as_os_str().is_empty() => {
                (side_index, 0.953, 0.187, 0.2)
            }
            Strip::Median { .. } => (
                0,
                spec.roadway_u(*lo),
                spec.roadway_u(*hi),
                spec.roadway_v(),
            ),
        };
        let footway_has_texture = |i: usize| match &strips[i] {
            Strip::Footway { tex, .. } => {
                !tex.as_os_str().is_empty() || !spec.footway_tex.as_os_str().is_empty()
            }
            _ => false,
        };
        if !matches!(strip, Strip::Footway { .. }) || footway_has_texture(i) {
            profile(&mut text, index, (*lo, h, u0, v), (*hi, h, u1, v));
        }
        if let Some(next) = strips.get(i + 1) {
            let next_h = next.height(spec.kerb);
            if h != next_h {
                let index = if matches!(strip, Strip::Footway { .. }) {
                    indices[i]
                } else if matches!(next, Strip::Footway { .. }) {
                    indices[i + 1]
                } else {
                    side_index
                };
                let (u0, u1) = if h < next_h {
                    (0.995, 0.953)
                } else {
                    (0.953, 0.995)
                };
                let available = if matches!(strip, Strip::Footway { .. }) {
                    footway_has_texture(i)
                } else if matches!(next, Strip::Footway { .. }) {
                    footway_has_texture(i + 1)
                } else {
                    spec.imported_sli || !spec.footway_tex.as_os_str().is_empty()
                };
                if available {
                    profile(&mut text, index, (*hi, h, u0, 0.2), (*hi, next_h, u1, 0.2));
                }
            }
        }
    }
    for (i, (strip, (lo, hi))) in strips.iter().zip(&positions).enumerate() {
        if !strip.has_ai_path() {
            continue;
        }
        let (kind, direction, scale) = match strip {
            Strip::Lane { dir, .. } => (0, dir.path_direction(), 0.9),
            Strip::Footway { .. } => (1, 2, 0.4),
            Strip::BusBay { .. } => {
                let adjacent = if strips[..i].iter().any(|s| matches!(s, Strip::Lane { .. })) {
                    strips[..i].iter().rev().find_map(|s| {
                        if let Strip::Lane { dir, .. } = s {
                            Some(*dir)
                        } else {
                            None
                        }
                    })
                } else {
                    strips[i + 1..].iter().find_map(|s| {
                        if let Strip::Lane { dir, .. } = s {
                            Some(*dir)
                        } else {
                            None
                        }
                    })
                };
                (0, adjacent.unwrap().path_direction(), 0.9)
            }
            Strip::Median { .. } => continue,
        };
        let _ = write!(
            text,
            "[path]\n{kind}\n{}\n{}\n{}\n{direction}\n\n",
            num(spec.strip_path_x(strip, *lo, *hi)),
            num(strip.height(spec.kerb)),
            num(strip.width() * scale)
        );
    }
    text
}

/// The collision heights of an imported road.
///
/// The imported spline keeps its own `[heightprofile]`, which may differ from the visible
/// surface: the asphalt shoulder of `2spur_11m_altonaer_d.sli` lies under a 0.250 m kerb.
/// The source's heights are kept where they cover the cross section; a strip that has been
/// raised above them keeps its own height.
fn collision_profile(spec: &RoadSpec, strips: &[Strip], positions: &[(f32, f32)]) -> Vec<HeightSpan> {
    let bounds: Vec<(f32, f32)> = strips.iter().zip(positions).map(|(_, (a, b))| (*a, *b)).collect();
    if bounds.is_empty() {
        return Vec::new();
    }
    let lo = bounds.iter().map(|(a, _)| *a).fold(f32::INFINITY, f32::min);
    let hi = bounds.iter().map(|(_, b)| *b).fold(f32::NEG_INFINITY, f32::max);
    let source: Vec<HeightSpan> = spec
        .source_height_profile
        .iter()
        .map(|s| HeightSpan { x0: s.x0.max(lo), x1: s.x1.min(hi), z: s.z })
        .filter(|s| s.x1 > s.x0)
        .collect();
    let mut borders: Vec<f32> = bounds.iter().flat_map(|(a, b)| [*a, *b]).collect();
    borders.extend(source.iter().flat_map(|s| [s.x0, s.x1]));
    borders.sort_by(|a, b| a.total_cmp(b));
    borders.dedup_by(|a, b| (*a - *b).abs() <= 1e-4);
    let mut out: Vec<HeightSpan> = Vec::new();
    for pair in borders.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if b - a <= 1e-4 {
            continue;
        }
        let mid = (a + b) / 2.0;
        let strip_height = strips
            .iter()
            .zip(&bounds)
            .find(|(_, (s0, s1))| mid > *s0 && mid < *s1)
            .map(|(s, _)| s.height(spec.kerb));
        let source_height = source.iter().find(|s| mid > s.x0 && mid < s.x1).map(|s| s.z);
        let z = match (strip_height, source_height) {
            (Some(h), Some(src)) => h.max(src),
            (Some(h), None) => h,
            (None, Some(src)) => src,
            (None, None) => continue,
        };
        match out.last_mut() {
            Some(last) if (last.z - z).abs() <= 1e-4 && (last.x1 - a).abs() <= 1e-4 => last.x1 = b,
            _ => out.push(HeightSpan { x0: a, x1: b, z }),
        }
    }
    out
}
