//! The maker's roadway picture row: the stock asphalt, and a warning instead of a guess.

use super::*;

fn draw(label: &str, textures: Vec<PathBuf>, roadway: Option<PathBuf>) -> (RoadSpec, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("om-road-tex-ui-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let roots = Roots { omsi: dir.join("omsi"), content: dir.join("content") };
    let mut spec = RoadSpec::default();
    if let Some(path) = roadway {
        spec.roadway_tex = path;
    }
    let mut list = Some(textures);
    let mut search: [String; 4] = Default::default();
    let mut pics = TexCache::default();
    let mut canvas = false;
    let ctx = egui::Context::default();
    ctx.set_fonts(icons::fonts());
    let screen = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 3000.0));
    let output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |root| {
        egui::CentralPanel::default().show(root, |ui| {
            RoadTypes::maker_ui(ui, &mut spec, &mut list, &mut search, &roots, &mut pics, &mut canvas, false, None);
        });
    });
    let text = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
            _ => None,
        })
        .collect();
    std::fs::remove_dir_all(dir).unwrap();
    (spec, text)
}

fn picture(path: &Path) {
    image::RgbaImage::from_pixel(4, 4, image::Rgba([70, 70, 70, 255])).save(path).unwrap();
}

#[test]
fn the_maker_fills_an_empty_roadway_with_the_stock_asphalt_and_warns_without_it() {
    let dir = std::env::temp_dir().join(format!("om-road-tex-ui-pictures-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let stock = dir.join("STR_AsphDrk.BMP");
    let two_line = dir.join("str_asphdrk_2line.bmp");
    let own = dir.join("own asphalt.bmp");
    for path in [&stock, &two_line, &own] {
        picture(path);
    }
    let list = vec![two_line.clone(), stock.clone()];

    let (picked, texts) = draw("found", list.clone(), None);
    assert_eq!(picked.roadway_tex, stock, "the exact name wins over a similar one, in any case");
    assert!(!texts.iter().any(|text| text.contains("was not found")));

    let (missing, texts) = draw("missing", vec![two_line], None);
    assert!(missing.roadway_tex.as_os_str().is_empty(), "no picture is made up");
    assert!(
        texts.iter().any(|text| text.contains("The default roadway texture str_asphdrk.bmp was not found")),
        "the user must be told to choose by hand: {texts:?}"
    );

    let (kept, _) = draw("kept", list, Some(own.clone()));
    assert_eq!(kept.roadway_tex, own, "a chosen picture is never replaced");
    std::fs::remove_dir_all(dir).unwrap();
}
