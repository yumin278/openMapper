//! The stock roadway picture: found by its exact name (case-insensitive only), never
//! invented when it is missing, and never imposed on a road that already names one.

use super::*;
use std::path::{Path, PathBuf};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("om-road-tex-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, b"picture").unwrap();
}

#[test]
fn the_default_roadway_texture_is_matched_by_exact_name_ignoring_only_case() {
    let dir = temp("names");
    let two_line = dir.join("str_asphdrk_2line.bmp");
    let prefixed = dir.join("my_str_asphdrk.bmp");
    let exact = dir.join("STR_AsphDrk.BMP");
    for path in [&two_line, &prefixed, &exact] {
        touch(path);
    }
    let list = vec![two_line.clone(), prefixed.clone(), exact.clone()];
    assert_eq!(
        default_roadway_texture(&list),
        Some(exact),
        "only the exact name, in any case, is the stock asphalt"
    );
    assert_eq!(find_named_texture(&list, DEFAULT_ROADWAY_TEX), Some(dir.join("STR_AsphDrk.BMP")));
    // Similar names and another format are not the stock asphalt.
    assert_eq!(default_roadway_texture(&[two_line]), None);
    assert_eq!(default_roadway_texture(&[prefixed]), None);
    assert_eq!(default_roadway_texture(&[dir.join("str_asphdrk.tga")]), None);
    assert_eq!(default_roadway_texture(&[]), None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_missing_choice_is_filled_but_a_chosen_picture_is_kept() {
    let dir = temp("apply");
    let stock = dir.join("str_asphdrk.bmp");
    touch(&stock);
    let mut fresh = RoadSpec::default();
    assert!(fresh.use_default_roadway_texture(&[stock.clone()]));
    assert_eq!(fresh.roadway_tex, stock);

    let chosen = dir.join("own asphalt.bmp");
    touch(&chosen);
    fresh.roadway_tex = chosen.clone();
    assert!(fresh.use_default_roadway_texture(&[stock.clone()]));
    assert_eq!(fresh.roadway_tex, chosen, "an existing picture is never replaced");

    let mut empty = RoadSpec::default();
    assert!(!empty.use_default_roadway_texture(&[dir.join("str_asphdrk_2line.bmp")]));
    assert!(empty.roadway_tex.as_os_str().is_empty(), "no substitute is invented");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_unset_roadway_texture_has_no_stock_or_lined_name_to_fall_back_on() {
    let mut spec = RoadSpec::default();
    assert!(spec.roadway_tex_name().is_empty(), "a missing choice is not replaced by a stock bitmap");
    // The only lined name is the copy this road paints itself, when markings are on.
    spec.marked = true;
    assert_eq!(spec.roadway_tex_name(), "road_lines.bmp");
}

#[test]
fn every_preset_starts_on_the_stock_asphalt_when_it_is_offered() {
    let dir = temp("presets");
    let stock = dir.join("str_asphdrk.bmp");
    touch(&stock);
    let preset_list = presets_with_roadway_texture(Some(stock.as_path()));
    assert_eq!(preset_list.len(), presets().len());
    for (label, spec) in &preset_list {
        assert_eq!(spec.roadway_tex, stock, "preset {label} must start on the stock asphalt");
    }
    // Without a picture to offer, presets stay empty (the maker warns instead).
    for (_, spec) in presets() {
        assert!(spec.roadway_tex.as_os_str().is_empty());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_manually_chosen_roadway_texture_survives_the_write_and_the_reload() {
    let content = temp("manual-write");
    let texture_dir = content.join("Splines").join("openMapper").join("texture");
    std::fs::create_dir_all(&texture_dir).unwrap();
    let stock = texture_dir.join("str_asphdrk.bmp");
    let own = texture_dir.join("own asphalt.bmp");
    touch(&stock);
    touch(&own);
    let spec = RoadSpec { name: "Manual".into(), roadway_tex: own.clone(), ..Default::default() };
    let rel = spec.write(&content).unwrap();
    let path = content.join(rel.replace('\\', "/"));
    let sli = std::fs::read_to_string(&path).unwrap();
    assert!(sli.contains("own asphalt.bmp"), "the written road must name the chosen picture:\n{sli}");
    assert!(!sli.contains("str_asphdrk.bmp"), "the written road must not swap in the stock asphalt:\n{sli}");
    let mut back = RoadSpec::of(&path).expect("the written road must reopen");
    assert_eq!(back.roadway_tex, own, "the reload must keep the chosen picture, not the stock one");
    assert_ne!(back.roadway_tex, stock);
    // Nor does showing the road replace it by the stock asphalt that is also installed.
    assert!(back.use_default_roadway_texture(&[stock.clone()]));
    assert_eq!(back.roadway_tex, own);
    std::fs::remove_dir_all(content).unwrap();
}

#[test]
fn sidecars_that_name_a_roadway_texture_keep_it() {
    let dir = temp("json");
    let stock = dir.join("str_asphdrk.bmp");
    let chosen = dir.join("map asphalt.bmp");
    touch(&stock);
    touch(&chosen);
    let text = serde_json::json!({ "roadway_tex": chosen.to_str() }).to_string();
    let mut spec: RoadSpec = serde_json::from_str(&text).unwrap();
    assert!(spec.use_default_roadway_texture(&[stock.clone()]));
    assert_eq!(spec.roadway_tex, chosen);
    let back = serde_json::to_value(&spec).unwrap();
    assert_eq!(back["roadway_tex"].as_str(), chosen.to_str());

    // A sidecar written before the field was named still has no picture of its own.
    let mut old: RoadSpec = serde_json::from_str("{}").unwrap();
    assert!(old.roadway_tex.as_os_str().is_empty());
    assert!(old.use_default_roadway_texture(&[stock]));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_stock_asphalt_is_found_in_another_installed_spline_folder_or_the_root_texture_folder() {
    use crate::install::Roots;

    // A source spline elsewhere, with the stock asphalt only in another spline's folder.
    let dir = temp("shared-splines");
    let roots = Roots { omsi: dir.join("omsi"), content: dir.join("content") };
    let marcel = roots.content.join("Splines").join("Marcel").join("texture").join("STR_AsphDrk.BMP");
    touch(&marcel);
    touch(&roots.content.join("Splines").join("Marcel").join("texture").join("str_asphdrk_2line.bmp"));
    // Seasonal copies are not offered.
    touch(&roots.content.join("Splines").join("Marcel").join("texture").join("Winter").join("str_asphdrk.bmp"));
    let list = crate::assets::spline_textures(&roots);
    assert_eq!(default_roadway_texture(&list), Some(marcel));

    // Also the installation's own Texture folder.
    let dir = temp("shared-root-texture");
    let roots = Roots { omsi: dir.join("omsi"), content: dir.join("content") };
    let stock = roots.omsi.join("Texture").join("str_asphdrk.bmp");
    touch(&stock);
    touch(&roots.omsi.join("Texture").join("str_asphdrk_2line.bmp"));
    let list = crate::assets::spline_textures(&roots);
    assert_eq!(default_roadway_texture(&list), Some(stock));

    // The content folder's picture comes before the installation's.
    let dir = temp("shared-order");
    let roots = Roots { omsi: dir.join("omsi"), content: dir.join("content") };
    let own = roots.content.join("Splines").join("Own").join("texture").join("str_asphdrk.bmp");
    touch(&own);
    touch(&roots.omsi.join("Splines").join("Stock").join("texture").join("str_asphdrk.bmp"));
    let list = crate::assets::spline_textures(&roots);
    assert_eq!(default_roadway_texture(&list), Some(own));

    // Nothing but a similar name: still no substitute.
    let dir = temp("shared-variant");
    let roots = Roots { omsi: dir.join("omsi"), content: dir.join("content") };
    touch(&roots.omsi.join("Splines").join("Stock").join("texture").join("str_asphdrk_2line.bmp"));
    assert_eq!(default_roadway_texture(&crate::assets::spline_textures(&roots)), None);
    for dir in [
        std::env::temp_dir().join(format!("om-road-tex-{}-shared-splines", std::process::id())),
        std::env::temp_dir().join(format!("om-road-tex-{}-shared-root-texture", std::process::id())),
        std::env::temp_dir().join(format!("om-road-tex-{}-shared-order", std::process::id())),
        std::env::temp_dir().join(format!("om-road-tex-{}-shared-variant", std::process::id())),
    ] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn an_imported_road_takes_the_stock_asphalt_from_its_own_or_another_splines_folder() {
    use crate::install::Roots;
    let dir = temp("import-search");
    let roots = Roots { omsi: dir.join("omsi"), content: dir.join("content") };
    let source = roots.content.join("Splines").join("Map").join("my road.sli");
    touch(&source);
    // (the map's own copy, beside the source, wins)
    let own = roots.content.join("Splines").join("Map").join("texture").join("STR_AsphDrk.BMP");
    touch(&own);
    let marcel = roots.content.join("Splines").join("Marcel").join("texture").join("str_asphdrk.bmp");
    touch(&marcel);
    assert_eq!(super::sli_import::stock_roadway_texture(&source, &roots), Some(own.clone()));
    std::fs::remove_file(&own).unwrap();
    assert_eq!(super::sli_import::stock_roadway_texture(&source, &roots), Some(marcel.clone()));
    std::fs::remove_file(&marcel).unwrap();
    // A similar name is not the stock asphalt, and neither is a missing one.
    touch(&roots.content.join("Splines").join("Marcel").join("texture").join("str_asphdrk_2line.bmp"));
    assert_eq!(super::sli_import::stock_roadway_texture(&source, &roots), None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_default_is_found_in_the_texture_folders_ignoring_case_only() {
    let dir = temp("dirs");
    let installed = dir.join("Texture");
    touch(&installed.join("Str_AsphDrk.BMP"));
    touch(&installed.join("str_asphdrk_2line.bmp"));
    let map_own = dir.join("map").join("texture").join("str_asphdrk.bmp");
    touch(&map_own);
    let dirs = vec![map_own.parent().unwrap().to_path_buf(), installed.clone()];
    assert_eq!(
        find_named_texture_in_dirs(DEFAULT_ROADWAY_TEX, &dirs),
        Some(map_own),
        "the map's own picture comes first"
    );
    assert_eq!(
        find_named_texture_in_dirs(DEFAULT_ROADWAY_TEX, &dirs[1..]),
        Some(installed.join("Str_AsphDrk.BMP"))
    );

    let variant_only = temp("dirs-variant");
    touch(&variant_only.join("STR_ASPHDRK_2LINE.BMP"));
    assert_eq!(find_named_texture_in_dirs(DEFAULT_ROADWAY_TEX, &[variant_only.clone()]), None);
    assert_eq!(find_named_texture_in_dirs(DEFAULT_ROADWAY_TEX, &[dir.join("nowhere")]), None);
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(variant_only).unwrap();
}
