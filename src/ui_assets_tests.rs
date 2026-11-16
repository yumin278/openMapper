//! Headless tests of the asset browser's own window and of the layout kept for it: the size
//! it settles at, when that size is written to the settings, and how a saved layout (old,
//! impossible, or off the screen) is read back.
//!
//! The panel is run through an egui `Context` without a renderer, as `ui_crossing`'s tests do:
//! this checks egui's computed bounds, not rendered behaviour or Windows input routing.

use super::*;

use crate::install::{AssetsLayout, Prefs};

const SCREEN: Vec2 = Vec2::new(1400.0, 900.0);

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_fonts(icons::fonts());
    ctx
}

/// One frame of the browser, with the `prefs` updated from what it asks to save, as
/// `app.rs` does on `Action::SetAssetsLayout`.
fn run(ctx: &egui::Context, browser: &mut AssetBrowser, ed: &mut Editor, prefs: &mut Prefs, events: Vec<egui::Event>, time: f64) -> Vec<Action> {
    let lib = Library::default();
    let thumbs: HashMap<String, Option<Thumb>> = HashMap::new();
    let swatches: HashMap<String, egui::TextureHandle> = HashMap::new();
    let mut wanted: Vec<(String, bool)> = Vec::new();
    let mut wanted_swatches: Vec<String> = Vec::new();
    let mut actions = Vec::new();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, SCREEN)),
        time: Some(time),
        events,
        ..Default::default()
    };
    let prefs_now = prefs.clone();
    let _ = ctx.run_ui(input, |ui| {
        browser.panel(ui, ed, &lib, &thumbs, &mut wanted, &[], &[], &swatches, &mut wanted_swatches, &prefs_now, &mut actions);
    });
    // what the application does with the action: the prefs take the layout and are saved
    for a in &actions {
        if let Action::SetAssetsLayout(layout) = a {
            if prefs.assets_layout != *layout {
                prefs.assets_layout = *layout;
            }
        }
    }
    actions
}

fn saved_layout(actions: &[Action]) -> Option<AssetsLayout> {
    actions.iter().find_map(|a| if let Action::SetAssetsLayout(l) = a { Some(*l) } else { None })
}

fn button_at(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::default() }
}

/// The window's collapsing state, as the window itself stores it (`containers_window.rs` keeps
/// it under the area's id).
fn set_collapsed(ctx: &egui::Context, open: bool) {
    let id = egui::Id::new("assets window").with("collapsing");
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ctx, id, true);
    state.set_open(open);
    state.store(ctx);
}

fn rect_of(bounds: [f32; 4]) -> egui::Rect {
    egui::Rect::from_min_size(Pos2::new(bounds[0], bounds[1]), Vec2::new(bounds[2], bounds[3]))
}

/// Settings written before the browser could be docked carry no `assets_layout`: the browser
/// opens floating, with no bounds of its own to force it anywhere.
#[test]
fn old_settings_open_the_assets_floating() {
    let old = r#"{"omsi_root":"C:\\OMSI 2","language":"en","camera":"map","ui_scale":1.0,"hints_seen":true}"#;
    let prefs: Prefs = serde_json::from_str(old).expect("old settings still read");
    assert_eq!(prefs.assets_layout, AssetsLayout::default());
    assert!(!prefs.assets_layout.docked, "an old settings file must not dock the browser");
    assert_eq!(prefs.assets_layout.rect, None);

    // and the field may be there but empty, which is the same
    let empty: Prefs = serde_json::from_str(r#"{"assets_layout":{}}"#).unwrap();
    assert_eq!(empty.assets_layout, AssetsLayout::default());
}

/// Docking and the last bounds go through the settings file unchanged.
#[test]
fn the_assets_layout_survives_a_round_trip() {
    let layout = AssetsLayout { docked: true, rect: Some([120.0, 80.0, 900.0, 320.0]) };
    let prefs = Prefs { assets_layout: layout, ..Default::default() };
    let text = serde_json::to_string(&prefs).expect("the settings write");
    assert!(text.contains("\"assets_layout\""), "{text}");
    let back: Prefs = serde_json::from_str(&text).expect("the settings read back");
    assert_eq!(back.assets_layout, layout);

    // the rect alone docks nothing: it is still the older, floating default
    let floating: Prefs = serde_json::from_str(r#"{"assets_layout":{"rect":[40.0,50.0,800.0,300.0]}}"#).unwrap();
    assert!(!floating.assets_layout.docked);
    assert_eq!(floating.assets_layout.window_rect(), Some(egui::Rect::from_min_size(Pos2::new(40.0, 50.0), Vec2::new(800.0, 300.0))));
}

/// Nothing is read from a rect that cannot be shown (a bad file, a hand edit): the window
/// asks for its own size instead of vanishing or growing to infinity.
#[test]
fn an_impossible_rect_is_never_used() {
    let bad = [
        [f32::NAN, 10.0, 900.0, 320.0],
        [10.0, f32::INFINITY, 900.0, 320.0],
        [10.0, 10.0, 0.0, 320.0],
        [10.0, 10.0, 900.0, -5.0],
        [f32::NEG_INFINITY, f32::NAN, f32::NAN, 0.0],
    ];
    for rect in bad {
        let layout = AssetsLayout { docked: false, rect: Some(rect) };
        assert!(layout.window_rect().is_none(), "{rect:?} must not be used as the window's bounds");
    }

    // the same rect in a settings file: read, then ignored
    let prefs: Prefs = serde_json::from_str(r#"{"assets_layout":{"rect":[12.0,9.0,0.0,0.0]}}"#).unwrap();
    assert_eq!(prefs.assets_layout.rect, Some([12.0, 9.0, 0.0, 0.0]));
    assert!(prefs.assets_layout.window_rect().is_none());

    // a good rect comes back exactly as it was saved
    let good = AssetsLayout { docked: false, rect: Some([11.0, 12.0, 900.0, 320.0]) };
    let r = good.window_rect().expect("a finite, positive rect is used");
    assert_eq!((r.left(), r.top(), r.width(), r.height()), (11.0, 12.0, 900.0, 320.0));
}

/// Left alone, the floating window settles at one size and keeps it: the layout saved once is
/// the layout the next frame starts from, so no bounds are asked to be saved every frame.
#[test]
fn the_floating_window_keeps_still_and_saves_its_bounds_once() {
    let ctx = context();
    let mut ed = Editor::default();
    let mut browser = AssetBrowser { open: true, ..Default::default() };
    let mut prefs = Prefs::default();
    let mut save_frames: Vec<usize> = Vec::new();
    let mut early = None;
    let mut late = None;
    let mut time = 0.0;
    for frame in 0..24 {
        let actions = run(&ctx, &mut browser, &mut ed, &mut prefs, vec![], time);
        if saved_layout(&actions).is_some() {
            save_frames.push(frame);
        }
        if frame == 5 {
            early = browser.layout.and_then(|l| l.rect);
        }
        if frame == 23 {
            late = browser.layout.and_then(|l| l.rect);
        }
        time += 0.05;
    }
    assert!(!save_frames.is_empty(), "the floating window never saved its bounds");
    assert!(save_frames.len() <= 2, "the settled window asks to save again and again: {save_frames:?}");
    assert!(save_frames.iter().all(|f| *f < 4), "the bounds churn before settling: {save_frames:?}");

    let early = early.expect("the window laid out and saved its bounds");
    let late = late.expect("the window kept its bounds");
    for i in 0..4 {
        assert!((early[i] - late[i]).abs() < 0.51, "the window moved while nothing happened: {early:?} then {late:?}");
    }
    assert!(late[2] >= 480.0 - 0.5 && late[3] >= 200.0 - 0.5, "the window is below its minimum: {late:?}");
    assert_eq!(prefs.assets_layout, browser.layout.unwrap(), "the prefs took the bounds the window asked to save");
}

/// The layout is written when the hand lets go, not while a window is being dragged or
/// resized: a half-dragged rect must not be the one remembered.
#[test]
fn the_layout_is_saved_only_after_the_pointer_is_released() {
    let ctx = context();
    let mut ed = Editor::default();
    let wanted = AssetsLayout { docked: false, rect: Some([160.0, 120.0, 900.0, 320.0]) };
    let mut browser = AssetBrowser { open: true, layout: Some(wanted), ..Default::default() };
    let mut prefs = Prefs::default();
    let pos = Pos2::new(1200.0, 760.0);
    let mut time = 0.0;

    let held = run(&ctx, &mut browser, &mut ed, &mut prefs, vec![button_at(pos, true)], time);
    assert!(saved_layout(&held).is_none(), "the bounds were saved with the mouse still down");
    time += 0.05;
    let still_held = run(&ctx, &mut browser, &mut ed, &mut prefs, vec![], time);
    assert!(saved_layout(&still_held).is_none(), "the bounds were saved while the mouse stayed down");
    time += 0.05;

    let released = run(&ctx, &mut browser, &mut ed, &mut prefs, vec![button_at(pos, false)], time);
    let saved = saved_layout(&released).expect("the bounds are saved once the mouse is let go");
    assert_eq!(Some(saved), browser.layout, "what is saved is the layout the window has");
    assert_eq!(prefs.assets_layout, saved);
}

/// Collapsing the window to its title bar must not replace the size it had expanded: moving
/// the collapsed bar moves only the place, and opening it again brings the same window back.
#[test]
fn collapsing_keeps_the_expanded_bounds() {
    let ctx = context();
    let mut ed = Editor::default();
    let mut browser = AssetBrowser { open: true, ..Default::default() };
    let mut prefs = Prefs::default();
    let mut time = 0.0;
    for _ in 0..6 {
        run(&ctx, &mut browser, &mut ed, &mut prefs, vec![], time);
        time += 0.1;
    }
    let expanded = browser.layout.unwrap().rect.expect("the expanded window saved its bounds");

    set_collapsed(&ctx, false);
    for _ in 0..20 {
        run(&ctx, &mut browser, &mut ed, &mut prefs, vec![], time);
        time += 0.1;
    }
    let collapsed = browser.layout.unwrap().rect.expect("the collapsed window keeps bounds");
    assert!((collapsed[2] - expanded[2]).abs() < 0.51, "collapsing replaced the expanded width: {expanded:?} -> {collapsed:?}");
    assert!((collapsed[3] - expanded[3]).abs() < 0.51, "collapsing replaced the expanded height: {expanded:?} -> {collapsed:?}");
    assert!((collapsed[0] - expanded[0]).abs() < 0.51 && (collapsed[1] - expanded[1]).abs() < 0.51, "the collapsed bar sits where the window was: {expanded:?} -> {collapsed:?}");

    set_collapsed(&ctx, true);
    for _ in 0..30 {
        run(&ctx, &mut browser, &mut ed, &mut prefs, vec![], time);
        time += 0.1;
    }
    let reopened = browser.layout.unwrap().rect.expect("the reopened window saved its bounds");
    for i in 0..4 {
        assert!((reopened[i] - expanded[i]).abs() < 0.51, "the window came back changed: {expanded:?} -> {reopened:?}");
    }
}

/// Bounds kept from a screen that no longer exists (a smaller display, a rect typed by hand)
/// are brought back into view, not left where the window cannot be reached.
#[test]
fn an_offscreen_rect_is_brought_back_onto_the_screen() {
    let ctx = context();
    let mut ed = Editor::default();
    let requested = [5000.0, 4000.0, 900.0, 320.0];
    let mut browser = AssetBrowser { open: true, layout: Some(AssetsLayout { docked: false, rect: Some(requested) }), ..Default::default() };
    let mut prefs = Prefs::default();
    let mut time = 0.0;
    for _ in 0..4 {
        run(&ctx, &mut browser, &mut ed, &mut prefs, vec![], time);
        time += 0.05;
    }
    let saved = browser.layout.unwrap().rect.expect("the window saved where it ended up");
    assert!(saved[0] < requested[0] && saved[1] < requested[1], "the window stayed off the screen: {saved:?}");
    let rect = rect_of(saved);
    let screen = egui::Rect::from_min_size(Pos2::ZERO, SCREEN);
    assert!(
        rect.left() >= screen.left() - 1.0 && rect.top() >= screen.top() - 1.0 && rect.right() <= screen.right() + 1.0 && rect.bottom() <= screen.bottom() + 1.0,
        "the window {rect:?} is not on the screen {screen:?}"
    );
}

/// The browser remembers its layout even while hidden: it is asked for a frame with `open`
/// false, and when the layout it holds differs from the settings, it asks for that one.
#[test]
fn a_closed_browser_still_saves_its_layout() {
    let ctx = context();
    let mut ed = Editor::default();
    let left = AssetsLayout { docked: true, rect: Some([40.0, 60.0, 900.0, 320.0]) };
    let mut browser = AssetBrowser { open: false, layout: Some(left), ..Default::default() };
    let mut prefs = Prefs::default();
    let actions = run(&ctx, &mut browser, &mut ed, &mut prefs, vec![], 0.0);
    assert_eq!(actions.len(), 1, "the closed browser should ask for its layout to be saved once");
    assert_eq!(saved_layout(&actions), Some(left));
    assert_eq!(prefs.assets_layout, left);

    // once the settings agree there is nothing to save, and a closed browser draws nothing
    let again = run(&ctx, &mut browser, &mut ed, &mut prefs, vec![], 0.05);
    assert!(saved_layout(&again).is_none());
}
