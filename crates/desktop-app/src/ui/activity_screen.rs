use super::*;
use activity_query::Sort;

impl GuiApp {
    pub(super) fn draw_activity(&mut self, ui: &mut egui::Ui) {
        theme::section(
            ui,
            "Activity",
            "Retained tool calls. Typed values are never recorded, only their lengths.",
        );
        if !self.activity.initialized && !self.activity.loading {
            self.activity.refresh(
                ui.ctx(),
                self.services
                    .config_path
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .to_path_buf(),
            );
        }
        self.activity_controls(ui);
        ui.add_space(16.0);
        let page = self.activity_cache.resolve(
            &mut self.activity_query,
            &self.activity.entries,
            self.activity.revision,
        );
        let p = theme::colors(ui);
        theme::group(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if self.activity.initialized {
                    ui.label(format!(
                        "{}-{} of {} matching calls",
                        page.start, page.end, page.total
                    ));
                    ui.label(
                        egui::RichText::new(format!("{} retained", self.activity.entries.len()))
                            .small()
                            .color(p.muted),
                    );
                } else {
                    ui.label("Loading retained activity...");
                }
            });
            ui.separator();
            if page.rows.is_empty() {
                if !self.activity.initialized {
                    theme::empty(
                        ui,
                        "Loading activity",
                        "Reading retained history in the background.",
                    );
                } else if self.activity.entries.is_empty() {
                    theme::empty(
                        ui,
                        "No tool calls yet",
                        "Connect an MCP client in Settings to see desktop calls here.",
                    );
                    if ui.button("Open Settings").clicked() {
                        self.tab = Tab::Settings;
                    }
                } else {
                    theme::empty(
                        ui,
                        "No matching calls",
                        "Change the search or filters to see more results.",
                    );
                    if ui.button("Clear search and filters").clicked() {
                        self.clear_activity_filters();
                    }
                }
            } else {
                let height = (ui.available_height() - 114.0).clamp(80.0, 320.0);
                egui::ScrollArea::horizontal()
                    .id_salt("activity_columns")
                    .show(ui, |ui| {
                        let widths = [
                            180.0,
                            205.0,
                            155.0,
                            (ui.available_width() - 570.0).max(240.0),
                        ];
                        ui.horizontal(|ui| {
                            for ((label, sort), width) in [
                                ("Date/time UTC", Sort::Time),
                                ("Tool", Sort::Tool),
                                ("App", Sort::App),
                                ("Result", Sort::Result),
                            ]
                            .into_iter()
                            .zip(widths)
                            {
                                let suffix = if self.activity_query.sort == sort {
                                    if self.activity_query.descending {
                                        " (desc)"
                                    } else {
                                        " (asc)"
                                    }
                                } else {
                                    ""
                                };
                                if ui
                                    .add_sized(
                                        [width, 34.0],
                                        egui::Button::new(
                                            egui::RichText::new(format!("{label}{suffix}"))
                                                .strong(),
                                        )
                                        .fill(p.soft)
                                        .truncate(),
                                    )
                                    .on_hover_text("Sort by this column; click again to reverse")
                                    .clicked()
                                {
                                    self.activity_query.select_sort(sort);
                                }
                            }
                        });
                        egui::ScrollArea::vertical()
                            .id_salt("activity_rows")
                            .max_height(height)
                            .show(ui, |ui| {
                                for (index, row) in page.rows.iter().enumerate() {
                                    egui::Frame::new()
                                        .fill(if index % 2 == 0 { p.soft } else { p.canvas })
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                let time = row
                                                    .timestamp
                                                    .split('.')
                                                    .next()
                                                    .unwrap_or(&row.timestamp)
                                                    .trim_end_matches('Z')
                                                    .replace('T', " ");
                                                table_cell(ui, &time, widths[0], p.ink);
                                                table_cell(ui, &row.tool, widths[1], p.ink);
                                                table_cell(ui, &row.app, widths[2], p.ink);
                                                let (label, color) = match row.tone {
                                                    activity::Tone::Ok => ("Allowed", p.success),
                                                    activity::Tone::Denied => ("Denied", p.warning),
                                                    activity::Tone::Error => ("Error", p.danger),
                                                };
                                                table_cell(
                                                    ui,
                                                    &format!("{label}: {}", row.outcome),
                                                    widths[3],
                                                    color,
                                                );
                                            });
                                        });
                                }
                            });
                    });
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(self.activity_query.page > 0, egui::Button::new("Previous"))
                    .clicked()
                {
                    self.activity_query.page -= 1;
                }
                ui.label(format!(
                    "Page {} of {}",
                    self.activity_query.page + 1,
                    page.pages
                ));
                if ui
                    .add_enabled(
                        self.activity_query.page + 1 < page.pages,
                        egui::Button::new("Next"),
                    )
                    .clicked()
                {
                    self.activity_query.page += 1;
                }
            });
        });
    }
}

fn table_cell(ui: &mut egui::Ui, text: &str, width: f32, color: egui::Color32) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 40.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(width, 40.0));
            ui.set_max_width(width);
            ui.add(egui::Label::new(egui::RichText::new(text).color(color)).truncate())
                .on_hover_text(text);
        },
    );
}
