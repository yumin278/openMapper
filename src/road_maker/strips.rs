//! Ordered road cross sections. Coordinates are relative to the through roadway.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Actual OMSI travel direction, independent of traffic handedness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Fwd,
    Back,
}

impl Direction {
    pub fn path_direction(self) -> u8 {
        match self {
            Self::Fwd => 0,
            Self::Back => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Strip {
    Lane {
        dir: Direction,
        width: f32,
        bus_only: bool,
    },
    Median {
        width: f32,
        raised: bool,
        tex: PathBuf,
        /// The strip's height (m) above the ground; `None` derives it from `raised`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<f32>,
    },
    Footway {
        width: f32,
        tex: PathBuf,
        /// Whether the footway gets an AI path (`[path]`) of its own.
        #[serde(default = "path_default", skip_serializing_if = "path_is_default")]
        path: bool,
        /// The footway's height (m) above the ground; `None` derives it from the kerb.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<f32>,
    },
    BusBay {
        width: f32,
    },
}

fn path_default() -> bool {
    true
}

fn path_is_default(path: &bool) -> bool {
    *path
}

impl Strip {
    pub fn width(&self) -> f32 {
        match self {
            Self::Lane { width, .. }
            | Self::Median { width, .. }
            | Self::Footway { width, .. }
            | Self::BusBay { width } => *width,
        }
    }

    pub fn height(&self, kerb: f32) -> f32 {
        match self {
            Self::Footway { height: Some(h), .. } | Self::Median { height: Some(h), .. } => *h,
            Self::Footway { .. } | Self::Median { raised: true, .. } => super::ROAD_H + kerb,
            _ => super::ROAD_H,
        }
    }

    /// Whether this strip writes a `[path]` of its own: every strip but a middle strip and a
    /// footway whose AI path is turned off.
    pub fn has_ai_path(&self) -> bool {
        match self {
            Self::Median { .. } => false,
            Self::Footway { path, .. } => *path,
            _ => true,
        }
    }

    pub fn is_roadway(&self) -> bool {
        matches!(self, Self::Lane { .. } | Self::Median { .. })
    }
}

/// Validate before writing any files; an invalid editor state remains editable.
pub fn validate(strips: &[Strip], left_hand: bool, kerb: f32) -> Result<(), String> {
    let first = strips
        .iter()
        .position(|s| matches!(s, Strip::Lane { .. }))
        .ok_or_else(|| crate::i18n::tr("A road needs at least one Lane", "Дороге нужна хотя бы одна полоса движения").to_string())?;
    let last = strips
        .iter()
        .rposition(|s| matches!(s, Strip::Lane { .. }))
        .unwrap();
    for (i, strip) in strips.iter().enumerate() {
        let w = strip.width();
        if !w.is_finite() || w < 0.0 || (w == 0.0 && !matches!(strip, Strip::Median { .. })) {
            return Err(crate::i18n::tr("Strip {} has an invalid width", "Недопустимая ширина полосы {}").replace("{}", &(i + 1).to_string()));
        }
        let h = strip.height(kerb);
        if !h.is_finite() || h < super::ROAD_H - 1e-4 {
            return Err(crate::i18n::tr(
                "The median and footway must not be lower than the roadway",
                "Разделительная полоса и тротуар не могут быть ниже проезжей части",
            )
            .into());
        }
        match strip {
            Strip::Footway { .. } if i != 0 && i + 1 != strips.len() => {
                return Err(crate::i18n::tr("Footways must be at the outer ends", "Тротуары должны быть по краям дороги").into());
            }
            Strip::BusBay { .. } if i.checked_add(1) != Some(first) && i != last + 1 => {
                return Err(crate::i18n::tr("BusBays must border the roadway, at most one per side", "Автобусные карманы должны примыкать к дороге, не более одного с каждой стороны").into());
            }
            Strip::Median { raised, .. }
                if (i < first || i > last) && (*raised || h != super::ROAD_H) =>
            {
                return Err(crate::i18n::tr("Medians must be inside the roadway", "Разделительные полосы должны быть внутри проезжей части").into());
            }
            _ => {}
        }
        if (first..=last).contains(&i) && !strip.is_roadway() {
            return Err(crate::i18n::tr("The roadway may contain only Lanes and Medians", "Проезжая часть может содержать только полосы движения и разделительные полосы").into());
        }
    }
    let left_dir = if left_hand {
        Direction::Fwd
    } else {
        Direction::Back
    };
    let mut reached_right = false;
    for strip in strips {
        if let Strip::Lane { dir, .. } = strip {
            if *dir != left_dir {
                reached_right = true;
            } else if reached_right {
                return Err(crate::i18n::tr("Lane directions do not match this road’s traffic side", "Направления полос не соответствуют стороне движения этой дороги").into());
            }
        }
    }
    Ok(())
}

/// Each strip's left and right edges, accumulated from its actual width.
pub fn positions(strips: &[Strip]) -> Vec<(f32, f32)> {
    let roadway: f32 = strips
        .iter()
        .filter(|s| s.is_roadway())
        .map(Strip::width)
        .sum();
    let before: f32 = strips
        .iter()
        .take_while(|s| !s.is_roadway())
        .map(Strip::width)
        .sum();
    let mut x = -roadway / 2.0 - before;
    strips
        .iter()
        .map(|s| {
            let start = x;
            x += s.width();
            (start, x)
        })
        .collect()
}

pub fn roadway_width(strips: &[Strip]) -> f32 {
    strips
        .iter()
        .filter(|s| s.is_roadway())
        .map(Strip::width)
        .sum()
}

pub fn canvas_width(strips: &[Strip]) -> f32 {
    strips
        .iter()
        .filter(|s| s.is_roadway())
        .map(|s| match s {
            Strip::Median { tex, .. } if !tex.as_os_str().is_empty() => 0.0,
            _ => s.width(),
        })
        .sum::<f32>()
        .max(0.5)
}

pub fn roadway_u(strips: &[Strip], x: f32) -> f32 {
    let mut used = x + roadway_width(strips) / 2.0;
    let mut removed = 0.0;
    for (strip, (lo, _)) in strips.iter().zip(positions(strips)) {
        if let Strip::Median { width, tex, .. } = strip {
            if !tex.as_os_str().is_empty() {
                used -= (x - lo).clamp(0.0, *width);
                removed += width;
            }
        }
    }
    used / (roadway_width(strips) - removed).max(0.1)
}

/// Boundaries of the divider between the two directional lane groups.
pub fn median_edges(strips: &[Strip], left_hand: bool) -> (f32, f32) {
    let left_dir = if left_hand {
        Direction::Fwd
    } else {
        Direction::Back
    };
    let mut left_end = -roadway_width(strips) / 2.0;
    let mut right_start = roadway_width(strips) / 2.0;
    for (strip, (lo, hi)) in strips.iter().zip(positions(strips)) {
        if let Strip::Lane { dir, .. } = strip {
            if *dir == left_dir {
                left_end = hi;
            } else {
                right_start = lo;
                break;
            }
        }
    }
    (left_end, right_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(dir: Direction, width: f32) -> Strip {
        Strip::Lane {
            dir,
            width,
            bus_only: false,
        }
    }

    #[test]
    fn widths_and_origin_exclude_bays_and_footways() {
        let strips = vec![
            Strip::Footway {
                width: 2.0,
                tex: "side.png".into(),
                path: true,
                height: None,
            },
            Strip::BusBay { width: 3.0 },
            lane(Direction::Back, 3.0),
            Strip::Median {
                width: 2.0,
                raised: true,
                tex: "grass.png".into(),
                height: None,
            },
            lane(Direction::Fwd, 4.0),
            Strip::BusBay { width: 2.5 },
        ];
        assert_eq!(roadway_width(&strips), 9.0);
        assert_eq!(canvas_width(&strips), 7.0);
        assert_eq!(
            positions(&strips),
            vec![
                (-9.5, -7.5),
                (-7.5, -4.5),
                (-4.5, -1.5),
                (-1.5, 0.5),
                (0.5, 4.5),
                (4.5, 7.0)
            ]
        );
        assert_eq!(median_edges(&strips, false), (-1.5, 0.5));
        assert_eq!(roadway_u(&strips, -4.5), 0.0);
        assert_eq!(roadway_u(&strips, 4.5), 1.0);
        assert_eq!(roadway_u(&strips, -1.5), roadway_u(&strips, 0.5));
        assert!(validate(&strips, false, 0.15).is_ok());
        assert!(validate(&strips, true, 0.15).is_err());
    }

    #[test]
    fn rejects_nonfinite_widths_and_disallowed_orderings() {
        let foot = Strip::Footway {
            width: 2.0,
            tex: PathBuf::new(),
            path: true,
            height: None,
        };
        let median = Strip::Median {
            width: 2.0,
            raised: true,
            tex: PathBuf::new(),
            height: None,
        };
        for strips in [
            vec![],
            vec![lane(Direction::Fwd, f32::NAN)],
            vec![median, lane(Direction::Fwd, 3.0)],
            vec![lane(Direction::Back, 3.0), foot, lane(Direction::Fwd, 3.0)],
            vec![lane(Direction::Fwd, 3.0), lane(Direction::Back, 3.0)],
            vec![
                Strip::BusBay { width: 3.0 },
                Strip::BusBay { width: 3.0 },
                lane(Direction::Fwd, 3.0),
            ],
        ] {
            assert!(validate(&strips, false, 0.15).is_err(), "{strips:?}");
        }
    }

    #[test]
    fn a_specified_height_overrides_raised_and_the_kerb() {
        let foot = Strip::Footway {
            width: 2.0,
            tex: PathBuf::new(),
            path: true,
            height: Some(0.30),
        };
        let raised = Strip::Median {
            width: 2.0,
            raised: true,
            tex: PathBuf::new(),
            height: Some(0.25),
        };
        assert_eq!(foot.height(0.15), 0.30);
        assert_eq!(raised.height(0.15), 0.25);
        // `raised` is ignored once a height is set.
        let level = Strip::Median {
            width: 2.0,
            raised: false,
            tex: PathBuf::new(),
            height: Some(0.25),
        };
        assert_eq!(level.height(0.15), 0.25);
        // Without a height the old behaviour holds.
        let plain = Strip::Median {
            width: 2.0,
            raised: false,
            tex: PathBuf::new(),
            height: None,
        };
        assert_eq!(plain.height(0.15), 0.10);
    }
}
