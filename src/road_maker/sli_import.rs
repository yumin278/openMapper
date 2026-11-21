//! Infer a strip layout from the cross section of an existing spline.

use std::io::Write;

use anyhow::{Context, Result, ensure};
use omsi_scenery::Spline;

use super::{Direction, RoadSpec, Strip};
use crate::i18n::tr;
use crate::install::Roots;
use std::path::{Path, PathBuf};

const EPS: f32 = 0.005;
type Span = (f32, f32);

pub(super) fn invalid_paths() -> &'static str {
    tr(
        "The vehicle path data or directions cannot be represented",
        "Данные или направления автомобильных путей невозможно представить"
    )
}

fn merged_spans(spans: impl Iterator<Item = Span>) -> Vec<Span> {
    let mut spans: Vec<_> = spans
        .filter(|&(lo, hi)| lo.is_finite() && hi.is_finite() && hi > lo)
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<Span> = Vec::new();
    for (lo, hi) in spans {
        if let Some(end) = merged.last_mut().filter(|end| lo <= end.1 + EPS) {
            end.1 = end.1.max(hi);
        } else {
            merged.push((lo, hi));
        }
    }
    merged
}

/// Only create the sidecar. In particular, do not use the maker's normal road writer.
pub(crate) fn create(
    roots: &Roots,
    rel: &str,
    name: String,
    map_left_hand: bool,
) -> Result<PathBuf> {
    let source = roots.resolve(rel);
    ensure!(
        source.is_file()
            && source
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("sli")),
        tr(
            "The source must be a local SLI file",
            "Источник должен быть локальным файлом SLI"
        )
    );
    let spline = Spline::load(&source)?;
    let mut spec = infer(&spline, name, map_left_hand)?;
    // Every road the maker writes starts on the stock plain asphalt; without it the maker
    // warns and the user chooses a roadway picture by hand.
    match stock_roadway_texture(&source, roots) {
        Some(found) => {
            log::info!(
                "road maker: imported road uses the default roadway texture {}",
                found.display()
            );
            spec.roadway_tex = found;
        }
        None => log::warn!(
            "road maker: {} is missing from the texture folders; the roadway texture must be chosen by hand",
            super::DEFAULT_ROADWAY_TEX
        ),
    }
    let mut json = serde_json::to_value(&spec)?;
    for field in RoadSpec::LEGACY_FIELDS {
        json.as_object_mut().unwrap().remove(field);
    }
    let bytes = serde_json::to_vec_pretty(&json)?;
    let path = source.with_extension("road.json");
    std::fs::File::create_new(&path)?.write_all(&bytes)?;
    Ok(path)
}

/// The stock asphalt for an imported road: by its exact name (case-insensitive) in the
/// source's texture folders first, then in the spline texture folders the maker offers (the
/// stock asphalt is often only in another installed spline's folder, e.g.
/// `Splines\Marcel\texture`). An explicit texture path the source names is not consulted:
/// imported roads start on the maker's stock asphalt like every other road.
pub(super) fn stock_roadway_texture(source: &Path, roots: &Roots) -> Option<PathBuf> {
    let dirs = crate::assets::texture_dirs(
        &roots.omsi,
        &roots.content,
        source.parent().unwrap_or(Path::new(".")),
    );
    super::find_named_texture_in_dirs(super::DEFAULT_ROADWAY_TEX, &dirs).or_else(|| {
        super::find_named_texture(&crate::assets::spline_textures(roots), super::DEFAULT_ROADWAY_TEX)
    })
}

/// Adapt the geometry in MapWorks' FromProfile / AddCarriageway: neighboring
/// paths share the space between their origins in proportion to their widths;
/// the ends retain the paved room outside the outer paths as shoulders.
fn infer(spline: &Spline, name: String, map_left_hand: bool) -> Result<RoadSpec> {
    let no_surface = tr(
        "A vehicle path has no matching flat road surface",
        "Для автомобильного пути не найдено ровное дорожное покрытие"
    );
    let gap = tr(
        "The road surface has a gap without cross-section data",
        "В дорожном покрытии есть разрыв без данных поперечного профиля"
    );
    let median_error = tr(
        "The central divider cannot be represented by an existing strip",
        "Центральную разделительную зону невозможно представить существующей полосой"
    );
    let mut paths: Vec<_> = spline.paths.iter().filter(|p| p.kind == 0).collect();
    ensure!(
        !paths.is_empty() && spline.paths.iter().all(|p| matches!(p.kind, 0 | 1)),
        "{}",
        invalid_paths()
    );
    ensure!(
        paths.iter().all(|p| p.start[0].is_finite()
            && p.start[2].is_finite()
            && p.width.is_finite()
            && p.width > 0.0
            && matches!(p.direction, 0 | 1)),
        "{}",
        invalid_paths()
    );
    paths.sort_by(|a, b| a.start[0].total_cmp(&b.start[0]));
    ensure!(
        paths.windows(2).all(|p| p[0].start[0] < p[1].start[0]),
        "{}",
        invalid_paths()
    );

    let first = paths[0];
    let last = paths[paths.len() - 1];
    let height_spans = |at: f32| {
        merged_spans(spline.height_profiles.iter()
            .filter(move |p| (p.z0 - at).abs() <= EPS && (p.z1 - at).abs() <= EPS)
            .map(|p| (p.x0.min(p.x1), p.x0.max(p.x1))))
    };
    // Geometry is independent of texture indices. Intersect render profiles
    // with collision profiles so neither can invent a strip on its own.
    let surface_spans = |at: f32| {
        merged_spans(spline.profiles.iter().flat_map(|p| p.points.windows(2))
            .filter(move |p| (p[0].z - at).abs() <= EPS && (p[1].z - at).abs() <= EPS)
            .map(|p| (p[0].x.min(p[1].x), p[0].x.max(p[1].x))))
    };
    let surfaces = surface_spans(super::ROAD_H);
    let carriageways: Vec<_> = height_spans(super::ROAD_H)
        .into_iter()
        .flat_map(|(lo, hi)| surfaces.iter().map(move |&(a, b)| (lo.max(a), hi.min(b))))
        .filter(|&(lo, hi)| hi > lo)
        .collect();
    let assigned = paths
        .iter()
        .map(|p| {
            ensure!((p.start[2] - super::ROAD_H).abs() <= EPS, "{}", no_surface);
            carriageways
                .iter()
                .position(|&(lo, hi)| lo < p.start[0] && p.start[0] < hi)
                .context(no_surface)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut strips = Vec::new();
    let shoulder = |width| Strip::Median {
        width,
        raised: false,
        tex: PathBuf::new(),
        height: None,
    };
    let left_edge = carriageways[assigned[0]].0;
    let mut previous_end = left_edge;
    let mut start = 0;
    while start < paths.len() {
        let section = assigned[start];
        let end = start + assigned[start..].partition_point(|&s| s == section);
        let (lo, hi) = carriageways[section];
        if start != 0 {
            // Do not infer a divider merely from the absence of paths. Both
            // source profiles must cover the entire interval at one height.
            let covers = |(a, b): Span| a <= previous_end + EPS && b >= lo - EPS;
            let height_data = merged_spans(spline.height_profiles.iter()
                .filter(|p| p.z0.is_finite() && p.z1.is_finite())
                .map(|p| (p.x0.min(p.x1), p.x0.max(p.x1))));
            let surface_data = merged_spans(spline.profiles.iter().flat_map(|p| p.points.windows(2))
                .filter(|p| p[0].z.is_finite() && p[1].z.is_finite())
                .map(|p| (p[0].x.min(p[1].x), p[0].x.max(p[1].x))));
            ensure!(
                height_data.into_iter().any(covers) && surface_data.into_iter().any(covers),
                "{}",
                gap
            );
            let height = spline.height_profiles.iter()
                .map(|p| p.z0)
                .find(|&h| {
                    h.is_finite() && h > super::ROAD_H + EPS
                        && height_spans(h).into_iter().any(covers)
                        && surface_spans(h).into_iter().any(covers)
                })
                .context(median_error)?;
            strips.push(Strip::Median {
                width: lo - previous_end,
                raised: true,
                tex: PathBuf::new(),
                height: Some(height),
            });
        }
        let a = (paths[start].start[0] - paths[start].width / 2.0).max(lo);
        let b = (paths[end - 1].start[0] + paths[end - 1].width / 2.0).min(hi);
        let mut edge = lo;
        if a - lo >= 0.001 {
            strips.push(shoulder(a - lo));
            edge = a;
        }
        for i in start..end {
            let p = paths[i];
            let next = if i + 1 == end {
                if hi - b >= 0.001 { b } else { hi }
            } else {
                let q = paths[i + 1];
                p.start[0] + (q.start[0] - p.start[0]) * (p.width / (p.width + q.width))
            };
            strips.push(Strip::Lane {
                dir: if p.direction == 0 { Direction::Fwd } else { Direction::Back },
                width: next - edge,
                bus_only: false,
                path_offset: p.start[0] - (edge + next) / 2.0,
            });
            edge = next;
        }
        if hi - edge >= 0.001 {
            strips.push(shoulder(hi - edge));
        }
        previous_end = hi;
        start = end;
    }
    let left_hand = if first.direction == last.direction {
        map_left_hand
    } else {
        first.direction == 0
    };
    let mut directions = paths.iter().map(|p| p.direction);
    let initial = directions.next().unwrap();
    ensure!(
        directions.skip_while(|&d| d == initial).all(|d| d != initial),
        "{}",
        invalid_paths()
    );
    let spec = RoadSpec {
        imported_sli: true,
        source_x_origin: (left_edge + previous_end) / 2.0,
        advanced: true,
        left_hand: Some(left_hand),
        name,
        strips,
        ..Default::default()
    };
    spec.validate().map_err(anyhow::Error::msg)?;
    Ok(spec)
}

#[cfg(test)]
mod marcel_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_maker::MakerTarget;

    #[test]
    fn road_maker_sli_import_keeps_an_off_centre_path_and_manual_shift_separate() {
        let input = "[texture]\nasphalt.bmp\n\
            [heightprofile]\n0\n4\n0.1\n0.1\n\
            [profile]\n0\n[profilepnt]\n0\n0.1\n0\n1\n[profilepnt]\n4\n0.1\n1\n1\n\
            [path]\n0\n1.5\n0.1\n4\n0\n";
        let spline = Spline::parse(&omsi_cfg::CfgFile::from_str("offset.sli", input));
        let mut spec = infer(&spline, "Offset path".into(), false).unwrap();
        let pos = spec.strip_positions();
        assert_eq!(spec.source_x_origin, 2.0);
        assert_eq!(spec.x_offset, 0.0);
        assert_eq!(spec.strip_path_x(&spec.strips[0], pos[0].0, pos[0].1), 1.5);
        spec.x_offset = -2.0;
        let reopened: RoadSpec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(reopened.source_x_origin, 2.0);
        assert_eq!(reopened.strip_path_x(&reopened.strips[0], pos[0].0, pos[0].1), -0.5);
    }

    #[test]
    fn road_maker_sli_import_preserves_source_and_reopens_strips() {
        let dir = std::env::temp_dir().join(format!("om-road-sli-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let roots = Roots {
            omsi: dir.clone(),
            content: dir.clone(),
        };
        std::fs::write(dir.join("asphalt.bmp"), []).unwrap();
        for (name, lo, hi, x, y, w, v, left, right) in [
            ("Symmetric", -3.0, 3.0, -1.5, 1.5, 3.0, 3.0, 0.0, 0.0),
            ("Asymmetric", -4.0, 3.0, -2.0, 1.5, 4.0, 3.0, 0.0, 0.0),
            ("Shoulders", -5.0, 4.0, -1.5, 1.5, 3.0, 3.0, 2.0, 1.0),
        ] {
            let rel = format!("{name}.sli");
            let source = dir.join(&rel);
            let input = format!(
                "[texture]\nasphalt.bmp\n\n[texture]\nside.bmp\n\n\
                 [heightprofile]\n{lo}\n{hi}\n0.1\n0.1\n\n\
                 [heightprofile]\n{}\n{lo}\n0.25\n0.25\n\n\
                 [profile]\n1\n[profilepnt]\n{}\n0.25\n0\n1\n[profilepnt]\n{lo}\n0.25\n1\n1\n\n\
                 [profile]\n0\n[profilepnt]\n{lo}\n0.1\n0\n1\n[profilepnt]\n{hi}\n0.1\n1\n1\n\n\
                 [path]\n0\n{y}\n0.1\n{v}\n0\n\n[path]\n1\n{}\n0.25\n1\n2\n\n[path]\n0\n{x}\n0.1\n{w}\n1\n",
                lo - 2.0,
                lo - 2.0,
                lo - 1.0,
            );
            std::fs::write(&source, &input).unwrap();
            let json_path = create(&roots, &rel, name.into(), true).unwrap();
            let json = std::fs::read_to_string(&json_path).unwrap();
            let spec = RoadSpec::of_with_traffic(&source, true).unwrap();
            println!(
                "{name} input:\n{input}\nInferred strips: {:#?}\nJSON:\n{json}\nGenerated preview SLI:\n{}",
                spec.strips,
                spec.sli(true)
            );
            assert_eq!(std::fs::read(&source).unwrap(), input.as_bytes());
            assert!(MakerTarget::open(rel.clone(), source.clone()).is_some());
            assert!(spec.advanced && spec.imported_sli);
            assert_eq!(spec.left_hand, Some(false));
            assert_eq!(spec.sli(true), super::super::free_layout::emit(&spec));
            let mut expected = Vec::new();
            if left > 0.0 {
                expected.push(Strip::Median {
                    width: left,
                    raised: false,
                    tex: PathBuf::new(),
                    height: None,
                });
            }
            expected.push(Strip::Lane {
                dir: Direction::Back,
                width: w,
                bus_only: false,
                path_offset: 0.0,
            });
            expected.push(Strip::Lane {
                dir: Direction::Fwd,
                width: v,
                bus_only: false,
                path_offset: 0.0,
            });
            if right > 0.0 {
                expected.push(Strip::Median {
                    width: right,
                    raised: false,
                    tex: PathBuf::new(),
                    height: None,
                });
            }
            assert_eq!(spec.strips, expected);
            assert!(create(&roots, &rel, name.into(), true).is_err());
            assert_eq!(std::fs::read_to_string(&json_path).unwrap(), json);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
