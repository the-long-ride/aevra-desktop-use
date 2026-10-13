# Changelog

## [Unreleased]

## [1.0.0] - 2026-10-19

### Added - Desktop management

- Match Aevra web styling with embedded canonical branding, light/dark themes, bordered content groups, and clearer spacing.
- Add Activity search, outcome/app filters, sortable date/tool/app/result columns, 25/50/100-row pages, and matching/retained counts across the current log and both rotated logs.
- Read history in the background and cache pages by actual history/query revisions so unchanged refreshes preserve content and no periodic refresh label flashes.
- Use a single Activity toolbar row in wide windows and two rows at narrower widths; add author and source-repository footer links and a red/green HTTP listener dot.

### Added - Custom startup port

- Choose the GUI's loopback HTTP port for one invocation with `--port=PORT`, `--p=PORT`, `--port PORT` or `--p PORT`. Accept digits-only values from 1 to 65535 and reject missing, invalid or duplicate options before startup.
- Preserve the HTTP enable switch while keeping the effective listener, Settings port display and copied HTTP client URL consistent through reloads. Keep the CLI override separate from saved settings and leave the window wake-up socket on its own OS-assigned port.
- Report an actionable error when a GUI is already running: quit it from the tray, then relaunch with the requested port. The stdio `mcp` command continues without an HTTP listener or port option.
- Document the implemented behavior and verification scope in the [startup-port specification](docs/specs/2026-10-19-custom-startup-port.md).

### Platform scope

- Preserve semantic accessibility control on Windows, macOS and Linux. Pixel capture and foreground input cover Windows, macOS 13+, Ubuntu X11, native GNOME Wayland and KDE KWin 6.6; native macOS execution and production Wayland approval/lock acceptance remain pending. Other Wayland compositors remain roadmap work.

### CI, build and fixes

- Build the Windows native .NET fixture with the runner account before entering the temporary standard-user logon. Pass fresh prepared output to required acceptance, preserve compiler diagnostics and cover build failures, missing output and child handoff.

- Request the required desktop read/write-object access alongside ACL-edit rights in the native CI launcher, and probe actual window-station/desktop security handles before running either acceptance path without changing their permissions.

- Fix native Windows CI on elevated hosts: use an ephemeral standard-user logon when SAFER filtering retains the elevation flag. Verify the suspended child token before execution, clean up account/ACL grants, and test failure/cleanup orchestration without touching real accounts. Keep production elevation refusal and required native acceptance.

- Restore the Node 24 version marker and use portable WebView2 fixture paths on Linux/macOS.
- Run required Windows background acceptance with a restricted medium-integrity token; retain production elevation refusal and fail when native fixture prerequisites are missing.
- Capture the config watcher baseline before starting its worker so an immediate allowlist save cannot be lost during startup.

- Fix macOS native build/test setup: explicitly select installed Xcode 26+ and verify Metal SDK declarations, persist its SDK path, and link Rust consumers with the system Swift runtime search path. Apply the same prerequisite to Desktop release builds and the root helper matrix.

- Fix GNOME 46 keyboard input refusal by distinguishing the stage fallback from Shell-owned focus; keep modal, overview and Shell actor guards, with a cross-version policy regression.
- Fix strict Rust 1.99 Clippy failures across desktop platforms using fixed-size pixel chunks, platform-specific compilation and simpler native bindings. Regenerate the shared helper overlay without weakening CI checks.

- Standardize current runtime/build CI on Node.js 24 and GitHub-maintained Actions `@v6`; add local `.node-version` files and keep independently versioned Rust cache support.
- Add manual CI/release dispatch while retaining push/PR builds and version-tag releases. Validate the tag/version and bind reusable CI, packaging and publishing to the same immutable commit SHA before release.

### Desktop authorization and architecture

- Bind window capture/describe to the authorized native identity and recheck read attribution/current policy before publishing; refuse native-ID reuse and mid-read allowlist revocation.
- Invalidate connection generations on disconnect/reconnect; reject cancelled connect completions and prevent older reads from returning data or reinstalling foreground refs after session changes.
- Refuse GNOME Shell keyboard/modal grabs, correct decorated frame/buffer capture and click geometry, preserve KDE fractional wheel coordinates, and invalidate stale native mappings/refs on explicit reconnect.
- Extract the canonical native helper library and shared prepared-action executor, removing mirrored standalone module/dependency wiring and broad lint suppression. Add behavioral host/native regressions and library-boundary checks.

- Add the KDE KWin 6.6 native plugin, explicit executable-path grants with persisted nonce consumption, pinned kernel/compositor identity, isolated window-item capture and guarded virtual input. Include reproducible source packaging and required KDE CI.
- Add real held-button disconnect regression; flush KWin pointer teardown and avoid forced GNOME device disposal while queued release events still reference the devices.

### Desktop capture and input

- Consume the canonical native GNOME Wayland adapter and package its explicit-install Aevra Desktop Access companion for Linux. Keep application allowlists, approved helper path, native focus/occlusion guards and lock/pause revocation.
- Add real private GNOME/GTK acceptance, Settings/doctor recovery guidance and companion setup instructions. KDE KWin 6.6 now has a native companion and software-rendered native acceptance; macOS runtime acceptance remains separate.

- Carry the allowlist-authorized process/window/executable/host identity into native foreground actions. Refuse focus changes before dispatch and stop further effects during interrupted typing or key sequences.
- Keep initialized Activity table geometry stable during automatic refresh and remove redundant clock-only row data.
- Synchronize targeted native guards, the 7.5-second input budget, typed action preparation, exact macOS AX focused-window binding and drift-safe overlay reconciliation from the canonical helper.

- Add macOS 13+ ScreenCaptureKit isolated-window capture and CoreGraphics text/key/mouse input, and Linux X11 Composite capture with XTest input. macOS native execution remains pending.
- Preserve process-instance attribution, unique native window binding, fresh ref validation, protected-field checks, pointer occlusion/focus guards and held-key cleanup. Validate complete chords and inject Unicode through the text transport.
- Add shared guard regressions, real private-X11 pixel/input tests, and an accessible GTK backend fixture proving exact Unicode text, Ctrl+A replacement, attributed capture and protected-field refusal. Include an explicit macOS fixture run.
- Carry the same canonical helper into standalone desktop using a reproducible, hash-checked overlay, including clean CI/release preparation, independent capture/input readiness and permission recovery guidance.
- Support Ubuntu X11 plus native GNOME Wayland and KDE KWin 6.6 companions. GNOME/KDE private-compositor acceptance runs in both consumers; native macOS, physical-GPU/OpenGL capture, actual lock-screen transitions and production grant workflows remain separate acceptance gates. Other Wayland compositors remain roadmap work.

### Initial functionality

First release. Initial backends from aevra v1.2.1, later replaced by the canonical native helper library (see Desktop capture and input).

- `desktop-core` compiles aevra's helper backends in-process and runs them on a dedicated
  actor thread.
- `desktop-policy` holds the allowlist (exe path, package family or bundle id), atomic config
  writes, the hard refusals for password, one-time-code and payment fields and secret-shaped
  values, and the value-free activity log.
- `desktop-mcp` serves seventeen `desktop_*` tools behind the allowlist gate, including
  `desktop_act_many` for batched semantic actions, over stdio and over authenticated loopback
  HTTP (bearer token, Host and Origin checks).
- `aevra-desktop` is a tray icon and settings window with installed-app discovery on Windows,
  macOS and Linux, hot reload of the allowlist, a single-instance lock, and the `mcp`,
  `doctor` and `--version` commands.
- Release archives for Windows, macOS and Linux on x64 and arm64, each with a SHA-256 file.
  macOS bundles are ad-hoc signed, not notarised.
- Platforms without a native capture/input adapter refuse those tools with
  `DESKTOP_CAPTURE_UNSUPPORTED` and `DESKTOP_INPUT_UNSUPPORTED`.
