use super::*;
use crate::road_maker::{Direction, Strip};

pub(super) fn panel(ui: &mut egui::Ui, s: &mut RoadSpec, textures: &[PathBuf]) {
    ui.horizontal(|ui| {
        ui.label("名稱");
        ui.text_edit_singleline(&mut s.name);
        ui.label("緣石高度");
        ui.add(egui::Slider::new(&mut s.kerb, 0.0..=0.3).suffix(" m"));
    });
    ui.label("積木依道路左至右排列");
    let count = s.strips.len();
    let mut remove = None;
    let mut swap = None;
    for (i, strip) in s.strips.iter_mut().enumerate() {
        ui.push_id(("strip", i), |ui| {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{}", i + 1));
                    ui.strong(match strip {
                        Strip::Lane { .. } => "Lane 車道",
                        Strip::Median { .. } => "Median 分隔島",
                        Strip::Footway { .. } => "Footway 人行道",
                        Strip::BusBay { .. } => "BusBay 巴士灣",
                    });
                    if ui.add_enabled(i > 0, egui::Button::new("上移")).clicked() {
                        swap = Some((i, i - 1));
                    }
                    if ui
                        .add_enabled(i + 1 < count, egui::Button::new("下移"))
                        .clicked()
                    {
                        swap = Some((i, i + 1));
                    }
                    if ui.button("刪除").clicked() {
                        remove = Some(i);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("寬度");
                    match strip {
                        Strip::Lane {
                            dir,
                            width,
                            bus_only,
                        } => {
                            ui.add(egui::Slider::new(width, 2.5..=5.0).suffix(" m"));
                            ui.selectable_value(dir, Direction::Fwd, "順向 ↑");
                            ui.selectable_value(dir, Direction::Back, "逆向 ↓");
                            ui.checkbox(bus_only, "公車專用");
                        }
                        Strip::Median { width, raised, .. } => {
                            ui.add(egui::Slider::new(width, 0.0..=12.0).suffix(" m"));
                            ui.checkbox(raised, "抬高");
                        }
                        Strip::Footway { width, .. } => {
                            ui.add(egui::Slider::new(width, 0.1..=8.0).suffix(" m"));
                        }
                        Strip::BusBay { width } => {
                            ui.add(egui::Slider::new(width, 0.1..=4.0).suffix(" m"));
                        }
                    }
                });
                if let Strip::Median { tex, .. } | Strip::Footway { tex, .. } = strip {
                    ui.horizontal(|ui| {
                        ui.label("材質");
                        egui::ComboBox::from_id_salt("material")
                            .selected_text(if tex.as_os_str().is_empty() {
                                "使用車道材質".into()
                            } else {
                                file_name(tex)
                            })
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(tex.as_os_str().is_empty(), "使用車道材質")
                                    .clicked()
                                {
                                    tex.clear();
                                }
                                for path in textures {
                                    if ui.selectable_label(tex == path, file_name(path)).clicked() {
                                        *tex = path.clone();
                                    }
                                }
                            });
                        if ui.button("選擇檔案…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("材質", &["png", "jpg", "bmp", "tga", "dds"])
                                .pick_file()
                            {
                                *tex = path;
                            }
                        }
                    });
                }
            });
        });
    }
    if let Some(i) = remove {
        s.strips.remove(i);
    } else if let Some((a, b)) = swap {
        s.strips.swap(a, b);
    }
    ui.horizontal(|ui| {
        ui.label("新增");
        if ui.button("Lane").clicked() {
            s.strips.push(Strip::Lane {
                dir: Direction::Fwd,
                width: 3.5,
                bus_only: false,
            });
        }
        if ui.button("Median").clicked() {
            s.strips.push(Strip::Median {
                width: 2.0,
                raised: true,
                tex: s.median_tex.clone(),
            });
        }
        if ui.button("Footway").clicked() {
            s.strips.push(Strip::Footway {
                width: 2.0,
                tex: s.footway_tex.clone(),
            });
        }
        if ui.button("BusBay").clicked() {
            s.strips.push(Strip::BusBay { width: 3.0 });
        }
    });
}
