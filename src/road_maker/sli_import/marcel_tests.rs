//! Regression tests on two real OMSI 2 stock splines.
//!
//! `tests/fixtures/sli-marcel/` holds the keyword blocks of
//! `Splines\Marcel\Hstr_6spur_Wilhelm2.sli` and
//! `Splines\Marcel\str_4spur_zeppelin1.sli` from the OMSI 2 installation; see the
//! `PROVENANCE.md` beside them. The import must not merely accept these files: every
//! AI path has to come out again at the x the stock spline gives it, and the pavement
//! has to stay where the stock spline puts it. The strip layout is only the means.
//!
//! Both files are unsupported by the original [`super::infer`] in different ways:
//!
//! * Wilhelm2 lays one continuous carriageway over two asphalt textures (0.000–2.900
//!   with `str_asphdrk_1line`, 2.900–5.900 with `str_asphdrk_2line`, 5.900–8.900 with
//!   `str_asphdrk_1line`), so no single texture's level profile spans every path.
//! * Zeppelin1 has a 3.500 m raised median (x = -1.750 … 1.750) between two level
//!   height profiles, so no single level height profile spans every path.

use std::path::Path;

use omsi_cfg::CfgFile;
use omsi_scenery::Spline;

use super::create;
use crate::install::Roots;
use crate::road_maker::{Direction, MakerTarget, RoadSpec, Strip, ROAD_H};

/// A stock spline and the geometry its import must reproduce.
struct Stock {
    /// File name without `.sli`; also the imported road's name.
    name: &'static str,
    /// Relative path in the installation.
    rel: &'static str,
    /// The exact keyword blocks copied from the installed file.
    text: &'static str,
    /// Whether the map drives on the left. Both files are German stock content.
    map_left_hand: bool,
    /// The `RoadSpec::source_x_origin` the roadway's centre has to become.
    source_x_origin: f32,
    /// Union of the source's roadway-level (z = 0.10) height profiles.
    pavement: (f32, f32),
    /// `(x, [path] direction)` of every AI street path, in file order.
    lanes: &'static [(f32, i32)],
    /// `(direction, width)` of every lane strip, from left to right.
    strips: &'static [(Direction, f32)],
}

const WILHELM2: Stock = Stock {
    name: "Hstr_6spur_Wilhelm2",
    rel: "Splines/Marcel/Hstr_6spur_Wilhelm2.sli",
    text: include_str!("../../../tests/fixtures/sli-marcel/Hstr_6spur_Wilhelm2.sli"),
    map_left_hand: false,
    source_x_origin: 4.45,
    pavement: (0.0, 8.9),
    lanes: &[(1.450, 0), (4.400, 0), (7.400, 0)],
    strips: &[
        (Direction::Fwd, 2.9),
        (Direction::Fwd, 3.0),
        (Direction::Fwd, 3.0),
    ],
};

const ZEPPELIN1: Stock = Stock {
    name: "str_4spur_zeppelin1",
    rel: "Splines/Marcel/str_4spur_zeppelin1.sli",
    text: include_str!("../../../tests/fixtures/sli-marcel/str_4spur_zeppelin1.sli"),
    map_left_hand: false,
    source_x_origin: 0.0,
    pavement: (-9.75, 9.75),
    lanes: &[(-6.25, 1), (-3.25, 1), (3.25, 0), (6.25, 0)],
    strips: &[
        (Direction::Back, 3.0),
        (Direction::Back, 3.0),
        (Direction::Fwd, 3.0),
        (Direction::Fwd, 3.0),
    ],
};

/// Put the fixture where a spline lives and install empty texture files beside it.
fn install(
    stock: &Stock,
    tag: &str,
    textures: &[&str],
) -> (Roots, std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "om-sli-marcel-{}-{}-{tag}",
        std::process::id(),
        stock.name
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let source = dir.join(stock.rel);
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, stock.text).unwrap();
    let texture_dir = source.parent().unwrap().join("texture");
    std::fs::create_dir_all(&texture_dir).unwrap();
    for texture in textures {
        std::fs::write(texture_dir.join(texture), b"").unwrap();
    }
    let roots = Roots {
        omsi: dir.clone(),
        content: dir.clone(),
    };
    (roots, source, dir)
}

/// The maker's future SLI path for a JSON-only imported draft.
fn draft_sli(json: &Path) -> std::path::PathBuf {
    let name = json.file_name().unwrap().to_str().unwrap();
    let stem = name.strip_suffix(".road.json").unwrap();
    json.with_file_name(format!("{stem}.sli"))
}

/// Everything an implementer needs when a regression fails.
fn report(stock: &Stock, source: &Spline, spec: &RoadSpec, json: &Path) {
    println!("== {} ==", stock.name);
    println!("source SLI keyword blocks:\n{}", stock.text);
    println!(
        "textures: {:?}",
        source
            .textures
            .iter()
            .map(|t| t.file.as_str())
            .collect::<Vec<_>>()
    );
    println!(
        "source [path]s: {:?}",
        source
            .paths
            .iter()
            .map(|p| (p.kind, p.start[0], p.start[2], p.width, p.direction))
            .collect::<Vec<_>>()
    );
    println!(
        "source [heightprofile]s: {:?}",
        source
            .height_profiles
            .iter()
            .map(|h| (h.x0, h.x1, h.z0, h.z1))
            .collect::<Vec<_>>()
    );
    println!("inferred strips: {:#?}", spec.strips);
    println!(
        "source_x_origin: {}, x_offset: {}",
        spec.source_x_origin, spec.x_offset
    );
    let positions = spec.strip_positions();
    for (i, (strip, (lo, hi))) in spec.strips.iter().zip(&positions).enumerate() {
        println!(
            "  strip {i}: [{lo:.3}, {hi:.3}] width {:.3} height {:.3} {strip:?}",
            strip.width(),
            strip.height(spec.kerb)
        );
    }
    println!(
        "JSON:\n{}",
        std::fs::read_to_string(json).unwrap_or_default()
    );
    println!("emitted SLI:\n{}", spec.sli(stock.map_left_hand));
}

/// The `.sli` the maker would write for `spec`, parsed the way the game parses it.
fn emitted(stock: &Stock, spec: &RoadSpec) -> Spline {
    let text = spec.sli(stock.map_left_hand);
    Spline::parse(&CfgFile::from_str(Path::new("emitted.sli"), &text))
}

fn check_source_after_save_as(
    stock: &Stock,
    roots: &Roots,
    path: &Path,
    json: &Path,
    spec: &RoadSpec,
) {
    let before = std::fs::read(path).unwrap();
    let sidecar_before = std::fs::read(json).unwrap();
    println!("{}: source bytes after import identical: {}", stock.name, before == stock.text.as_bytes());
    assert_eq!(before, stock.text.as_bytes());
    let draft = draft_sli(json);
    assert_eq!(draft, crate::doc::safe::own_path(&roots.content, &spec.maker_filename(None)));
    assert!(!draft.exists(), "import must not create the generated SLI");
    assert!(!path.with_extension("road.json").exists(), "import must not create a source sidecar");
    let target = MakerTarget::open(spec.maker_filename(None), draft.clone()).unwrap();
    let mut copy = spec.clone();
    copy.name = format!("{} copy", stock.name);
    let rel = copy.write_in_maker_as_new(&roots.content, stock.map_left_hand, &target).unwrap();
    let saved = crate::doc::safe::own_path(&roots.content, &rel);
    assert_ne!(saved, path);
    check_paths(stock, &Spline::load(path).unwrap(), &Spline::load(&saved).unwrap());
    let after = std::fs::read(path).unwrap();
    let sidecar_after = std::fs::read(json).unwrap();
    println!(
        "{}: source bytes after Save As identical: {}; source sidecar identical: {}",
        stock.name, before == after, sidecar_before == sidecar_after
    );
    assert_eq!(before, after);
    assert_eq!(sidecar_before, sidecar_after);
    assert!(!draft.exists(), "Save As must not create the original draft's SLI");
}

/// The fixture is what the test thinks it is, and the import reproduces every path.
fn check_paths(stock: &Stock, source: &Spline, out: &Spline) {
    let want: Vec<(f32, i32)> = source
        .paths
        .iter()
        .filter(|p| p.kind == 0)
        .map(|p| (p.start[0], p.direction))
        .collect();
    assert_eq!(
        want.len(),
        stock.lanes.len(),
        "{}: fixture drifted",
        stock.name
    );
    for (i, (got, expected)) in want.iter().zip(stock.lanes).enumerate() {
        assert!(
            (got.0 - expected.0).abs() <= 0.0005,
            "{}: fixture path {i} is x={} (expected {})",
            stock.name,
            got.0,
            expected.0
        );
        assert_eq!(
            got.1, expected.1,
            "{}: fixture path {i} direction",
            stock.name
        );
    }

    let got: Vec<(f32, i32)> = out
        .paths
        .iter()
        .filter(|p| p.kind == 0)
        .map(|p| (p.start[0], p.direction))
        .collect();
    println!(
        "{} vehicle Path X/direction: {want:?} → {got:?}",
        stock.name
    );
    assert!(out.paths.iter().all(|p| p.kind == 0));
    assert_eq!(
        got.len(),
        want.len(),
        "{}: the import must write one AI path per travelled lane, got {got:?}",
        stock.name
    );
    for (i, (got, want)) in got.iter().zip(&want).enumerate() {
        assert!(
            (got.0 - want.0).abs() <= 0.0005,
            "{}: lane {i} path moved from x={:.3} to x={:.3}",
            stock.name,
            want.0,
            got.0
        );
        assert_eq!(
            got.1, want.1,
            "{}: lane {i} drives the wrong way",
            stock.name
        );
    }
}

/// The roadway stays where the stock spline has it, without relying on the strips the
/// import chose outside it (grass, footways) which it may drop.
fn check_pavement(stock: &Stock, out: &Spline) {
    let mut spans: Vec<(f32, f32)> = out
        .height_profiles
        .iter()
        .filter(|h| (h.z0 - ROAD_H).abs() <= 1e-3 && (h.z1 - ROAD_H).abs() <= 1e-3)
        .map(|h| (h.x0.min(h.x1), h.x0.max(h.x1)))
        .collect();
    assert!(
        !spans.is_empty(),
        "{}: no roadway-level height profile",
        stock.name
    );
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let lo = spans.first().unwrap().0;
    let hi = spans.last().unwrap().1;
    assert!(
        (lo - stock.pavement.0).abs() <= 0.001,
        "{}: pavement starts at x={lo:.3}, the stock spline at {:.3}",
        stock.name,
        stock.pavement.0
    );
    assert!(
        (hi - stock.pavement.1).abs() <= 0.001,
        "{}: pavement ends at x={hi:.3}, the stock spline at {:.3}",
        stock.name,
        stock.pavement.1
    );
}

/// Lane order, directions and widths, and no lane the stock file does not have.
fn check_strips(stock: &Stock, spec: &RoadSpec) {
    let lanes: Vec<(Direction, f32)> = spec
        .strips
        .iter()
        .filter_map(|s| match s {
            Strip::Lane { dir, width, .. } => Some((*dir, *width)),
            _ => None,
        })
        .collect();
    assert_eq!(
        lanes.len(),
        stock.strips.len(),
        "{}: lane strips {lanes:?}",
        stock.name
    );
    for (i, ((dir, width), (want_dir, want_width))) in lanes.iter().zip(stock.strips).enumerate() {
        assert_eq!(*dir, *want_dir, "{}: lane {i} direction", stock.name);
        assert!(
            (width - want_width).abs() <= 0.001,
            "{}: lane {i} width {width:.3}, the stock spline {want_width:.3}",
            stock.name
        );
    }
    assert!(
        !spec
            .strips
            .iter()
            .any(|s| matches!(s, Strip::BusBay { .. })),
        "{}: neither stock spline has a bus bay",
        stock.name
    );
}

#[test]
fn wilhelm2_import_keeps_every_lane_path_where_the_stock_spline_put_it() {
    let stock = &WILHELM2;
    // The maker's stock asphalt is the one texture an imported road needs; the
    // spline's own asphalt names (str_asphdrk_1line/_2line here) are not consulted.
    let (roots, source, dir) = install(stock, "lanes", &["str_asphdrk.bmp"]);
    let json = create(
        &roots,
        stock.rel,
        stock.name.to_string(),
        stock.map_left_hand,
    )
    .unwrap_or_else(|error| panic!("{}: stock spline must import: {error:#}", stock.name));
    let spec = RoadSpec::of_with_traffic(&draft_sli(&json), stock.map_left_hand)
        .unwrap_or_else(|| panic!("{}: the sidecar must reopen", stock.name));
    let loaded = Spline::load(&source).unwrap();
    report(stock, &loaded, &spec, &json);

    assert_eq!(
        std::fs::read(&source).unwrap(),
        stock.text.as_bytes(),
        "{}: the source must not be rewritten",
        stock.name
    );
    assert!(
        json.is_file(),
        "{}: the sidecar must be written",
        stock.name
    );
    assert!(
        spec.imported_sli && spec.advanced,
        "{}: import flags",
        stock.name
    );
    assert_eq!(
        spec.left_hand,
        Some(stock.map_left_hand),
        "{}: traffic side",
        stock.name
    );
    assert!(
        (spec.source_x_origin - stock.source_x_origin).abs() <= 0.001,
        "{}: source_x_origin {} puts the roadway at the wrong place (expected {})",
        stock.name,
        spec.source_x_origin,
        stock.source_x_origin
    );

    let out = emitted(stock, &spec);
    check_paths(stock, &loaded, &out);
    check_pavement(stock, &out);
    check_strips(stock, &spec);
    check_source_after_save_as(stock, &roots, &source, &json, &spec);
    assert!(
        (spec.roadway_width() - (stock.pavement.1 - stock.pavement.0)).abs() <= 0.001,
        "{}: the imported roadway is {:.3} m wide, the pavement {:.3} m",
        stock.name,
        spec.roadway_width(),
        stock.pavement.1 - stock.pavement.0
    );

    // A second import may not overwrite the sidecar the first one wrote.
    assert!(
        create(
            &roots,
            stock.rel,
            stock.name.to_string(),
            stock.map_left_hand
        )
        .is_err(),
        "{}: an existing sidecar must not be overwritten",
        stock.name
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn zeppelin1_import_keeps_the_raised_median_and_every_lane_path() {
    let stock = &ZEPPELIN1;
    let (roots, source, dir) = install(stock, "lanes", &["str_asphdrk.bmp"]);
    let json = create(
        &roots,
        stock.rel,
        stock.name.to_string(),
        stock.map_left_hand,
    )
    .unwrap_or_else(|error| panic!("{}: stock spline must import: {error:#}", stock.name));
    let spec = RoadSpec::of_with_traffic(&draft_sli(&json), stock.map_left_hand)
        .unwrap_or_else(|| panic!("{}: the sidecar must reopen", stock.name));
    let loaded = Spline::load(&source).unwrap();
    report(stock, &loaded, &spec, &json);

    assert_eq!(
        std::fs::read(&source).unwrap(),
        stock.text.as_bytes(),
        "{}: the source must not be rewritten",
        stock.name
    );
    assert!(
        spec.imported_sli && spec.advanced,
        "{}: import flags",
        stock.name
    );
    assert_eq!(
        spec.left_hand,
        Some(stock.map_left_hand),
        "{}: traffic side",
        stock.name
    );
    assert!(
        (spec.source_x_origin - stock.source_x_origin).abs() <= 0.001,
        "{}: source_x_origin {} (expected {})",
        stock.name,
        spec.source_x_origin,
        stock.source_x_origin
    );

    let out = emitted(stock, &spec);
    check_paths(stock, &loaded, &out);
    check_pavement(stock, &out);
    check_strips(stock, &spec);
    check_source_after_save_as(stock, &roots, &source, &json, &spec);

    // The 3.500 m median is the reason the inner paths sit at ±3.250 and not at ±2.125.
    let raised: Vec<(f32, f32, f32)> = out
        .height_profiles
        .iter()
        .filter(|h| h.z0 > ROAD_H + 0.01)
        .map(|h| (h.x0.min(h.x1), h.x0.max(h.x1), h.z0))
        .collect();
    assert!(
        raised.iter().any(|&(lo, hi, z)| (lo + 1.75).abs() <= 0.001
            && (hi - 1.75).abs() <= 0.001
            && (z - 0.25).abs() <= 0.001),
        "{}: the raised median is not at x = [-1.750, 1.750] 0.25 m high: {raised:?}",
        stock.name
    );

    // A second import may not overwrite the sidecar the first one wrote.
    assert!(
        create(
            &roots,
            stock.rel,
            stock.name.to_string(),
            stock.map_left_hand
        )
        .is_err(),
        "{}: an existing sidecar must not be overwritten",
        stock.name
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// The import uses the same unmarked asphalt as a newly created road, without
/// relying on the source's texture list.
#[test]
fn zeppelin1_finds_its_plain_asphalt_texture_by_name() {
    let stock = &ZEPPELIN1;
    let (roots, source, dir) = install(stock, "lookup", &["str_asphdrk.bmp"]);
    let json = create(
        &roots,
        stock.rel,
        stock.name.to_string(),
        stock.map_left_hand,
    )
    .unwrap_or_else(|error| {
        panic!(
            "{}: the only installed texture is str_asphdrk.bmp, but the import failed: {error:#}",
            stock.name
        )
    });
    let spec = RoadSpec::of_with_traffic(&draft_sli(&json), stock.map_left_hand)
        .unwrap_or_else(|| panic!("{}: the sidecar must reopen", stock.name));
    let found = spec
        .roadway_tex
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    println!("{}: roadway texture resolved to {found:?}", stock.name);
    assert_eq!(
        found, "str_asphdrk.bmp",
        "{}: the import picked {found:?} although only str_asphdrk.bmp exists",
        stock.name
    );
    assert!(json.is_file());
    assert_eq!(json, roots.content.join("Splines").join("openMapper").join(format!("{}.road.json", stock.name)));
    assert!(!source.with_extension("road.json").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

/// A Hong Kong street whose visible asphalt is wider than its flat height profile.
///
/// The roadway is drawn from x = -5.500 to x = +5.500 at 0.100 m, while the height
/// profile only says 0.100 m between -4.000 and +4.000: the outer 1.500 m of asphalt
/// lies under a 0.250 m kerb. The import must keep the visible 11.000 m of pavement
/// and the source's own height profile, not crop the pavement at the height profile.
const ALTONAER: Stock = Stock {
    name: "2spur_11m_altonaer_d",
    rel: "Splines/3dtranstudio/hkstreet/2spur_11m_altonaer_d.sli",
    text: include_str!("../../../tests/fixtures/sli-marcel/2spur_11m_altonaer_d.sli"),
    map_left_hand: true,
    source_x_origin: 0.0,
    pavement: (-5.5, 5.5),
    lanes: &[(-2.0, 0), (2.0, 1)],
    strips: &[(Direction::Fwd, 3.25), (Direction::Back, 3.25)],
};

/// The asphalt the game draws: the render profile at the roadway's height.
fn visible_asphalt(spline: &Spline) -> (f32, f32) {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for profile in &spline.profiles {
        for pair in profile.points.windows(2) {
            if (pair[0].z - ROAD_H).abs() <= 1e-3 && (pair[1].z - ROAD_H).abs() <= 1e-3 {
                lo = lo.min(pair[0].x).min(pair[1].x);
                hi = hi.max(pair[0].x).max(pair[1].x);
            }
        }
    }
    (lo, hi)
}

/// The collision heights, as `(x0, x1, z)`.
fn height_bands(spline: &Spline) -> Vec<(f32, f32, f32)> {
    spline
        .height_profiles
        .iter()
        .map(|h| (h.x0.min(h.x1), h.x0.max(h.x1), h.z0))
        .collect()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 0.001
}

/// Every render-profile segment the game draws at the roadway's height, left to right.
fn paved_segments(spline: &Spline) -> Vec<(f32, f32)> {
    let mut segments: Vec<(f32, f32)> = spline
        .profiles
        .iter()
        .flat_map(|profile| profile.points.windows(2))
        .filter(|pair| (pair[0].z - ROAD_H).abs() <= 1e-3 && (pair[1].z - ROAD_H).abs() <= 1e-3)
        .map(|pair| (pair[0].x.min(pair[1].x), pair[0].x.max(pair[1].x)))
        .filter(|(lo, hi)| hi - lo > 1e-3)
        .collect();
    segments.sort_by(|a, b| a.0.total_cmp(&b.0));
    segments
}

/// The pavement is one continuous surface: each paved segment meets the next, so neither a
/// gap nor an unpaved stretch is left between two strips, and the paved run covers exactly
/// the width the stock spline draws.
fn check_continuous_paving(stock: &Stock, out: &Spline) {
    let segments = paved_segments(out);
    assert!(
        !segments.is_empty(),
        "{}: the import draws no pavement at the roadway's height",
        stock.name
    );
    println!(
        "{} paved segments at z={ROAD_H:.3}: {segments:?}",
        stock.name
    );
    for pair in segments.windows(2) {
        assert!(
            pair[1].0 <= pair[0].1 + 1e-3,
            "{}: paved segment {:.3}..{:.3} leaves a gap before {:.3}..{:.3}",
            stock.name,
            pair[0].0,
            pair[0].1,
            pair[1].0,
            pair[1].1
        );
    }
    let lo = segments.first().unwrap().0;
    let hi = segments.last().unwrap().1;
    assert!(
        close(lo, stock.pavement.0) && close(hi, stock.pavement.1),
        "{}: the paved run is {lo:.3}..{hi:.3}, the stock spline {:.3}..{:.3}",
        stock.name,
        stock.pavement.0,
        stock.pavement.1
    );
}

/// Every AI street path, source and imported, runs at the roadway's height.
fn check_vehicle_path_z(stock: &Stock, source: &Spline, out: &Spline) {
    for (which, spline) in [("source", source), ("imported", out)] {
        let heights: Vec<f32> = spline
            .paths
            .iter()
            .filter(|p| p.kind == 0)
            .map(|p| p.start[2])
            .collect();
        assert_eq!(
            heights.len(),
            stock.lanes.len(),
            "{}: {which} has {} vehicle paths",
            stock.name,
            heights.len()
        );
        for (i, z) in heights.iter().enumerate() {
            assert!(
                close(*z, ROAD_H),
                "{}: {which} vehicle path {i} sits at z={z:.3}, the roadway at {ROAD_H:.3}",
                stock.name
            );
        }
    }
}

#[test]
fn altonaer_import_keeps_the_visible_pavement_and_the_source_height_profile() {
    let stock = &ALTONAER;
    let (roots, source, dir) = install(stock, "altonaer", &["str_asphdrk.bmp"]);
    let loaded = Spline::parse(&CfgFile::from_str(Path::new("altonaer.sli"), stock.text));

    println!("== {} ==", stock.name);
    println!("source SLI keyword blocks:\n{}", stock.text);
    println!("source [heightprofile]s: {:?}", height_bands(&loaded));
    println!("source visible asphalt: {:?}", visible_asphalt(&loaded));
    println!(
        "source [path]s: {:?}",
        loaded
            .paths
            .iter()
            .map(|p| (p.kind, p.start[0], p.start[2], p.width, p.direction))
            .collect::<Vec<_>>()
    );
    assert!(
        close(visible_asphalt(&loaded).0, -5.5) && close(visible_asphalt(&loaded).1, 5.5),
        "the fixture's visible asphalt moved: {:?}",
        visible_asphalt(&loaded)
    );
    assert_eq!(
        height_bands(&loaded),
        vec![(-7.0, -4.0, 0.25), (-4.0, 4.0, 0.1), (4.0, 7.0, 0.25)],
        "the fixture's height profile drifted"
    );

    let json = create(&roots, stock.rel, stock.name.to_string(), stock.map_left_hand)
        .unwrap_or_else(|error| panic!("{}: the stock spline must import: {error:#}", stock.name));
    let spec = RoadSpec::of_with_traffic(&draft_sli(&json), stock.map_left_hand)
        .unwrap_or_else(|| panic!("{}: the sidecar must reopen", stock.name));

    println!("inferred strips: {:#?}", spec.strips);
    println!(
        "source_x_origin: {}, x_offset: {}",
        spec.source_x_origin, spec.x_offset
    );
    let positions = spec.strip_positions();
    for (i, (strip, (lo, hi))) in spec.strips.iter().zip(&positions).enumerate() {
        println!(
            "  strip {i}: [{lo:.3}, {hi:.3}] width {:.3} height {:.3} {strip:?}",
            strip.width(),
            strip.height(spec.kerb)
        );
    }
    println!("JSON:\n{}", std::fs::read_to_string(&json).unwrap());
    println!("emitted SLI:\n{}", spec.sli(stock.map_left_hand));

    let out = emitted(stock, &spec);
    println!("emitted [heightprofile]s: {:?}", height_bands(&out));
    println!("emitted visible asphalt: {:?}", visible_asphalt(&out));

    // The pavement the game draws is the whole 11.000 m, not the 8.000 m that the
    // flat height profile alone would give.
    let (lo, hi) = visible_asphalt(&out);
    assert!(
        close(lo, -5.5) && close(hi, 5.5),
        "the imported pavement is {lo:.3}..{hi:.3}, the stock spline -5.500..5.500"
    );
    assert!(
        close(hi - lo, 11.0),
        "the imported asphalt is {:.3} m wide, the stock spline 11.000 m",
        hi - lo
    );
    assert!(
        close(spec.roadway_width(), 11.0),
        "the imported roadway is {:.3} m wide, the stock spline 11.000 m",
        spec.roadway_width()
    );

    // The source's own height profile is kept: the outer 1.500 m of asphalt still
    // lies under the 0.250 m kerb.
    assert_eq!(
        height_bands(&out),
        vec![(-5.5, -4.0, 0.25), (-4.0, 4.0, 0.1), (4.0, 5.5, 0.25)],
        "the height profile must not be flattened to one level"
    );

    check_paths(stock, &loaded, &out);
    check_continuous_paving(stock, &out);
    check_vehicle_path_z(stock, &loaded, &out);
    check_strips(stock, &spec);
    assert!(
        close(spec.source_x_origin, 0.0),
        "the source origin moved to {}",
        spec.source_x_origin
    );
    assert_eq!(
        std::fs::read(&source).unwrap(),
        stock.text.as_bytes(),
        "{}: the source must not be rewritten",
        stock.name
    );
    check_source_after_save_as(stock, &roots, &source, &json, &spec);

    // A second import may not overwrite the sidecar the first one wrote.
    assert!(
        create(
            &roots,
            stock.rel,
            stock.name.to_string(),
            stock.map_left_hand
        )
        .is_err(),
        "{}: an existing sidecar must not be overwritten",
        stock.name
    );
    std::fs::remove_dir_all(dir).unwrap();
}
