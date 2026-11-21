use super::*;

// Exercise real egui layout and input without a GPU or an OMSI installation.
#[test]
fn maker_scroll_keeps_actions_visible() {
    for size in [Vec2::new(1366.0, 768.0), Vec2::new(1920.0, 1080.0)] {
        for advanced in [false, true] {
            for bus_preview in [false, true] {
                eprintln!("case {size:?} advanced={advanced} preview={bus_preview}");
                let ctx = egui::Context::default();
                ctx.set_fonts(icons::fonts());
                let screen = egui::Rect::from_min_size(Pos2::ZERO, size);
                let mut roads = RoadTypes::default();
                roads.open_maker(false);
                let spec = roads.maker.as_mut().unwrap();
                spec.name = "My road?".into();
                spec.advanced = advanced;
                spec.marked = true;
                spec.markings = vec![Marking::default(); 5];
                if advanced {
                    spec.strips = vec![
                        crate::road_maker::Strip::Lane {
                            dir: crate::road_maker::Direction::Fwd,
                            width: 3.0,
                            bus_only: false,
                            path_offset: 0.0,
                        };
                        8
                    ];
                }
                if bus_preview {
                    roads.pending_bus_rules = Some(
                        (0..20)
                            .map(|i| crate::road_maker::bus_rules::batch::Group {
                                file: format!("road-{i}.sli"),
                                splines: Vec::new(),
                                paths: Vec::new(),
                            })
                            .collect(),
                    );
                }
                let mut ed = Editor::default();
                let lib = Library::default();
                let temp = std::env::temp_dir().join(format!(
                    "openmapper-scroll-{}-{}-{advanced}",
                    std::process::id(),
                    size.x
                ));
                let _ = std::fs::remove_dir_all(&temp);
                let roots = Roots {
                    omsi: temp.join("omsi"),
                    content: temp.join("content"),
                };
                std::fs::create_dir_all(&temp).unwrap();
                let texture = temp.join("asphalt.png");
                image::RgbaImage::from_pixel(8, 8, image::Rgba([70, 70, 70, 255]))
                    .save(&texture)
                    .unwrap();
                roads.maker.as_mut().unwrap().roadway_tex = texture;
                // Avoid scanning the working directory for textures.
                roads.textures = Some(Vec::new());
                let mut actions = Vec::new();
                let mut frame = |events| {
                    ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(screen),
                            events,
                            ..Default::default()
                        },
                        |root| {
                            roads.window(root.ctx(), &mut ed, &lib, Some(&roots), &mut actions);
                        },
                    )
                };
                let text_rect = |output: &egui::FullOutput, needle: &str| {
                    output
                        .shapes
                        .iter()
                        .rev()
                        .find_map(|shape| {
                            if let egui::Shape::Text(text) = &shape.shape {
                                if text.galley.job.text.contains(needle) {
                                    return Some((
                                        egui::Rect::from_min_size(text.pos, text.galley.size()),
                                        shape.clip_rect,
                                    ));
                                }
                            }
                            None
                        })
                        .unwrap_or_else(|| panic!("missing text: {needle}"))
                };
                let mut output = frame(Vec::new());
                for _ in 0..4 {
                    output = frame(Vec::new());
                }
                let (create, clip) = text_rect(&output, "Create the road type");
                assert!(screen.contains_rect(create) && clip.contains_rect(create));
                let (warning, clip) = text_rect(&output, "Some characters in the road name");
                assert!(screen.contains_rect(warning) && clip.contains_rect(warning));
                let (title, title_clip) = text_rect(&output, "Road maker");
                assert!(title_clip.contains_rect(title));
                let hover = title.center() + Vec2::new(100.0, 80.0);
                for _ in 0..8 {
                    output = frame(vec![
                        egui::Event::PointerMoved(hover),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            phase: egui::TouchPhase::Move,
                            delta: Vec2::new(0.0, -10000.0),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                }
                let (last, clip) = text_rect(&output, "The roadway texture is laid once");
                assert!(
                    clip.contains_rect(last),
                    "cannot reach bottom: {size:?}, advanced={advanced}: {last:?} / {clip:?}"
                );
                let (after, clip) = text_rect(&output, "Create the road type");
                assert_eq!(create, after);
                assert!(screen.contains_rect(after) && clip.contains_rect(after));
                // A real click must reach Create, not only paint it within the screen.
                for pressed in [true, false] {
                    output = frame(vec![
                        egui::Event::PointerMoved(after.center()),
                        egui::Event::PointerButton {
                            pos: after.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                }
                let (cancel, clip) = text_rect(&output, "Cancel");
                assert!(clip.contains_rect(cancel));
                let pos = cancel.center();
                for pressed in [true, false] {
                    frame(vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                }
                drop(frame);
                assert!(roads.maker.is_none(), "Cancel must remain clickable");
                assert!(
                    actions
                        .iter()
                        .any(|a| matches!(a, Action::ReloadContent(_))),
                    "Create must write the road: {:?}",
                    roads.error
                );
                std::fs::remove_dir_all(temp).unwrap();
            }
        }
    }
}
