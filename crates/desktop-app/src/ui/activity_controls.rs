use super::*;
use activity_query::Query;
use std::collections::BTreeSet;

impl GuiApp {
    pub(super) fn activity_controls(&mut self, ui: &mut egui::Ui) {
        let before = self.activity_query.clone();
        theme::group(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let wide = ui.available_width() >= 1000.0;
            if wide {
                ui.horizontal(|ui| {
                    self.activity_search_controls(ui);
                    ui.add_space(12.0);
                    self.activity_filter_controls(ui);
                });
            } else {
                ui.horizontal_wrapped(|ui| self.activity_search_controls(ui));
                ui.horizontal_wrapped(|ui| self.activity_filter_controls(ui));
            }
        });
        if before != self.activity_query {
            self.activity_query.page = 0;
        }
    }

    pub(super) fn clear_activity_filters(&mut self) {
        self.activity_query = Query {
            sort: self.activity_query.sort,
            descending: self.activity_query.descending,
            page_size: self.activity_query.page_size,
            ..Query::default()
        };
    }

    fn activity_search_controls(&mut self, ui: &mut egui::Ui) {
        ui.label("Search");
        ui.add(
            egui::TextEdit::singleline(&mut self.activity_query.search)
                .hint_text("Tool, app or result…")
                .desired_width(260.0)
                .min_size(egui::vec2(0.0, 32.0)),
        );
        if ui.button("Clear").clicked() {
            self.clear_activity_filters();
        }
    }
    fn activity_filter_controls(&mut self, ui: &mut egui::Ui) {
        ui.label("Outcome");
        egui::ComboBox::from_id_salt("activity_verdict")
            .selected_text(match self.activity_query.verdict {
                None => "All outcomes",
                Some(activity::Tone::Ok) => "Allowed",
                Some(activity::Tone::Denied) => "Denied",
                Some(activity::Tone::Error) => "Error",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.activity_query.verdict, None, "All outcomes");
                for (tone, text) in [
                    (activity::Tone::Ok, "Allowed"),
                    (activity::Tone::Denied, "Denied"),
                    (activity::Tone::Error, "Error"),
                ] {
                    ui.selectable_value(&mut self.activity_query.verdict, Some(tone), text);
                }
            });
        ui.label("App");
        egui::ComboBox::from_id_salt("activity_app")
            .selected_text(self.activity_query.app.as_deref().unwrap_or("All apps"))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.activity_query.app, None, "All apps");
                let mut apps: BTreeSet<_> = self
                    .activity
                    .entries
                    .iter()
                    .map(|e| e.app.clone().unwrap_or_else(|| "-".into()))
                    .collect();
                if let Some(selected) = &self.activity_query.app {
                    apps.insert(selected.clone());
                }
                for app in apps {
                    ui.selectable_value(&mut self.activity_query.app, Some(app.clone()), &app);
                }
            });
        ui.label("Rows");
        egui::ComboBox::from_id_salt("activity_size")
            .selected_text(self.activity_query.page_size.to_string())
            .show_ui(ui, |ui| {
                for size in [25, 50, 100] {
                    ui.selectable_value(&mut self.activity_query.page_size, size, size.to_string());
                }
            });
    }
}
