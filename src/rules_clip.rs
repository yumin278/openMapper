//! Copy and paste of path rules (the traffic rules of an object's or road's paths) and
//! nothing else: the shape stays as it is. The rules have a clipboard of their own, apart
//! from the clipboard of objects and roads (`clipboard.rs`).
//!
//! The rules go over as they are, `extra` (the vehicle class) included. A rule is dropped
//! only when it names a path the target does not have.

use omsi_map::MapRule;

use crate::doc::{Document, Sel};

/// The rules of one object or road, as copied.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RulesClip {
    /// Every rule that is not a `[kill_rule]`, in the order they stand.
    pub rules: Vec<MapRule>,
    /// The file (`.sco` or `.sli`) of the thing they were copied from.
    pub file: String,
    /// How many paths it has, when its model could be read (for the message and the log).
    pub paths: Option<usize>,
}

/// What a paste into one target did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pasted {
    /// The target's rules afterwards.
    pub after: Vec<MapRule>,
    /// How many rules were written.
    pub written: usize,
    /// The rules left out: they name a path the target does not have.
    pub dropped: Vec<MapRule>,
}

/// The file and the rules of a thing that has rules.
pub fn rules_of(doc: &Document, sel: Sel) -> Option<(String, Vec<MapRule>)> {
    match sel {
        Sel::Object(k, id) => crate::selection::any_object(doc, k, id).map(|o| (o.file.clone(), o.rules.clone())),
        Sel::Spline(k, id) => doc.spline(k, id).map(|s| (s.file.clone(), s.rules.clone())),
        Sel::Attached(k, id) => doc.attachment(k, id).map(|a| (a.file.clone(), a.rules.clone())),
    }
}

/// The rules to copy: all but the `[kill_rule]`s.
pub fn copied(rules: &[MapRule]) -> Vec<MapRule> {
    rules.iter().filter(|r| !r.kill).cloned().collect()
}

/// A target's new rules: its `[kill_rule]`s stay, its other rules are replaced by those of
/// `clip` that name a path below `paths`.
pub fn paste(target: &[MapRule], clip: &[MapRule], paths: usize) -> Pasted {
    let mut after: Vec<MapRule> = target.iter().filter(|r| r.kill).cloned().collect();
    let mut dropped = Vec::new();
    let mut written = 0;
    for r in clip.iter().filter(|r| !r.kill) {
        if r.path_index < 0 || r.path_index as usize >= paths {
            dropped.push(r.clone());
        } else {
            after.push(r.clone());
            written += 1;
        }
    }
    Pasted { after, written, dropped }
}

/// A rule in one line, for the log.
pub fn describe(r: &MapRule) -> String {
    format!("{}[path {}, {} {}, extra {}]", if r.kill { "kill_rule" } else { "rule" }, r.path_index, r.kind, r.value, r.extra)
}

/// Rules in one line, for the log.
pub fn describe_all(rules: &[MapRule]) -> String {
    if rules.is_empty() {
        return "(none)".to_string();
    }
    rules.iter().map(describe).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(path: i32, kind: &str, value: f64, extra: f64, kill: bool) -> MapRule {
        MapRule { path_index: path, kind: kind.into(), value, extra, kill }
    }

    #[test]
    fn kill_rules_are_not_copied() {
        let rules = vec![rule(0, "speedlimit", 30.0, 0.0, false), rule(1, "trafficdensity", 1.0, 4.0, true)];
        assert_eq!(copied(&rules), vec![rules[0].clone()]);
    }

    #[test]
    fn the_rules_are_replaced_and_the_kill_rules_stay() {
        let target = vec![rule(0, "speedlimit", 70.0, 0.0, false), rule(2, "trafficdensity", 1.0, 4.0, true), rule(3, "no_cars", 0.0, 0.0, false)];
        let clip = vec![rule(1, "speedlimit", 30.0, 0.0, false), rule(1, "speedlimit", 20.0, 4.0, false)];
        let p = paste(&target, &clip, 6);
        assert_eq!(p.after, vec![target[1].clone(), clip[0].clone(), clip[1].clone()]);
        assert_eq!((p.written, p.dropped.len()), (2, 0));
    }

    #[test]
    fn extra_goes_over_as_it_is() {
        let clip = vec![rule(0, "trafficdensity", 0.5, 4.0, false), rule(1, "speedlimit", 30.0, 7.5, false)];
        assert_eq!(paste(&[], &clip, 2).after, clip);
    }

    #[test]
    fn rules_of_paths_the_target_lacks_are_dropped() {
        let clip: Vec<MapRule> = (0..10).map(|i| rule(i, "speedlimit", 10.0 + i as f64, 0.0, false)).collect();
        let p = paste(&[], &clip, 6);
        assert_eq!(p.after.iter().map(|r| r.path_index).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(p.dropped.iter().map(|r| r.path_index).collect::<Vec<_>>(), vec![6, 7, 8, 9]);
        assert_eq!(p.written, 6);
        // (a negative number names no path either)
        assert_eq!(paste(&[], &[rule(-1, "bus", 0.0, 0.0, false)], 6).dropped.len(), 1);
    }

    #[test]
    fn an_empty_clip_clears_the_rules_but_the_kill_rules() {
        let target = vec![rule(0, "speedlimit", 70.0, 0.0, false), rule(1, "speedlimit", 70.0, 0.0, true)];
        let p = paste(&target, &[], 4);
        assert_eq!(p.after, vec![target[1].clone()]);
        assert_eq!(p.written, 0);
    }
}
