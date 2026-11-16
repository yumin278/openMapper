//! Hand-authored signal head and pole combinations for a map's automatic junctions.

use std::path::Path;

use glam::{DVec2, DVec3};
use omsi_map::MapObject;
use serde::Deserialize;

use crate::assets::Assets;
use crate::doc::{Document, tile_of, tile_origin};
use crate::history::History;

pub(crate) const FILE_NAME: &str = "signal_sets.json";

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct SignalSet {
    pub id: String,
    pub name: String,
    head: String,
    pole: String,
    pub head_height: f64,
    /// From the head to the pole in the head's frame: +x right, +y forward, +z up.
    pole_offset: [f64; 3],
    #[serde(default = "mirror_by_default")]
    mirror_x_lht: bool,
}

fn mirror_by_default() -> bool {
    true
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

#[derive(Deserialize)]
struct Config {
    version: u32,
    default: String,
    sets: Vec<serde_json::Value>,
}

pub(crate) struct SignalSets {
    sets: Vec<SignalSet>,
    default_id: Option<String>,
    builtin: SignalSet,
}

impl Default for SignalSets {
    fn default() -> Self {
        Self {
            sets: Vec::new(),
            default_id: None,
            builtin: SignalSet {
                id: "mc_small".into(),
                name: "MC 小型燈桿".into(),
                head: r"Sceneryobjects\Verkehrszeichen_MC\Ampel_Kfz_1.sco".into(),
                pole: r"Sceneryobjects\Verkehrszeichen_MC\Mast_Ampel_klein_1.sco".into(),
                head_height: 2.55,
                pole_offset: [0.0, -0.47, -2.55],
                mirror_x_lht: true,
            },
        }
    }
}

impl SignalSets {
    /// Replace the combinations with the ones for this map, including when the file disappears.
    pub fn reload(&mut self, map_dir: &Path, content: &Path) {
        *self = Self::load(map_dir, content);
    }

    fn load(map_dir: &Path, content: &Path) -> Self {
        let path = map_dir.join("openMapper").join(FILE_NAME);
        let mut result = Self::default();
        let parsed = std::fs::read(&path)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(serde_json::from_slice::<Config>(&bytes)?));
        match parsed {
            Err(e) => log::warn!(
                "signal sets {}: {e}; using built-in {}",
                path.display(),
                result.builtin.id
            ),
            Ok(config) if config.version != 1 => log::warn!(
                "signal sets {}: unsupported version {} (expected 1); using built-in {}",
                path.display(),
                config.version,
                result.builtin.id,
            ),
            Ok(config) => {
                for (index, entry) in config.sets.into_iter().enumerate() {
                    match serde_json::from_value::<SignalSet>(entry)
                        .map_err(anyhow::Error::from)
                        .and_then(|set| set.validated(content, &result.sets))
                    {
                        Ok(set) => result.sets.push(set),
                        Err(e) => log::warn!("signal sets {}: skipping sets[{index}]: {e}", path.display()),
                    }
                }
                if result.find(&config.default).is_some() {
                    result.default_id = Some(config.default);
                } else {
                    let why = if result.sets.is_empty() {
                        "no valid sets"
                    } else {
                        "default ID is unavailable"
                    };
                    log::warn!(
                        "signal sets {}: {why} (default {:?}); using built-in {}",
                        path.display(),
                        config.default,
                        result.builtin.id
                    );
                }
            }
        }
        log::info!(
            "signal sets {}: sets={:?}, effective default={:?}",
            path.display(),
            result.sets,
            result.resolve("").0,
        );
        result
    }

    fn find(&self, id: &str) -> Option<&SignalSet> {
        self.sets.iter().find(|set| set.id == id)
    }

    pub fn resolve(&self, requested: &str) -> (&SignalSet, &'static str) {
        if let Some(set) = self.find(requested) {
            return (set, "requested ID");
        }
        if let Some(set) = self.default_id.as_deref().and_then(|id| self.find(id)) {
            let why = if requested.is_empty() {
                "map default"
            } else {
                "requested ID unavailable; map default"
            };
            return (set, why);
        }
        let why = if requested.is_empty() {
            "map default unavailable; built-in"
        } else {
            "requested ID and map default unavailable; built-in"
        };
        (&self.builtin, why)
    }
}

impl SignalSet {
    fn validated(mut self, content: &Path, sets: &[Self]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            valid_id(&self.id),
            "invalid ID {:?}; expected [A-Za-z0-9_-]+",
            self.id
        );
        anyhow::ensure!(
            !sets.iter().any(|set| set.id == self.id),
            "duplicate ID {:?}",
            self.id
        );
        for (part, rel) in [("head", &mut self.head), ("pole", &mut self.pole)] {
            *rel = rel.replace('/', "\\");
            let resolved = omsi_cfg::resolve_path(content, rel);
            anyhow::ensure!(
                omsi_cfg::vfs::is_file(&resolved),
                "set {:?} {part} {rel:?}: no file at {}",
                self.id,
                resolved.display()
            );
        }
        Ok(self)
    }

    fn pole_position(&self, head: DVec3, heading: f64, left_hand: bool) -> DVec3 {
        let [mut x, y, z] = self.pole_offset;
        if left_hand && self.mirror_x_lht {
            x = -x;
        }
        // Same clockwise heading as the renderer's object_rotation with zero pitch and bank.
        head + crate::selection::turn_point(DVec2::new(x, y), DVec2::ZERO, heading).extend(z)
    }

    /// The .sco defines whether the stored z is in world coordinates or above the terrain.
    pub fn height_modes(&self, assets: &Assets) -> [bool; 2] {
        [&self.head, &self.pole].map(|file| {
            let path = assets.resolve(file);
            match omsi_scenery::SceneryObject::load(&path) {
                Ok(sco) => sco.absolute_height(),
                Err(e) => {
                    log::warn!(
                        "signal set {:?}: {}: {e}; using ground-relative height for {file:?}",
                        self.id,
                        path.display()
                    );
                    false
                }
            }
        })
    }

    pub fn place(
        &self,
        doc: &mut Document,
        history: Option<&mut History>,
        at: Placement,
        [head_abs, pole_abs]: [bool; 2],
    ) {
        let Placement {
            junction,
            light,
            heading,
            head,
        } = at;
        let pole = self.pole_position(head, heading, doc.global.left_hand_traffic);
        let (hk, pk) = (tile_of(head.truncate()), tile_of(pole.truncate()));
        let have = (doc.tiles.contains_key(&hk), doc.tiles.contains_key(&pk));
        if have != (true, true) {
            log::warn!(
                "junction {junction} signal set {:?} heading={heading}: skipped pair; head world={head:?} tile={hk:?} present={}; pole world={pole:?} tile={pk:?} present={}",
                self.id,
                have.0,
                have.1,
            );
            return;
        }
        if let Some(history) = history {
            history.touch_tile(doc, hk);
            history.touch_tile(doc, pk);
        }
        let stored_pos = |p: DVec3, k, absolute| {
            let xy = p.truncate();
            let z = if absolute { p.z } else { p.z - doc.ground(xy) };
            (xy - tile_origin(k)).extend(z).to_array()
        };
        let head_pos = stored_pos(head, hk, head_abs);
        let pole_pos = stored_pos(pole, pk, pole_abs);
        log::info!(
            "junction {junction} signal set {:?} heading={heading}: head world={head:?} tile={hk:?} pos={head_pos:?}; pole world={pole:?} tile={pk:?} pos={pole_pos:?}",
            self.id,
        );
        for (k, file, pos, extra) in [
            (pk, &self.pole, pole_pos, Vec::new()),
            (hk, &self.head, head_pos, vec![light.to_string()]),
        ] {
            let id = doc.new_id();
            let tile = doc.tiles.get_mut(&k).unwrap();
            tile.tile.objects.push(MapObject {
                file: file.clone(),
                id,
                pos,
                rot: [heading, 0.0, 0.0],
                flag: extra.len() as i32,
                extra,
                var_parent: Some(junction),
                ..Default::default()
            });
            tile.touch_content();
        }
    }
}

pub(crate) struct Placement {
    pub junction: i64,
    pub light: usize,
    pub heading: f64,
    pub head: DVec3,
}

#[cfg(test)]
mod tests;
