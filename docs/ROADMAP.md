# Desktop roadmap

## Current support and validation

- **1.1.0 custom HTTP startup port** is implemented for GUI/default mode. The override is invocation-only and preserves saved settings, HTTP enable/disable and the separate wake-up socket. See the [current specification](specs/2026-10-19-custom-startup-port.md) and [release notes](../CHANGELOG.md#110---2026-10-19).
- **Ubuntu X11** uses XTest/Composite. **GNOME Wayland** uses the explicit-install Aevra Desktop Access companion with a human-approved helper path. Private GNOME 50 on Ubuntu 26.04 has native capture/input/sentinel/permission evidence; GNOME 46 CI is configured. Both products consume the canonical helper implementation.
- **KDE KWin 6.6 Wayland** uses a native companion with an approved helper-path grant; private software-rendered capture/input acceptance passes in both repositories.
- Semantic accessibility actions remain available through AT-SPI when the provider is reachable.
- macOS adapters and fixtures are implemented; native Mac build/runtime acceptance remains outstanding.

## Planned work

- **Wayland acceptance and wider compositor support:** GPU capture acceptance and additional KWin versions remain follow-up work. Other compositors and a generic portal/PipeWire/EIS adapter remain future work and must preserve source-to-allowlisted-app proof and native revocation tests.

On GNOME Wayland install/enable the bundled companion, enable access and approve the helper path from its panel menu, then reconnect. Pause/lock revokes access. Window capture does not fall back to monitor pixels; XWayland and privileged uinput are not native Wayland fallbacks.
