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
use crate::road_maker::{Direction, RoadSpec, Strip, ROAD_H};

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
    println!("{} vehicle Path X/direction: {want:?} → {got:?}", stock.name);
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
    let (roots, source, dir) = install(
        stock,
        "lanes",
        &[
            "str_gruenstreifen.bmp",
            "str_asphdrk_1line.bmp",
            "str_asphdrk_2line.bmp",
            "str_gehweg03.bmp",
        ],
    );
    let json = create(
        &roots,
        stock.rel,
        stock.name.to_string(),
        stock.map_left_hand,
    )
    .unwrap_or_else(|error| panic!("{}: stock spline must import: {error:#}", stock.name));
    let spec = RoadSpec::of_with_traffic(&source, stock.map_left_hand)
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
    let (roots, source, dir) = install(
        stock,
        "lanes",
        &[
            "str_asphdrk_1line.bmp",
            "str_asphdrk.bmp",
            "str_side1.bmp",
            "str_gruenstreifen.bmp",
        ],
    );
    let json = create(
        &roots,
        stock.rel,
        stock.name.to_string(),
        stock.map_left_hand,
    )
    .unwrap_or_else(|error| panic!("{}: stock spline must import: {error:#}", stock.name));
    let spec = RoadSpec::of_with_traffic(&source, stock.map_left_hand)
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

/// The import resolves the file names the spline lists. For Zeppelin1 the plain
/// asphalt (`str_asphdrk.bmp`, its texture 1) is all that has to be installed: the
/// stock file's texture folder is not part of these fixtures.
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
    let spec = RoadSpec::of_with_traffic(&source, stock.map_left_hand)
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
    std::fs::remove_dir_all(dir).unwrap();
}
