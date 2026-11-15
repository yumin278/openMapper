use super::*;
use crate::road_maker::{Direction, Strip};

pub(super) fn panel(ui: &mut egui::Ui, s: &mut RoadSpec, textures: &[PathBuf]) {
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr("Name", "Название"));
        ui.text_edit_singleline(&mut s.name);
        ui.label(crate::i18n::tr("Kerb height", "Высота бордюра"));
        ui.add(egui::Slider::new(&mut s.kerb, 0.0..=0.3).suffix(" m"));
    });
    ui.label(crate::i18n::tr(
        "Strips run from left to right",
        "Полосы расположены слева направо",
    ));
    let count = s.strips.len();
    let mut remove = None;
    let mut swap = None;
    for (i, strip) in s.strips.iter_mut().enumerate() {
        ui.push_id(("strip", i), |ui| {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{}", i + 1));
                    ui.strong(match strip {
                        Strip::Lane { .. } => crate::i18n::tr("Lane", "Полоса движения"),
                        Strip::Median { .. } => crate::i18n::tr("Median", "Разделительная полоса"),
                        Strip::Footway { .. } => crate::i18n::tr("Footway", "Тротуар"),
                        Strip::BusBay { .. } => crate::i18n::tr("BusBay", "Автобусный карман"),
                    });
                    if ui
                        .add_enabled(
                            i > 0,
                            egui::Button::new(crate::i18n::tr("Move up", "Вверх")),
                        )
                        .clicked()
                    {
                        swap = Some((i, i - 1));
                    }
                    if ui
                        .add_enabled(
                            i + 1 < count,
                            egui::Button::new(crate::i18n::tr("Move down", "Вниз")),
                        )
                        .clicked()
                    {
                        swap = Some((i, i + 1));
                    }
                    if ui.button(crate::i18n::tr("Delete", "Удалить")).clicked() {
                        remove = Some(i);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::tr("Width", "Ширина"));
                    match strip {
                        Strip::Lane {
                            dir,
                            width,
                            bus_only,
                        } => {
                            ui.add(egui::Slider::new(width, 2.5..=5.0).suffix(" m"));
                            ui.selectable_value(
                                dir,
                                Direction::Fwd,
                                crate::i18n::tr("Forward ↑", "По ходу ↑"),
                            );
                            ui.selectable_value(
                                dir,
                                Direction::Back,
                                crate::i18n::tr("Backward ↓", "Против хода ↓"),
                            );
                            ui.checkbox(bus_only, crate::i18n::tr("bus only", "только автобусы"));
                        }
                        Strip::Median { width, raised, .. } => {
                            ui.add(egui::Slider::new(width, 0.0..=12.0).suffix(" m"));
                            ui.checkbox(raised, crate::i18n::tr("raised", "приподнят"));
                        }
                        Strip::Footway { width, .. } => {
                            ui.add(egui::Slider::new(width, 0.1..=8.0).suffix(" m"));
                        }
                        Strip::BusBay { width } => {
                            ui.add(egui::Slider::new(width, 0.1..=4.0).suffix(" m"));
                        }
                    }
                });
                let is_footway = matches!(strip, Strip::Footway { .. });
                if let Strip::Median { tex, .. } | Strip::Footway { tex, .. } = strip {
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::tr("Texture", "Текстура"));
                        egui::ComboBox::from_id_salt("material")
                            .selected_text(if tex.as_os_str().is_empty() {
                                if is_footway {
                                    tr("Default footway texture", "Стандартная текстура тротуара")
                                        .into()
                                } else {
                                    crate::i18n::tr(
                                        "Use roadway texture",
                                        "Использовать текстуру дороги",
                                    )
                                    .into()
                                }
                            } else {
                                file_name(tex)
                            })
                            .show_ui(ui, |ui| {
                                if !is_footway
                                    && ui
                                        .selectable_label(
                                            tex.as_os_str().is_empty(),
                                            crate::i18n::tr(
                                                "Use roadway texture",
                                                "Использовать текстуру дороги",
                                            ),
                                        )
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
                        if ui
                            .button(crate::i18n::tr("Choose file…", "Выбрать файл…"))
                            .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter(
                                    crate::i18n::tr("Texture", "Текстура"),
                                    &["png", "jpg", "bmp", "tga", "dds"],
                                )
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
        ui.label(crate::i18n::tr("Add", "Добавить"));
        if ui
            .button(crate::i18n::tr("Lane", "Полоса движения"))
            .clicked()
        {
            s.strips.push(Strip::Lane {
                dir: Direction::Fwd,
                width: 3.5,
                bus_only: false,
            });
        }
        if ui
            .button(crate::i18n::tr("Median", "Разделительная полоса"))
            .clicked()
        {
            s.strips.push(Strip::Median {
                width: 2.0,
                raised: true,
                tex: s.median_tex.clone(),
            });
        }
        if ui.button(crate::i18n::tr("Footway", "Тротуар")).clicked() {
            s.strips.push(Strip::Footway {
                width: 2.0,
                tex: s.footway_tex.clone(),
            });
        }
        if ui
            .button(crate::i18n::tr("BusBay", "Автобусный карман"))
            .clicked()
        {
            s.strips.push(Strip::BusBay { width: 3.0 });
        }
    });
}
