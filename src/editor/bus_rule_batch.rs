use super::*;
use crate::road_maker::bus_rules::{self, batch::Group};

impl Editor {
    pub fn preview_all_bus_rules(&self) -> Vec<Group> {
        self.doc
            .as_ref()
            .map(|doc| bus_rules::batch::preview(doc, self.content_dir.as_deref()))
            .unwrap_or_default()
    }

    /// Only called after the user confirms the displayed groups. Opening a map
    /// never invokes this operation. Refuse an outdated confirmation preview.
    pub fn apply_all_bus_rules(&mut self, preview: &[Group]) -> Result<usize, String> {
        if self.preview_all_bus_rules() != preview {
            return Err(tr(
                "Roads changed; review the preview again",
                "Дороги изменились; проверьте список ещё раз",
            )
            .into());
        }
        let Some(doc) = self.doc.as_mut() else {
            return Ok(0);
        };
        if preview.is_empty() {
            return Ok(0);
        }
        let needs_rules = preview.iter().any(|group| {
            group.splines.iter().any(|&(k, id)| {
                doc.spline(k, id).is_some_and(|s| {
                    group.paths.iter().any(|&p| {
                        ["bus", "no_cars"].iter().any(|kind| {
                            !s.rules
                                .iter()
                                .any(|r| r.path_index == p as i32 && r.kind == *kind && !r.kill)
                        })
                    })
                })
            })
        });
        if !needs_rules {
            return Ok(0);
        }
        if self.content_dir.is_some() {
            doc.content_root = self.content_dir.clone();
        }
        self.history.begin(tr(
            "Apply bus lane rules to all existing roads",
            "Применить правила автобусных полос ко всем существующим дорогам",
        ));
        let mut added = 0;
        for group in preview {
            for &(key, _) in &group.splines {
                self.history.touch_tile(doc, key);
            }
            let count = |doc: &Document| {
                group
                    .splines
                    .iter()
                    .filter_map(|&(k, id)| doc.spline(k, id))
                    .map(|s| s.rules.len())
                    .sum::<usize>()
            };
            let before = count(doc);
            bus_rules::apply_for_splines(doc, &group.splines, "editor::apply_all_bus_rules");
            added += count(doc) - before;
        }
        self.history.commit();
        Ok(added)
    }
}
