//! Infer a strip layout from the flat, textured carriageway of an existing spline.

use std::io::Write;

use anyhow::{Context, Result, ensure};
use omsi_scenery::Spline;

use super::{Direction, RoadSpec, Strip};
use crate::i18n::tr;
use crate::install::Roots;
use std::path::{Path, PathBuf};

const EPS: f32 = 0.005;

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
    let (mut spec, _texture) = infer(&spline, name, map_left_hand)?;
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

/// Adapt the geometry in MapWorks' FromProfile / AddCarriageway: neighboring
/// paths share the space between their origins in proportion to their widths;
/// the ends retain the paved room outside the outer paths as shoulders.
fn infer(spline: &Spline, name: String, map_left_hand: bool) -> Result<(RoadSpec, usize)> {
    let unsupported = || {
        tr(
            "Only flat roads with one continuous carriageway can be imported",
            "Импортируются только ровные дороги с одной непрерывной проезжей частью",
        )
    };
    let mut paths: Vec<_> = spline.paths.iter().filter(|p| p.kind == 0).collect();
    ensure!(
        !paths.is_empty() && spline.paths.iter().all(|p| matches!(p.kind, 0 | 1)),
        "{}",
        unsupported()
    );
    ensure!(
        paths.iter().all(|p| p.start[0].is_finite()
            && p.start[2].is_finite()
            && p.width.is_finite()
            && p.width > 0.0
            && matches!(p.direction, 0 | 1)),
        "{}",
        unsupported()
    );
    paths.sort_by(|a, b| a.start[0].total_cmp(&b.start[0]));
    ensure!(
        paths.windows(2).all(|p| p[0].start[0] < p[1].start[0]),
        "{}",
        unsupported()
    );

    let first = paths[0];
    let last = paths[paths.len() - 1];
    let z = first.start[2];
    let level = |h: f32| (h - z).abs() <= EPS;
    ensure!(paths.iter().all(|p| level(p.start[2])), "{}", unsupported());

    let hull = |mut spans: Vec<(f32, f32)>| {
        spans.retain(|(lo, hi)| lo.is_finite() && hi.is_finite() && hi > lo);
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f32, f32)> = Vec::new();
        for (lo, hi) in spans {
            if let Some(end) = merged.last_mut().filter(|end| lo <= end.1 + EPS) {
                end.1 = end.1.max(hi);
            } else {
                merged.push((lo, hi));
            }
        }
        merged
            .into_iter()
            .find(|&(lo, hi)| lo < first.start[0] && hi > last.start[0])
    };
    let height = hull(
        spline
            .height_profiles
            .iter()
            .filter(|p| level(p.z0) && level(p.z1))
            .map(|p| (p.x0.min(p.x1), p.x0.max(p.x1)))
            .collect(),
    )
    .context(unsupported())?;
    let (texture, (lo, hi)) = (0..spline.textures.len())
        .filter_map(|texture| {
            let surface = hull(
                spline
                    .profiles
                    .iter()
                    .filter(|p| p.texture == texture)
                    .flat_map(|p| p.points.windows(2))
                    .filter(|p| level(p[0].z) && level(p[1].z))
                    .map(|p| (p[0].x.min(p[1].x), p[0].x.max(p[1].x)))
                    .collect(),
            )?;
            Some((texture, (surface.0.max(height.0), surface.1.min(height.1))))
        })
        .max_by(|a, b| (a.1.1 - a.1.0).total_cmp(&(b.1.1 - b.1.0)))
        .context(unsupported())?;

    let mut strips = Vec::new();
    let shoulder = |width| Strip::Median {
        width,
        raised: false,
        tex: PathBuf::new(),
        height: None,
    };
    // The edge lane takes at most half its nominal path width on the outside.
    // Ignore trim smaller than the SLI coordinate precision.
    let a = (first.start[0] - first.width / 2.0).max(lo);
    let b = (last.start[0] + last.width / 2.0).min(hi);
    let mut edge = lo;
    if a - lo >= 0.001 {
        strips.push(shoulder(a - lo));
        edge = a;
    }
    for (i, p) in paths.iter().enumerate() {
        let next = paths
            .get(i + 1)
            .map_or(if hi - b >= 0.001 { b } else { hi }, |q| {
                p.start[0] + (q.start[0] - p.start[0]) * (p.width / (p.width + q.width))
            });
        let dir = if p.direction == 0 {
            Direction::Fwd
        } else {
            Direction::Back
        };
        strips.push(Strip::Lane {
            dir,
            width: next - edge,
            bus_only: false,
        });
        edge = next;
    }
    if hi - edge >= 0.001 {
        strips.push(shoulder(hi - edge));
    }
    let left_hand = if first.direction == last.direction {
        map_left_hand
    } else {
        first.direction == 0
    };
    let spec = RoadSpec {
        imported_sli: true,
        advanced: true,
        left_hand: Some(left_hand),
        name,
        strips,
        ..Default::default()
    };
    spec.validate().map_err(anyhow::Error::msg)?;
    Ok((spec, texture))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_maker::MakerTarget;

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
            });
            expected.push(Strip::Lane {
                dir: Direction::Fwd,
                width: v,
                bus_only: false,
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
