//! Copy and paste of path rules (the traffic rules of an object's or road's paths) and
//! nothing else: the shape stays as it is. The rules travel through the system clipboard as
//! text, in the lines a map file holds them in:
//!
//! ```text
//! [rule]
//! 48
//! trafficdensity
//! 0.500
//! 0
//! ```
//!
//! They go over as they are, `extra` (the vehicle class) included. A rule is dropped only
//! when it names a path the target does not have.

use omsi_map::MapRule;

use crate::doc::{Document, Sel};

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

/// The rules as the clipboard's text: the lines of the map file (`[rule]`, path, kind,
/// value, extra), one block each.
pub fn to_text(rules: &[MapRule]) -> String {
    let mut s = String::new();
    crate::doc::save::rules(&mut s, rules);
    s
}

/// The rules in a clipboard text, if it is nothing but `[rule]` / `[kill_rule]` blocks (a
/// `[kill_rule]` is read past and left out). None: it is some other text, or holds no rule.
pub fn from_text(text: &str) -> Option<Vec<MapRule>> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let mut rules = Vec::new();
    while let Some(head) = lines.next() {
        let kill = match head.to_ascii_lowercase().as_str() {
            "[rule]" => false,
            "[kill_rule]" => true,
            _ => return None,
        };
        let path_index = lines.next()?.parse::<i32>().ok()?;
        let kind = lines.next()?;
        if kind.starts_with('[') || kind.contains(char::is_whitespace) {
            return None;
        }
        let value = lines.next()?.parse::<f64>().ok().filter(|v| v.is_finite())?;
        let extra = lines.next()?.parse::<f64>().ok().filter(|v| v.is_finite())?;
        if !kill {
            rules.push(MapRule { path_index, kind: kind.to_string(), value, extra, kill });
        }
    }
    (!rules.is_empty()).then_some(rules)
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

    #[test]
    fn the_text_is_a_map_files_rule_lines_and_reads_back() {
        let rules = vec![rule(48, "trafficdensity", 0.5, 0.0, false), rule(61, "trafficdensity", 0.0, 6.0, false), rule(18, "priority", 192.0, 0.0, false)];
        let text = to_text(&rules);
        assert!(text.starts_with("[rule]\n48\ntrafficdensity\n0.5\n0\n\n[rule]\n61\n"), "{text}");
        assert_eq!(from_text(&text), Some(rules));
    }

    #[test]
    fn a_map_files_own_lines_are_read() {
        let text = "[rule]\n18\npriority\n192\n0\n\n[rule]\n73\ntrafficdensity\n3.000\n3\n\n[kill_rule]\n5\nspeedlimit\n30\n0\n";
        let r = from_text(text).unwrap();
        assert_eq!(r, vec![rule(18, "priority", 192.0, 0.0, false), rule(73, "trafficdensity", 3.0, 3.0, false)], "the kill_rule is left out");
    }

    #[test]
    fn other_text_is_not_rules() {
        for t in ["", "hello", "[rule]\n1\nbus\n0", "[rule]\nx\nbus\n0\n0", "[object]\n0\nfoo.sco", "[rule]\n1\nbus\n0\n0\nstray", "[kill_rule]\n1\nbus\n0\n0\n"] {
            assert_eq!(from_text(t), None, "{t:?}");
        }
    }
}
