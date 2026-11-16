//! Compatibility at the old parameter-model boundary.

use super::{
    RoadSpec,
    strips::{Direction, Strip},
};
use std::path::Path;

pub fn from_legacy(spec: &RoadSpec, left_hand: bool) -> Vec<Strip> {
    let mut strips = Vec::new();
    if spec.footway > 0.0 {
        strips.push(Strip::Footway {
            width: spec.footway,
            tex: spec.footway_tex.clone(),
            path: true,
            height: None,
        });
    }
    let left_dir = if left_hand {
        Direction::Fwd
    } else {
        Direction::Back
    };
    let right_dir = if left_hand {
        Direction::Back
    } else {
        Direction::Fwd
    };
    strips.extend((0..spec.lanes_back).map(|_| Strip::Lane {
        dir: left_dir,
        width: spec.lane_width,
        bus_only: false,
    }));
    if spec.lanes_back > 0 && spec.lanes_fwd > 0 && spec.median > 0.0 {
        strips.push(Strip::Median {
            width: spec.median,
            raised: spec.median_raised,
            tex: spec.median_tex.clone(),
            height: None,
        });
    }
    strips.extend((0..spec.lanes_fwd).map(|_| Strip::Lane {
        dir: right_dir,
        width: spec.lane_width,
        bus_only: false,
    }));
    if spec.pocket > 0.0 {
        strips.push(Strip::BusBay { width: spec.pocket });
    }
    let right = spec.footway_right.unwrap_or(spec.footway);
    if right > 0.0 {
        strips.push(Strip::Footway {
            width: right,
            tex: spec.footway_tex.clone(),
            path: true,
            height: None,
        });
    }
    strips
}

/// The original maker appends its pocket after all through lanes. Take only the
/// declared through-lane count before sorting so that the pocket cannot vote.
pub fn infer_traffic(spec: &RoadSpec, sli: &Path, saved: Option<bool>, fallback: bool) -> bool {
    log::info!(
        "road traffic inference: sli={}, saved={saved:?}, lanes_back={}, lanes_fwd={}, pocket={}, map_left_hand={fallback}",
        sli.display(),
        spec.lanes_back,
        spec.lanes_fwd,
        spec.pocket
    );
    if let Some(left_hand) = saved {
        log::info!("road traffic inference: source=json, left_hand={left_hand}");
        return left_hand;
    }
    let count = usize::from(spec.lanes_back) + usize::from(spec.lanes_fwd);
    match omsi_scenery::Spline::load(sli) {
        Ok(spline) => {
            let mut lanes: Vec<_> = spline
                .paths
                .iter()
                .filter(|p| p.kind == 0)
                .take(count)
                .collect();
            log::info!(
                "road traffic inference: through paths (x, direction)={:?}",
                lanes
                    .iter()
                    .map(|p| (p.start[0], p.direction))
                    .collect::<Vec<_>>()
            );
            if count > 0 && lanes.len() == count && lanes.iter().all(|p| p.start[0].is_finite()) {
                lanes.sort_by(|a, b| a.start[0].total_cmp(&b.start[0]));
                let direction = lanes[0].direction;
                if direction == 0 || direction == 1 {
                    let left_hand = if spec.lanes_back > 0 {
                        direction == 0
                    } else {
                        direction == 1
                    };
                    log::info!(
                        "road traffic inference: source=sli, leftmost_direction={direction}, left_hand={left_hand}"
                    );
                    return left_hand;
                }
            }
        }
        Err(error) => log::warn!(
            "road traffic inference: cannot read {}: {error}",
            sli.display()
        ),
    }
    log::warn!(
        "road traffic inference: cannot determine traffic side for {}; using map_left_hand={fallback}",
        sli.display()
    );
    fallback
}
