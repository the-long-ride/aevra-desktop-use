use super::*;

impl GuiApp {
    pub(super) fn draw_settings(&mut self, ui: &mut egui::Ui) {
        theme::section(
            ui,
            "Settings",
            "Connect MCP clients and manage this desktop server.",
        );
        theme::group(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            theme::group_title(ui, "Desktop permissions");
            if cfg!(target_os="macos") {
                ui.label("Allow Accessibility for app control and Screen Recording for screenshots in System Settings > Privacy & Security. Reconnect the MCP client after changing permissions.");
            } else if cfg!(target_os="linux") {
                ui.label("AX/AT-SPI semantic actions use the accessibility bus. X11 needs XTest and a compositing window manager. GNOME Wayland needs the Aevra Desktop Access companion with access enabled and the helper path approved in its panel menu; locking or pausing revokes access. KDE KWin 6.6 needs its native companion and a private helper-path grant; lock/restart requires a fresh grant and reconnect.");
            } else {
                ui.label("Desktop control needs an interactive user session. Protected desktops and elevated targets are refused.");
            }
            ui.label(egui::RichText::new("Run aevra-desktop doctor for current backend readiness and recovery steps.").small().weak());
        });
        ui.add_space(16.0);
        theme::group(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
        theme::group_title(ui, "HTTP connection");
        ui.label("Lets MCP clients connect over http://127.0.0.1 with a bearer token. Off by default; stdio needs no server.");
        let mut enabled = self.config.http.enabled;
        if ui
            .add_enabled(
                self.problem.is_none(),
                egui::Checkbox::new(&mut enabled, "Enable the HTTP server"),
            )
            .changed()
        {
            let mut next = self.config.clone();
            next.http.enabled = enabled;
            self.commit(next);
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Port");
            ui.add(
                egui::TextEdit::singleline(&mut self.port_text)
                    .desired_width(70.0)
                    .min_size(egui::vec2(0.0, 32.0)),
            );
            if ui
                .add_enabled(self.problem.is_none(), egui::Button::new("Apply port"))
                .clicked()
            {
                match settings::parse_port(&self.port_text) {
                    Ok(port) => {
                        let mut next = self.config.clone();
                        next.http.port = port;
                        self.commit(next);
                    }
                    Err(message) => self.status = message,
                }
            }
        });
        let status = match self.services.http.status() {
            HttpStatus::Stopped => "Stopped".to_string(),
            HttpStatus::Running(addr) => format!("Listening on http://{addr}/mcp"),
            HttpStatus::Failed(why) => format!("Not running: {why}"),
        };
        let p = theme::colors(ui);
        let color = match self.services.http.status() {
            HttpStatus::Running(_) => p.success,
            HttpStatus::Failed(_) => p.danger,
            HttpStatus::Stopped => p.muted,
        };
        ui.colored_label(color, format!("Status: {status}"));
        });
        ui.add_space(16.0);
        theme::group(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            theme::group_title(ui, "Access token");
            ui.label("Regenerating the token disconnects clients using the old token.");
            ui.horizontal_wrapped(|ui| {
                ui.monospace(settings::token_display(
                    &self.config.http.token,
                    self.reveal_token,
                ));
                let label = if self.reveal_token { "Hide" } else { "Show" };
                if ui.button(label).clicked() {
                    self.reveal_token = !self.reveal_token;
                }
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(self.config.http.token.clone());
                    self.status = "Token copied.".to_string();
                }
                if ui
                    .add_enabled(
                        self.problem.is_none(),
                        egui::Button::new("Regenerate token"),
                    )
                    .clicked()
                {
                    let mut next = self.config.clone();
                    settings::regenerate_token(&mut next);
                    self.commit(next);
                }
            });
        });
        ui.add_space(16.0);
        theme::group(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
        theme::group_title(ui, "MCP client configuration");
        self.snippet(
            ui,
            "stdio (recommended)",
            settings::stdio_snippet(&self.services.exe),
        );
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(12.0);
        let http = self.config.http.clone();
        self.snippet(ui, "HTTP", settings::http_snippet(http.port, &http.token));
        ui.label(egui::RichText::new("The HTTP snippet contains your token. Paste it into the client, not into chat or a repository.").small().weak());
        });
        ui.add_space(16.0);
        theme::group(ui).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            theme::group_title(ui, "Configuration file");
            ui.add(egui::Label::new(self.services.config_path.display().to_string()).wrap());
        });
    }

    fn snippet(&mut self, ui: &mut egui::Ui, title: &str, text: String) {
        ui.horizontal_wrapped(|ui| {
            ui.strong(title);
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(text.clone());
                self.status = format!("Copied the {title} configuration.");
            }
        });
        let mut shown = if title == "HTTP" && !self.reveal_token {
            text.replace(
                &self.config.http.token,
                &settings::token_display(&self.config.http.token, false),
            )
        } else {
            text
        };
        ui.add(
            egui::TextEdit::multiline(&mut shown)
                .code_editor()
                .desired_rows(7)
                .desired_width(f32::INFINITY)
                .interactive(false),
        );
    }
}
