use super::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub file: String,
    pub splines: Vec<(TileKey, i64)>,
    pub paths: Vec<usize>,
}

pub fn preview(doc: &Document, content: Option<&std::path::Path>) -> Vec<Group> {
    let mut groups = BTreeMap::<String, Vec<(TileKey, i64)>>::new();
    for (&key, tile) in &doc.tiles {
        for s in tile.tile.splines.iter().filter(|s| !s.deleted) {
            groups.entry(s.file.clone()).or_default().push((key, s.id));
        }
    }
    groups
        .into_iter()
        .filter_map(|(file, splines)| {
            let paths = paths_for(doc, &file, content);
            (!paths.is_empty()).then_some(Group {
                file,
                splines,
                paths,
            })
        })
        .collect()
}
