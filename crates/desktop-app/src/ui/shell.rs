use super::*;

impl GuiApp {
    pub(super) fn draw_shell(&mut self, ui: &mut egui::Ui) {
        let p = theme::colors(ui);
        egui::Panel::top("brand")
            .frame(
                egui::Frame::new()
                    .fill(p.canvas)
                    .inner_margin(egui::Margin::symmetric(20, 12)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::Image::new(&self.logo).fit_to_exact_size(egui::vec2(38.0, 38.0)));
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("aevra").size(19.0).strong());
                        ui.label(
                            egui::RichText::new("Desktop control")
                                .small()
                                .color(p.muted),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let dark = ui.visuals().dark_mode;
                        if ui
                            .button(if dark { "Light theme" } else { "Dark theme" })
                            .clicked()
                        {
                            ui.ctx().set_theme(if dark {
                                egui::Theme::Light
                            } else {
                                egui::Theme::Dark
                            });
                        }
                        let (color, detail) = match self.services.http.status() {
                            HttpStatus::Running(address) => {
                                (p.success, format!("Listening on http://{address}/mcp"))
                            }
                            HttpStatus::Stopped => (p.danger, "HTTP server is off".to_string()),
                            HttpStatus::Failed(error) => {
                                (p.danger, format!("HTTP server failed: {error}"))
                            }
                        };
                        ui.horizontal(|ui| {
                            ui.label("HTTP").on_hover_text(&detail);
                            let (rect, response) = ui
                                .allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 3.5, color);
                            response.on_hover_text(&detail);
                        });
                    });
                });
            });
        egui::Panel::top("navigation")
            .frame(
                egui::Frame::new()
                    .fill(p.soft)
                    .inner_margin(egui::Margin::symmetric(20, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (Tab::Apps, "Apps"),
                        (Tab::Activity, "Activity"),
                        (Tab::Settings, "Settings"),
                    ] {
                        let selected = self.tab == tab;
                        let text = egui::RichText::new(label).color(if selected {
                            p.canvas
                        } else {
                            p.ink
                        });
                        if ui
                            .add(egui::Button::new(text).fill(if selected {
                                p.ink
                            } else {
                                p.soft
                            }))
                            .clicked()
                        {
                            self.tab = tab;
                        }
                    }
                });
            });
        egui::Panel::bottom("feedback")
            .frame(
                egui::Frame::new()
                    .fill(p.soft)
                    .inner_margin(egui::Margin::symmetric(20, 8)),
            )
            .show(ui, |ui| {
                let footer_width = ui.available_width();
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(egui::vec2((footer_width - 320.0).max(100.0), 20.0),
                        egui::Layout::top_down(egui::Align::Min), |ui| {
                if let Some(problem) = &self.problem {
                    ui.colored_label(p.danger, format!("Changes disabled: {problem}"));
                    ui.label("Fix the config file to enable saving. Settings shows its location.");
                } else if !self.status.is_empty() {
                    let color = if self.status.starts_with("Saved")
                        || self.status.contains("copied")
                        || self.status.starts_with("Copied")
                    {
                        p.success
                    } else {
                        p.danger
                    };
                    ui.colored_label(color, &self.status);
                } else {
                    ui.label(
                        egui::RichText::new(
                            "Changes save automatically. Only allowed apps are accessible.",
                        )
                        .small()
                        .color(p.muted),
                    );
                }
                        });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        for (label, url) in [
                            ("the-long-ride", "https://github.com/the-long-ride/"),
                            (
                                "aevra-desktop-use",
                                "https://github.com/the-long-ride/aevra-desktop-use",
                            ),
                        ] {
                            ui.add(egui::Hyperlink::from_label_and_url(
                                egui::RichText::new(label).color(p.ink).underline(),
                                url,
                            ));
                        }
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.canvas).inner_margin(20))
            .show(ui, |ui| match self.tab {
                Tab::Apps => self.draw_apps(ui),
                Tab::Activity => {
                    if ui.available_height() < 420.0 {
                        egui::ScrollArea::vertical()
                            .id_salt("activity_compact")
                            .show(ui, |ui| self.draw_activity(ui));
                    } else {
                        self.draw_activity(ui);
                    }
                }
                Tab::Settings => {
                    egui::ScrollArea::vertical()
                        .id_salt("settings_scroll")
                        .show(ui, |ui| {
                            self.draw_settings(ui);
                        });
                }
            });
    }
}
