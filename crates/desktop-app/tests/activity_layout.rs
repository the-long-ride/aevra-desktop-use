//! Exercise the production Activity renderer without a native window or OS services.
#![allow(dead_code)]
use eframe::egui;
use std::path::PathBuf;
#[path = "../src/ui/activity.rs"]
mod activity;
#[path = "../src/ui/activity_cache.rs"]
mod activity_cache;
#[path = "../src/ui/activity_controls.rs"]
mod activity_controls;
#[path = "../src/ui/activity_history.rs"]
mod activity_history;
#[path = "../src/ui/activity_query.rs"]
mod activity_query;
#[path = "../src/ui/activity_screen.rs"]
mod activity_screen;
#[path = "../src/ui/theme.rs"]
mod theme;
enum Tab {
    Activity,
    Settings,
}
struct Services {
    config_path: PathBuf,
}
struct GuiApp {
    services: Services,
    activity: activity_history::History,
    activity_query: activity_query::Query,
    activity_cache: activity_cache::Cache,
    tab: Tab,
}

#[test]
fn retained_refresh_keeps_table_geometry_and_timestamp() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut entry = desktop_policy::activity::ActivityEntry::new("desktop_windows", "allow");
    entry.ts = "2026-10-15T08:15:30.123Z".into();
    let mut history = activity_history::History::default();
    history.entries = vec![entry];
    history.initialized = true;
    let mut app = GuiApp {
        services: Services {
            config_path: PathBuf::from("unused.json"),
        },
        activity: history,
        activity_query: Default::default(),
        activity_cache: Default::default(),
        tab: Tab::Activity,
    };
    let mut render = |loading| {
        app.activity.loading = loading;
        let mut frame = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| app.draw_activity(ui),
        );
        frame.textures_delta.clear();
        frame
            .shapes
            .into_iter()
            .filter_map(|item| match item.shape {
                egui::epaint::Shape::Text(text)
                    if ["desktop_windows", "Tool", "2026-10-15 08:15:30"]
                        .contains(&text.galley.text()) =>
                {
                    Some((text.galley.text().to_owned(), text.pos))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    render(false); // Fonts and layout settle before comparing retained frames.
    let idle = render(false);
    assert_eq!(
        idle.len(),
        3,
        "header, first row and displayed UTC timestamp must be present"
    );
    assert_eq!(
        idle,
        render(true),
        "background polling must not move the table"
    );
    assert_eq!(idle, render(false));
}
