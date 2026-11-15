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
    },
    Footway {
        width: f32,
        tex: PathBuf,
    },
    BusBay {
        width: f32,
    },
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
            Self::Footway { .. } | Self::Median { raised: true, .. } => 0.10 + kerb,
            _ => 0.10,
        }
    }

    pub fn is_roadway(&self) -> bool {
        matches!(self, Self::Lane { .. } | Self::Median { .. })
    }
}

/// Validate before writing any files; an invalid editor state remains editable.
pub fn validate(strips: &[Strip], left_hand: bool) -> Result<(), String> {
    let first = strips
        .iter()
        .position(|s| matches!(s, Strip::Lane { .. }))
        .ok_or_else(|| "道路至少需要一條 Lane".to_string())?;
    let last = strips
        .iter()
        .rposition(|s| matches!(s, Strip::Lane { .. }))
        .unwrap();
    for (i, strip) in strips.iter().enumerate() {
        let w = strip.width();
        if !w.is_finite() || w < 0.0 || (w == 0.0 && !matches!(strip, Strip::Median { .. })) {
            return Err(format!("積木 {} 的寬度無效", i + 1));
        }
        match strip {
            Strip::Footway { .. } if i != 0 && i + 1 != strips.len() => {
                return Err("Footway 只能位於道路兩端".into());
            }
            Strip::BusBay { .. } if i.checked_add(1) != Some(first) && i != last + 1 => {
                return Err("BusBay 只能緊貼車道段外緣，各側最多一個".into());
            }
            Strip::Median { .. } if i < first || i > last => {
                return Err("Median 必須位於車道段內".into());
            }
            _ => {}
        }
        if (first..=last).contains(&i) && !strip.is_roadway() {
            return Err("車道段內只能包含 Lane 和 Median".into());
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
                return Err("車道方向排列不符合此道路的行駛側設定".into());
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
            },
            Strip::BusBay { width: 3.0 },
            lane(Direction::Back, 3.0),
            Strip::Median {
                width: 2.0,
                raised: true,
                tex: "grass.png".into(),
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
        assert!(validate(&strips, false).is_ok());
        assert!(validate(&strips, true).is_err());
    }

    #[test]
    fn rejects_nonfinite_widths_and_disallowed_orderings() {
        let foot = Strip::Footway {
            width: 2.0,
            tex: PathBuf::new(),
        };
        let median = Strip::Median {
            width: 2.0,
            raised: false,
            tex: PathBuf::new(),
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
            assert!(validate(&strips, false).is_err(), "{strips:?}");
        }
    }
}
