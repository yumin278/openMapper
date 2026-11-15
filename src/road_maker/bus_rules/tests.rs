use super::*;
use crate::selection::tests::{add_spline, flat_map};

fn rule(path_index: i32, kind: &str) -> omsi_map::MapRule {
    omsi_map::MapRule {
        path_index,
        kind: kind.into(),
        ..Default::default()
    }
}

#[test]
fn application_is_idempotent_and_preserves_indices_and_other_rules() {
    let mut doc = flat_map(1);
    add_spline(&mut doc, (0, 0), 1, 0.0, 0.0, 0.0, 80.0, 0, 0);
    let mut killed = rule(1, "no_cars");
    killed.kill = true;
    doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].rules =
        vec![rule(1, "bus"), killed.clone(), rule(9, "speedlimit")];
    apply_bus_lane_rules(&mut doc, &[((0, 0), 1)], &[1, 3]);
    let first = doc.spline((0, 0), 1).unwrap().rules.clone();
    apply_bus_lane_rules(&mut doc, &[((0, 0), 1)], &[1, 3]);
    assert_eq!(doc.spline((0, 0), 1).unwrap().rules, first);
    assert_eq!(first.len(), 6);
    assert_eq!(first[1], killed);
    assert_eq!(first[2], rule(9, "speedlimit"));
    for index in [1, 3] {
        for kind in ["bus", "no_cars"] {
            assert_eq!(
                first
                    .iter()
                    .filter(|r| r.path_index == index && r.kind == kind && !r.kill)
                    .count(),
                1
            );
        }
    }
}

#[test]
fn cleanup_removes_only_complete_live_pairs_on_declared_paths() {
    let mut doc = flat_map(1);
    add_spline(&mut doc, (0, 0), 1, 0.0, 0.0, 0.0, 80.0, 0, 0);
    let speed = omsi_map::MapRule {
        value: 30.0,
        ..rule(1, "speedlimit")
    };
    let untouched = vec![
        speed.clone(),
        rule(2, "bus"),
        rule(3, "no_cars"),
        rule(8, "bus"),
        rule(8, "no_cars"),
    ];
    let mut rules = vec![rule(1, "bus"), rule(1, "no_cars")];
    rules.extend(untouched.clone());
    doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].rules = rules;
    remove_old(&mut doc, (0, 0), 1, &[1, 2, 3], "test::cleanup");
    assert_eq!(doc.spline((0, 0), 1).unwrap().rules, untouched);
}

#[test]
fn reserved_names_are_rejected_before_any_output_is_created() {
    for name in ["Main_bay_road", "Main_BaY_road", "Main bay road"] {
        let root = std::env::temp_dir().join(format!("om-reserved-name-{}", std::process::id()));
        let spec = RoadSpec {
            name: name.into(),
            ..Default::default()
        };
        assert_eq!(
            spec.validate().unwrap_err(),
            crate::i18n::tr("Names must not contain _bay_ (reserved for the bus bay tool)", "Имя не должно содержать _bay_ (зарезервировано для инструмента автобусных карманов)")
        );
        assert!(spec.write(&root).is_err());
        assert!(!root.exists());
    }
    assert!(
        RoadSpec {
            name: "BusBay Road".into(),
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn retyping_without_old_metadata_preserves_all_existing_rules() {
    let mut doc = flat_map(1);
    add_spline(&mut doc, (0, 0), 1, 0.0, 0.0, 0.0, 80.0, 0, 0);
    let original = vec![rule(1, "bus"), rule(1, "no_cars"), rule(1, "speedlimit")];
    doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].rules = original.clone();
    retype(
        &mut doc,
        &[((0, 0), 1)],
        "Splines\\missing_new.sli",
        "test::missing_metadata",
    );
    assert_eq!(doc.spline((0, 0), 1).unwrap().rules, original);
    assert_eq!(
        doc.spline((0, 0), 1).unwrap().file,
        "Splines\\missing_new.sli"
    );
}
