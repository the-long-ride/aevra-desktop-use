//! The tray icon and its menu: "Open aevra-desktop", "HTTP server: on/off" and
//! "Quit". Linux uses tray-icon's StatusNotifierItem backend, so there is no
//! GTK main loop to run; a desktop without a StatusNotifier host simply has no
//! tray, and [`Tray::new`] reports that instead of failing the program.

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Open,
    ToggleHttp,
    Quit,
}

/// The ids of the three menu items, cloneable into event handlers.
#[derive(Debug, Clone)]
pub struct Ids {
    pub open: MenuId,
    pub http: MenuId,
    pub quit: MenuId,
}

impl Ids {
    pub fn action(&self, clicked: &MenuId) -> Option<TrayAction> {
        action_for(clicked, &self.open, &self.http, &self.quit)
    }
}

pub struct Tray {
    _icon: TrayIcon,
    http: MenuItem,
    ids: Ids,
}

pub fn http_label(enabled: bool) -> String {
    format!("HTTP server: {}", if enabled { "on" } else { "off" })
}

/// RGBA8 pixels of the canonical Aevra app icon.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    crate::branding::icon(size).rgba
}

/// Maps a menu click to an action, given the ids of the three items.
pub fn action_for(
    clicked: &MenuId,
    open: &MenuId,
    http: &MenuId,
    quit: &MenuId,
) -> Option<TrayAction> {
    if clicked == open {
        Some(TrayAction::Open)
    } else if clicked == http {
        Some(TrayAction::ToggleHttp)
    } else if clicked == quit {
        Some(TrayAction::Quit)
    } else {
        None
    }
}

impl Tray {
    /// Creates the tray icon. Call it on the thread that runs the event loop.
    pub fn new(http_enabled: bool) -> Result<Self, String> {
        let open = MenuItem::new("Open aevra-desktop", true, None);
        let http = MenuItem::new(http_label(http_enabled), true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &http, &PredefinedMenuItem::separator(), &quit])
            .map_err(|e| e.to_string())?;
        let icon = Icon::from_rgba(icon_rgba(32), 32, 32).map_err(|e| e.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("aevra-desktop")
            .with_icon(icon)
            .build()
            .map_err(|e| e.to_string())?;
        let ids = Ids {
            open: open.id().clone(),
            http: http.id().clone(),
            quit: quit.id().clone(),
        };
        Ok(Self {
            _icon: tray,
            http,
            ids,
        })
    }

    pub fn ids(&self) -> Ids {
        self.ids.clone()
    }

    pub fn action(&self, event: &MenuEvent) -> Option<TrayAction> {
        self.ids.action(&event.id)
    }

    pub fn set_http_enabled(&self, enabled: bool) {
        self.http.set_text(http_label(enabled));
    }
}

/// A left click on the icon itself, as opposed to its menu.
pub fn is_open_click(event: &TrayIconEvent) -> bool {
    matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brand_icon_is_valid_rgba_with_visible_and_translucent_pixels() {
        let pixels = icon_rgba(32);
        assert_eq!(pixels.len(), 32 * 32 * 4);
        assert!(pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] == 255));
        assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] < 255));
        // Accepted by the tray library.
        assert!(Icon::from_rgba(pixels, 32, 32).is_ok());
    }

    #[test]
    fn menu_ids_map_to_actions() {
        let open = MenuId::new("open");
        let http = MenuId::new("http");
        let quit = MenuId::new("quit");
        assert_eq!(
            action_for(&open, &open, &http, &quit),
            Some(TrayAction::Open)
        );
        assert_eq!(
            action_for(&http, &open, &http, &quit),
            Some(TrayAction::ToggleHttp)
        );
        assert_eq!(
            action_for(&quit, &open, &http, &quit),
            Some(TrayAction::Quit)
        );
        assert_eq!(action_for(&MenuId::new("other"), &open, &http, &quit), None);
    }

    #[test]
    fn the_http_item_says_on_or_off() {
        assert_eq!(http_label(true), "HTTP server: on");
        assert_eq!(http_label(false), "HTTP server: off");
    }
}
