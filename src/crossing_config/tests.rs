//! The settings beside a `.sco`: what is read back, what is refused, and what is left alone.

use super::*;

fn scratch(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("om-crossing-config-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sco = dir.join("Kreuzung.sco");
    std::fs::write(&sco, "[path]\n0\n0\n0\n0\n0\n10\n0\n0\n0\n3\n0\n0\n\n[path]\n0\n0\n0\n0\n0\n10\n0\n0\n0\n3\n0\n0\n").unwrap();
    (dir, sco)
}

fn point(light: usize, x: f32) -> SignalPoint {
    SignalPoint { light, set: String::new(), x, up: 0.0, fwd: 0.0, heading: 0.0 }
}

fn rule(index: i32, kind: &str) -> TemplateRule {
    TemplateRule { path_index: index, kind: kind.into(), value: 0.0, extra: 0.0 }
}

#[test]
fn what_is_written_is_read_back() {
    let (dir, sco) = scratch("round-trip");
    let config = CrossingConfig::of(Some("Sceneryobjects\\A\\B.sco".into()), vec![point(0, 1.5), point(1, -2.0)], vec![rule(0, "bus"), rule(1, "speedlimit")]);
    write(&sco, &config, 2).unwrap();
    let back = CrossingConfig::load(&sco, 2).unwrap();
    assert_eq!(back, config);
    assert!(back.is_copy());
    let text = std::fs::read_to_string(path_for(&sco)).unwrap();
    assert!(text.contains("\"version\": 1"), "{text}");
    assert!(text.contains("\"copy\""), "{text}");
    assert!(text.contains("\"rule_template\""), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn signals_and_a_template_without_a_copy_are_not_a_copy() {
    let (dir, sco) = scratch("no-copy");
    write(&sco, &CrossingConfig::of(None, vec![point(0, 0.0)], vec![rule(0, "bus")]), 2).unwrap();
    let back = CrossingConfig::load(&sco, 2).unwrap();
    assert!(!back.is_copy(), "nothing says it was copied");
    // (writing it again keeps it that way: nothing invents a source)
    write(&sco, &back, 2).unwrap();
    assert!(!CrossingConfig::load(&sco, 2).unwrap().is_copy());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_old_top_level_from_is_the_copy_marker() {
    let (dir, sco) = scratch("old-marker");
    std::fs::write(path_for(&sco), "{\"from\": \"Sceneryobjects\\\\Old\\\\Kreuzung.sco\"}").unwrap();
    let back = CrossingConfig::load(&sco, 2).unwrap();
    assert_eq!(back.copy_from.as_deref(), Some("Sceneryobjects\\Old\\Kreuzung.sco"));
    assert!(back.signals.is_empty() && back.rules.is_empty());
    // (the next write is the current form, with the source kept)
    write(&sco, &back, 2).unwrap();
    let text = std::fs::read_to_string(path_for(&sco)).unwrap();
    assert!(text.contains("\"version\": 1") && text.contains("\"copy\""), "{text}");
    assert_eq!(CrossingConfig::load(&sco, 2).unwrap().copy_from, back.copy_from);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_file_without_a_version_and_without_a_marker_is_refused() {
    let (dir, sco) = scratch("no-version");
    let path = path_for(&sco);
    std::fs::write(&path, "{\"signals\": []}").unwrap();
    assert!(CrossingConfig::load(&sco, 2).is_none());
    let bytes = std::fs::read(&path).unwrap();
    assert!(prepare(&sco, &CrossingConfig::of(None, vec![point(0, 0.0)], Vec::new()), 2).is_err(), "not written over");
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file is left as it was");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_later_version_and_broken_json_are_left_alone() {
    let (dir, sco) = scratch("future");
    let path = path_for(&sco);
    for text in ["{\"version\": 2, \"signals\": []}", "{not json", "[]", "\"text\""] {
        std::fs::write(&path, text).unwrap();
        assert!(CrossingConfig::load(&sco, 2).is_none(), "{text}");
        assert!(prepare(&sco, &CrossingConfig::of(None, vec![point(0, 0.0)], Vec::new()), 2).is_err(), "{text}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "left as it was: {text}");
        assert_eq!(CrossingConfig::counts(&sco), (0, 0), "a panel is offered nothing: {text}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unusable_entries_are_skipped_and_the_rest_read() {
    let (dir, sco) = scratch("invalid-entries");
    let text = serde_json::json!({
        "version": 1,
        "signals": [
            {"light": 0, "set": "", "x": 1.0, "up": 0.0, "fwd": 0.0, "heading": 0.0},
            {"light": 1, "x": 0.0, "up": 0.0, "fwd": 0.0, "heading": 0.0},
            {"light": 0, "set": "", "x": 1.0, "up": 0.0, "fwd": 0.0, "heading": 0.0},
            "not a signal"
        ],
        "rule_template": [
            {"path_index": 1, "kind": "bus", "value": 0, "extra": 0},
            {"path_index": 9, "kind": "bus", "value": 0, "extra": 0},
            {"path_index": 0, "kind": "two words", "value": 0, "extra": 0},
            {"path_index": -1, "kind": "no_cars", "value": 0, "extra": 0}
        ]
    });
    std::fs::write(path_for(&sco), serde_json::to_string(&text).unwrap()).unwrap();
    let back = CrossingConfig::load(&sco, 2).unwrap();
    assert_eq!(back.signals.len(), 2, "a signal with no set, and one that is not a signal, are skipped");
    assert_eq!(back.rules, vec![rule(1, "bus")], "a rule beyond the paths, a kind with a space, and a negative path are skipped");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_non_finite_place_is_refused_before_it_is_written() {
    let (dir, sco) = scratch("non-finite");
    let bad = SignalPoint { light: 0, set: String::new(), x: f32::INFINITY, up: 0.0, fwd: 0.0, heading: 0.0 };
    assert!(prepare(&sco, &CrossingConfig::of(None, vec![bad], Vec::new()), 2).is_err());
    let bad_rule = TemplateRule { path_index: 0, kind: "bus".into(), value: f64::NAN, extra: 0.0 };
    assert!(prepare(&sco, &CrossingConfig::of(None, Vec::new(), vec![bad_rule]), 2).is_err());
    assert!(!path_for(&sco).exists(), "nothing was written");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nothing_to_say_makes_no_file_and_removes_one_that_says_it() {
    let (dir, sco) = scratch("empty");
    let path = path_for(&sco);
    write(&sco, &CrossingConfig::default(), 2).unwrap();
    assert!(!path.exists(), "no file is made for nothing");

    write(&sco, &CrossingConfig::of(None, vec![point(0, 0.0)], Vec::new()), 2).unwrap();
    assert!(path.exists());
    let prepared = prepare(&sco, &CrossingConfig::default(), 2).unwrap();
    assert!(prepared.writes() && prepared.remove && prepared.bytes.is_none());
    write(&sco, &CrossingConfig::default(), 2).unwrap();
    assert!(!path.exists(), "an empty file is taken away");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn keys_this_version_does_not_know_are_kept() {
    let (dir, sco) = scratch("unknown-keys");
    let text = serde_json::json!({
        "version": 1,
        "later": {"something": [1, 2, 3]},
        "signals": []
    });
    std::fs::write(path_for(&sco), serde_json::to_string(&text).unwrap()).unwrap();
    let mut config = CrossingConfig::load(&sco, 2).unwrap();
    config.signals = vec![point(0, 2.0)];
    write(&sco, &config, 2).unwrap();
    let back: serde_json::Value = serde_json::from_slice(&std::fs::read(path_for(&sco)).unwrap()).unwrap();
    assert_eq!(back["later"], text["later"], "a key this version does not read is kept as it was");
    assert_eq!(back["signals"].as_array().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_counts_a_panel_asks_for_are_the_entries_that_can_be_used() {
    let (dir, sco) = scratch("counts");
    assert_eq!(CrossingConfig::counts(&sco), (0, 0), "no file, nothing offered");
    write(&sco, &CrossingConfig::of(None, vec![point(0, 0.0), point(1, 0.0)], vec![rule(0, "bus")]), 2).unwrap();
    assert_eq!(CrossingConfig::counts(&sco), (2, 1));
    // (the cache follows the file when it changes)
    std::fs::remove_file(path_for(&sco)).unwrap();
    assert_eq!(CrossingConfig::counts(&sco), (0, 0));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn sign_positions_have_their_frame() {
    let p = SignalPoint { light: 0, set: String::new(), x: 1.0, up: 2.0, fwd: 3.0, heading: 0.0 };
    // (x, y forward, z up: the crossing editor's own order)
    assert_eq!(p.local(), DVec3::new(1.0, 3.0, 2.0));
}

#[test]
fn a_template_rule_becomes_a_map_rule_without_a_kill_flag() {
    let r = TemplateRule { path_index: 3, kind: "no_cars".into(), value: 1.5, extra: 2.0 };
    let m = r.to_map_rule();
    assert_eq!((m.path_index, m.kind.as_str(), m.value, m.extra, m.kill), (3, "no_cars", 1.5, 2.0, false));
    assert_eq!(TemplateRule::from_map_rule(&m), r);
}
