use super::*;

impl GuiApp {
    pub(super) fn draw_apps(&mut self, ui: &mut egui::Ui) {
        let p = theme::colors(ui);
        theme::section(
            ui,
            "Allowed apps",
            "Choose which apps MCP clients can see and control.",
        );
        let (entries, loading) = {
            let view = self.catalog.lock().unwrap();
            (view.entries.clone(), view.loading)
        };
        theme::group(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search apps by name…")
                        .desired_width((ui.available_width() - 250.0).max(180.0))
                        .min_size(egui::vec2(0.0, 32.0)),
                );
                if ui
                    .add_enabled(!loading, egui::Button::new("Rescan apps"))
                    .clicked()
                {
                    self.rescan(ui.ctx(), true);
                }
                if ui
                    .add_enabled(self.problem.is_none(), egui::Button::new("Add custom…"))
                    .clicked()
                {
                    self.add_custom_app();
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} allowed", self.config.allowed_apps.len()))
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!("{} discovered", entries.len())).color(p.muted),
                );
                if loading {
                    ui.spinner();
                    ui.label("Scanning installed apps…");
                }
            });
        });
        ui.add_space(16.0);
        let rows = apps::apps_view_model(&entries, &self.config, &self.search);
        let mut toggled = None;
        egui::ScrollArea::vertical()
            .id_salt("apps_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                for row in &rows {
                    ui.push_id(crate::scan::identity_key(&row.identity), |ui| {
                        egui::Frame::new()
                            .fill(if row.allowed { p.soft } else { p.canvas })
                            .stroke(egui::Stroke::new(1.0, p.line))
                            .corner_radius(4)
                            .inner_margin(egui::Margin::same(12))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let mut allowed = row.allowed;
                                    if ui
                                        .add_enabled(
                                            self.problem.is_none(),
                                            egui::Checkbox::new(&mut allowed, &row.name),
                                        )
                                        .on_hover_text(format!("Allow MCP access to {}", row.name))
                                        .changed()
                                    {
                                        toggled =
                                            Some((row.name.clone(), row.identity.clone(), allowed));
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            theme::badge(
                                                ui,
                                                if row.allowed { "Allowed" } else { "Blocked" },
                                                if row.allowed { p.success } else { p.muted },
                                            );
                                            if row.running {
                                                theme::badge(ui, "Running", p.muted);
                                            }
                                            if !row.installed_entry {
                                                theme::badge(ui, "Custom", p.muted);
                                            }
                                        },
                                    );
                                });
                                let path = apps::identity_label(&row.identity);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&path).small().color(p.muted),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(path);
                            });
                    });
                    ui.add_space(0.0);
                }
                if rows.is_empty() && !loading {
                    if self.search.trim().is_empty() {
                        theme::empty(
                            ui,
                            "No apps discovered",
                            "Rescan apps, or add a custom executable to get started.",
                        );
                    } else {
                        theme::empty(
                            ui,
                            "No matching apps",
                            "Try another name or clear your search.",
                        );
                        ui.vertical_centered(|ui| {
                            if ui.button("Clear search").clicked() {
                                self.search.clear();
                            }
                        });
                    }
                }
            });
        if let Some((name, identity, allow)) = toggled {
            let mut next = self.config.clone();
            if apps::toggle_app(&mut next, &name, &identity, allow) {
                self.commit(next);
            }
        }
    }

    fn add_custom_app(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            match apps::custom_identity(&path) {
                Some((name, identity)) => {
                    let mut next = self.config.clone();
                    apps::toggle_app(&mut next, &name, &identity, true);
                    self.commit(next);
                }
                None => self.status = "That is not an app. Choose an executable file.".to_string(),
            }
        }
    }
}
