//! Rules live on map instances; road files only describe which paths need them.
use super::RoadSpec;
use crate::doc::{Document, TileKey};
use omsi_map::MapSpline;

pub fn paths_for(doc: &Document, file: &str) -> Vec<usize> {
    // Made roads live in the map's content installation. A source map can also
    // refer to a made road beside the installation it was read from.
    for map in std::iter::once(doc.dir.as_path()).chain(doc.source.as_deref()) {
        let Some(root) = map.parent().and_then(|p| p.parent()) else {
            continue;
        };
        let sli = omsi_cfg::resolve_path(root, file);
        if sli.with_extension("road.json").is_file() {
            return RoadSpec::of_with_traffic(&sli, doc.global.left_hand_traffic)
                .map(|s| {
                    let mut paths = s.bus_paths;
                    paths.extend(s.bay_paths);
                    paths.sort_unstable();
                    paths.dedup();
                    paths
                })
                .unwrap_or_default();
        }
    }
    Vec::new()
}

fn log_rule(caller: &str, phase: &str, spline: &MapSpline, path: i32, kind: &str) {
    log::info!(
        "bus lane rules: caller={caller} phase={phase} spline_id={} file={:?} path_index={path} kind={kind} rules={:?}",
        spline.id,
        spline.file,
        spline.rules
    );
}

fn add(spline: &mut MapSpline, paths: &[usize], caller: &str) -> bool {
    let mut changed = false;
    for &path in paths {
        let Ok(path_index) = i32::try_from(path) else {
            continue;
        };
        for kind in ["bus", "no_cars"] {
            if spline
                .rules
                .iter()
                .any(|r| r.path_index == path_index && r.kind == kind && !r.kill)
            {
                continue;
            }
            log_rule(caller, "before-add", spline, path_index, kind);
            spline.rules.push(omsi_map::MapRule {
                path_index,
                kind: kind.into(),
                ..Default::default()
            });
            log_rule(caller, "after-add", spline, path_index, kind);
            changed = true;
        }
    }
    changed
}

/// Idempotent and never renumbers existing rules.
pub fn apply_bus_lane_rules(doc: &mut Document, splines: &[(TileKey, i64)], bus_paths: &[usize]) {
    apply_logged(doc, splines, bus_paths, "apply_bus_lane_rules");
}

pub fn apply_logged(doc: &mut Document, splines: &[(TileKey, i64)], paths: &[usize], caller: &str) {
    for &(key, id) in splines {
        let Some(tile) = doc.tiles.get_mut(&key) else {
            continue;
        };
        let Some(spline) = tile
            .tile
            .splines
            .iter_mut()
            .find(|s| s.id == id && !s.deleted)
        else {
            continue;
        };
        if add(spline, paths, caller) {
            tile.touch_content();
        }
    }
}

/// Called after insertion, while the caller's history step is still open.
pub fn apply_for_splines(doc: &mut Document, splines: &[(TileKey, i64)], caller: &str) {
    let mut by_file = std::collections::BTreeMap::<String, Vec<(TileKey, i64)>>::new();
    for &(key, id) in splines {
        if let Some(s) = doc.spline(key, id) {
            by_file.entry(s.file.clone()).or_default().push((key, id));
        }
    }
    for (file, instances) in by_file {
        let paths = paths_for(doc, &file);
        if !paths.is_empty() {
            apply_logged(doc, &instances, &paths, caller);
        }
    }
}

/// Only a complete live bus/no_cars pair on the old declared indices is removed.
pub fn remove_old(doc: &mut Document, key: TileKey, id: i64, paths: &[usize], caller: &str) {
    let Some(tile) = doc.tiles.get_mut(&key) else {
        return;
    };
    let Some(spline) = tile
        .tile
        .splines
        .iter_mut()
        .find(|s| s.id == id && !s.deleted)
    else {
        return;
    };
    let mut changed = false;
    for &path in paths {
        let Ok(index) = i32::try_from(path) else {
            continue;
        };
        if !["bus", "no_cars"].iter().all(|kind| {
            spline
                .rules
                .iter()
                .any(|r| r.path_index == index && r.kind == *kind && !r.kill)
        }) {
            continue;
        }
        for kind in ["bus", "no_cars"] {
            log_rule(caller, "before-remove", spline, index, kind);
            spline
                .rules
                .retain(|r| !(r.path_index == index && r.kind == kind && !r.kill));
            log_rule(caller, "after-remove", spline, index, kind);
        }
        changed = true;
    }
    if changed {
        tile.touch_content();
    }
}

pub fn retype(doc: &mut Document, splines: &[(TileKey, i64)], file: &str, caller: &str) {
    for &(key, id) in splines {
        let Some(old) = doc.spline(key, id).map(|s| s.file.clone()) else {
            continue;
        };
        let paths = paths_for(doc, &old);
        remove_old(doc, key, id, &paths, caller);
        if let Some(tile) = doc.tiles.get_mut(&key) {
            if let Some(s) = tile
                .tile
                .splines
                .iter_mut()
                .find(|s| s.id == id && !s.deleted)
            {
                s.file = file.into();
            }
            tile.touch_content();
        }
    }
    apply_for_splines(doc, splines, caller);
}

#[cfg(test)]
mod tests;
