# aevra-desktop

Current source release: **1.0.0**.

A native desktop-control MCP server. It runs the desktop backends from
[aevra](https://github.com/the-long-ride/aevra) in-process and exposes the `desktop_*` tools to
MCP clients over stdio and loopback HTTP. An allowlist that you manage in a tray icon and
settings window decides which apps the tools can reach. Nothing is reachable until you add it.

## Keep using your mouse while the agent works

Aevra Desktop supports mouse-free background automation: an agent can invoke buttons, set values, select items and toggle controls through the application's accessibility API while you use your mouse and keyboard elsewhere. This is useful when you want the agent to work in one application while you continue in another.

1. Allow the target application in **Apps**.
2. Have the agent call `desktop_describe` with `mode: "background"` and an explicit `windowId`.
3. Use the returned refs with `desktop_invoke`, `desktop_set_value`, `desktop_select` or `desktop_toggle`; refresh the describe after mutations or stale refs.

These semantic operations address the target controls directly and do not synthesize host mouse/keyboard events. Explicit foreground click/type/key/scroll tools use shared OS input and need a coordinated turn. An app may bring a window or dialog forward after an action; `focusChanged` reports that and suspends the lease until a fresh describe. Work on separate windows or fields when possible, since simultaneous edits to the same content can still conflict.

### Is this the same desktop engine as root Aevra?

Yes: both products consume the canonical helper implementation, including mouse-free semantic actions, native capture/input and target guards. Root Aevra runs the stdio helper behind gateway workspace/connector policy; this project hosts the library in-process and adds an app allowlist, native UI and Activity history. The overlay prepares the pinned vendor checkout from canonical source; it does not create a separate native backend implementation. Sessions and access settings belong to each host independently, so assign a shared target to one host at a time.

## Why Aevra Desktop?

Use this project for background desktop work alongside your own activity, with app access managed from a native window.

- **App access you can see:** discover applications, search the Apps list and choose which executables clients may reach. The allowlist applies to running clients as you edit it.
- **Semantic actions for background work:** UI Automation, macOS AX and Linux AT-SPI expose controls for invoke/set/select/toggle actions. Explicit screenshot and foreground-input tools handle workflows that need pixels or native events.
- **A native management interface:** Apps, Activity and Settings follow Aevra's visual style. Activity has search, outcome/app filters, sortable columns and pagination; unchanged refreshes preserve the table layout. HTTP state is visible in the header.
- **One native implementation across both products:** the canonical helper library supplies target attribution, protected-field refusal, guarded input and cleanup. The standalone host adds executable allowlisting and local call history.
- **Direct MCP setup:** use the native binary over stdio, or enable authenticated loopback HTTP from Settings. The tray/settings interface manages policy; headless `mcp` clients use the same configuration.

## How it differs from the other Aevra repositories

| Project                                                         | Focus                                                                               | Access and workflow                                                             |
| --------------------------------------------------------------- | ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| **Aevra Desktop — this repository**                             | Native application windows and accessibility controls                               | Application allowlist, native settings/tray UI and local Activity history       |
| [Aevra](https://github.com/the-long-ride/aevra)                 | Files, commands, Git, processes, browser/desktop tools and upstream MCP aggregation | Gateway Web UI, workspace/connector policy, approvals and hash-chained auditing |
| [Aevra Browser](https://github.com/the-long-ride/aevra-browser) | Browser tabs through an extension or CDP                                            | A shared local browser daemon and one-time extension pairing                    |

Choose Desktop for an application-focused setup. Choose the gateway for coordinated workspace automation and its approval/audit model. Choose Browser for browser-only automation. Desktop's local Activity history records calls without argument values; gateway auditing and per-connection grants belong to the root product.

## Recent changes in current source

Version **1.0.0** includes a custom GUI HTTP startup port with `--port` / `--p`.
See the [startup-port specification](docs/specs/2026-10-19-custom-startup-port.md)
and [1.0.0 changelog](CHANGELOG.md#100---2026-10-19) for its invocation-only scope,
Settings behavior and existing-instance recovery.

The native UI now groups controls with clearer borders and spacing, uses Aevra's canonical logo, and provides a responsive Activity toolbar plus author/source links. Foreground capture/input adapters cover Windows, macOS 13+, Ubuntu X11, GNOME Wayland and KDE KWin 6.6; consult the [platform table](#tools-and-platform-support) for runtime prerequisites and acceptance limits.

Window reads carry the original authorized identity and recheck the current allowlist before returning data. Disconnect/reconnect invalidates session generations, pending connect/read completions and stale refs/mappings. GNOME refuses Shell-owned keyboard input and handles decorated frame/buffer geometry; KDE wheel input accepts fractional native pointer positions.

The additional native/UI work above remains recorded under [1.0.0](CHANGELOG.md#100---2026-10-19), with current source version **1.0.0**. Native macOS, physical-GPU capture and production Wayland approval/lock acceptance remain open. The browser project's pinned drivers are released independently.

## Install

Download the archive for your platform from the
[releases page](https://github.com/the-long-ride/aevra-desktop-use/releases) and check it against the
`.sha256` file beside it.

| Platform                    | Archive                                                              | Contents                      |
| --------------------------- | -------------------------------------------------------------------- | ----------------------------- |
| Windows x64 / arm64         | `aevra-desktop-win32-x64.zip`, `aevra-desktop-win32-arm64.zip`       | `aevra-desktop.exe`           |
| macOS Apple silicon / Intel | `aevra-desktop-darwin-arm64.zip`, `aevra-desktop-darwin-x64.zip`     | `aevra-desktop.app`           |
| Linux x64 / arm64           | `aevra-desktop-linux-x64.tar.gz`, `aevra-desktop-linux-arm64.tar.gz` | binary, `.desktop` file, icon |

### Windows

Unzip anywhere and run `aevra-desktop.exe`. SmartScreen may warn about an unsigned download;
choose "More info" then "Run anyway". The tray icon appears in the notification area.

### macOS

1. Unzip and move `aevra-desktop.app` to `/Applications`.
2. The app is ad-hoc signed, not notarised, so Gatekeeper blocks the first launch. Clear the
   quarantine flag once:

   ```sh
   xattr -d com.apple.quarantine /Applications/aevra-desktop.app
   ```

3. Start the app, then grant it **Accessibility**: System Settings, Privacy & Security,
   Accessibility, switch `aevra-desktop` on. The permission belongs to the app bundle, so MCP
   clients must launch the binary inside it (see below). `aevra-desktop doctor` reports whether
   the permission is in place.

The app runs from the menu bar only and has no Dock icon.

### Linux

Unpack the archive and put `aevra-desktop` on your `PATH`. Optionally copy
`aevra-desktop.desktop` to `~/.local/share/applications/` and `aevra-desktop.png` to
`~/.local/share/icons/` for a launcher entry.

The backend reads other apps through AT-SPI, so the accessibility bus must be running
(package `at-spi2-core`; on GNOME it is on by default). Toolkits only publish their widgets when
accessibility is enabled for the session. The tray uses the StatusNotifierItem D-Bus service
(KDE, and GNOME with the AppIndicator extension). If no tray host exists the settings window
still opens, and `aevra-desktop mcp` never needs either.

## Allow apps

To choose the GUI's loopback HTTP port for one launch, run
`aevra-desktop --port=48900` (or `--p=48900`, `--port 48900`, `--p 48900`).
The value must contain only digits and be between 1 and 65535; pass the port
option only once. Enable HTTP in Settings to start listening. The selected port
appears in Settings and its HTTP client snippet, survives configuration reloads,
and stays separate from the saved port when you save other settings. While an
override is active, the saved port field is read-only. Quit a running GUI from
the tray before relaunching with a port option; an existing GUI cannot adopt it.
The window wake-up socket still chooses its own free port. The `mcp` command
uses stdio and does not accept a port option.
See the [implemented startup-port specification](docs/specs/2026-10-19-custom-startup-port.md)
for exact validation, defaults and reload behavior.

Run `aevra-desktop`. The native window follows Aevra web UI colors, monospace typography,
and compact controls. Use **Light theme** / **Dark theme** in the header to switch appearance.
The header uses the canonical Aevra logo and a green HTTP dot while listening, red when off
or failed (hover for details). The underlined footer links open the author GitHub profile and the aevra-desktop-use source repository.
The header shows the HTTP listener state; save feedback stays visible below the scrolling content.
Related controls are grouped with thin borders and extra padding. Settings sections have
larger gaps, and each app has its own outlined row for easier scanning.
The window has three tabs:

- **Apps** lists installed apps, with allowed, running and custom indicators. Search by name
  or use **Rescan apps** to refresh discovery. Tick the ones MCP clients may use. **Add custom...** allows a
  program that is not in the list, by path.
- **Activity** searches all retained calls across the current log and both rotated logs. Search
  tool, app or result text; filter by outcome or app; click a column heading to sort or reverse
  its order. Choose 25, 50 or 100 rows per page and use Previous/Next. Dates and times use UTC.
  Search/filter/sort changes return to page one; history refreshes in the background without
  replacing unchanged results or flashing a refresh label. Search and filters share one row
  in wide windows and use two rows at narrower widths. Argument
  values and typed text are never recorded.
- **Settings** has the HTTP server switch, the port and the token, plus config snippets to copy.

The allowlist is the file below. Both `aevra-desktop mcp` and the HTTP server re-read it when it
changes, so edits in the window apply to running clients at once.

| Platform | Config file                                                         |
| -------- | ------------------------------------------------------------------- |
| Windows  | `%APPDATA%\aevra-desktop\config.json`                               |
| macOS    | `~/Library/Application Support/aevra-desktop/config.json`           |
| Linux    | `$XDG_CONFIG_HOME/aevra-desktop/config.json` (default `~/.config/`) |

Always refused, whatever the allowlist says: password, one-time-code and payment fields, values
that look like card numbers or credentials, the secure desktop and elevated windows, and
aevra-desktop's own window. Windows of apps that are not allowed are not listed.

## Connect an MCP client

### stdio

The client launches `aevra-desktop mcp`. Use the path of the binary you installed:

```json
{
  "mcpServers": {
    "aevra-desktop": {
      "command": "<path to aevra-desktop>",
      "args": ["mcp"]
    }
  }
}
```

On macOS the command is `/Applications/aevra-desktop.app/Contents/MacOS/aevra-desktop`.

### HTTP

Turn on the HTTP server in **Settings**. The switch saves immediately; use **Apply port**
when changing the port. It binds to `127.0.0.1` only,
refuses any `Host` or `Origin` that is not loopback, and requires the token on every request
except `GET /healthz`.

```json
{
  "mcpServers": {
    "aevra-desktop": {
      "type": "http",
      "url": "http://127.0.0.1:47851/mcp",
      "headers": { "Authorization": "Bearer <token from Settings>" }
    }
  }
}
```

The server is off by default. The token is generated on first run; **Regenerate** replaces it,
and clients that still send the old one get 401. Treat it like a password: anyone who can reach
loopback with the token can drive your allowed apps.

## Tools and platform support

Seventeen tools, with the names and input schemas of aevra's desktop tools, minus
`desktop_request_access` (the allowlist replaces per-window access requests).

| Tool group                                      | Windows | macOS 13+                                                     | Linux                                               |
| ----------------------------------------------- | ------- | ------------------------------------------------------------- | --------------------------------------------------- |
| Session, app/window discovery, describe         | yes     | AX permission                                                 | AT-SPI provider                                     |
| Semantic actions, leases and `desktop_act_many` | yes     | AX                                                            | AT-SPI                                              |
| `desktop_capture`                               | yes     | ScreenCaptureKit; Screen Recording; native acceptance pending | Ubuntu X11 Composite; GNOME and KWin 6.6 companions |
| Foreground click/type/key/scroll                | yes     | CoreGraphics; Accessibility; native acceptance pending        | Ubuntu X11 XTest; GNOME and KWin 6.6 companions     |

Capture and input readiness are probed separately. `doctor` reports recovery steps, and missing runtime permission returns `DESKTOP_PERMISSION_REQUIRED`. All operations retain the app allowlist; whole-desktop capture remains refused. **Ubuntu X11, GNOME Wayland and KDE KWin 6.6 have native adapters.** GNOME needs the bundled Aevra Desktop Access companion: install/enable its extension, enable access, connect once, approve the actual helper path in its panel menu and reconnect. Lock/pause revokes access. Native GNOME 50/Ubuntu 26.04 fixtures run through both repositories; GNOME 46 CI is configured. KDE KWin 6.6 now has a native companion built against the exact compositor version; see the [roadmap](docs/ROADMAP.md) and [Wayland setup](https://github.com/the-long-ride/aevra/blob/main/docs/user-manual/22-wayland-desktop.md). XWayland and privileged uinput fallback remain refused.

Background actions go through the platform accessibility API (UI Automation, AX, AT-SPI) and do
not move the mouse or type into whatever has focus. An app may still take focus when a control
is invoked; the result says so (`focusChanged`) and the lease is suspended until the next
`desktop_describe`.

Window and element text returned by the tools is marked as untrusted data.

## Commands

```text
aevra-desktop                  open the tray icon and the allowlist window
aevra-desktop mcp              serve MCP over stdio
aevra-desktop doctor [--json]  check the platform, config and HTTP port
aevra-desktop --version
```

## Build from source

The source uses the aevra v1.2.1 submodule plus a reviewed canonical helper overlay exported from the sibling Aevra repository. Run `node scripts/prepare-helper.mjs` after initializing submodules. The overlay is hash-checked, idempotent, and refuses local edits; it does not change the pinned base revision.

```sh
git clone --recurse-submodules https://github.com/the-long-ride/aevra-desktop-use aevra-desktop
cd aevra-desktop
node scripts/prepare-helper.mjs
cargo build --release
```

Needs a recent stable Rust (1.95 or newer) and Node.js 24 for `scripts/check-pins.mjs`, which
checks that `crates/desktop-core` consumes the prepared canonical helper library without mirroring native platform dependencies. Linux builds need `libwayland-dev` and `libxkbcommon-dev`; native input needs the X11/XTest session and capture needs a compositing window manager. The native Windows test builds aevra's WinForms fixture and needs
the .NET 8 SDK; it skips itself without one.

See `docs/specs/` for the design.

## Native capture/input verification

`cargo test --workspace --locked` covers shared contracts, handlers and allowlist refusals. The explicitly ignored native tests are separate acceptance runs, and fail when their required prerequisites are missing.

On Linux install Xvfb, x11-xkb-utils, python3-gi, gir1.2-gtk-3.0, at-spi2-core and dbus-x11. Then run:

```sh
AEVRA_GTK_FIXTURE="$PWD/vendor/aevra/helper/tests/fixtures/portable_gtk_fixture.py" \
dbus-run-session -- cargo test -p aevra-desktop-helper --locked native_x11_ -- --ignored --nocapture --test-threads=1
```

The tests create private displays. They check capture beneath an occluding sentinel, actual mouse/key/wheel events, and the real AX/AT-SPI backend against an accessible GTK fixture: ref clicks, exact Unicode text, Ctrl+A replacement, attributed JPEG capture, and protected-field refusal. The same canonical tests execute in root Aevra.

On a Mac, build `vendor/aevra/helper/tests/fixtures/PortableMacFixture.swift` using `swiftc -framework AppKit`, grant the test host Accessibility and Screen Recording, and set `AEVRA_MAC_FIXTURE` to that binary. Run `cargo test -p aevra-desktop-helper --locked native_macos_capture_and_input -- --ignored --nocapture`. This fixture checks actual pixels, an unnamed/unshareable occluder, Unicode text, a Command chord, clicks and scrolling. Native macOS execution is still pending; build matrices alone do not establish parity.

To refresh the source overlay during coordinated development, run `python helper/export-desktop-overlay.py ../aevra-desktop --sync` from the canonical Aevra checkout. It updates only the previous exact generated overlay and aborts on local drift. Until upstream source can be pinned, commit the reviewed patch and manifest together with the standalone integration changes.

The native helper is a shared library; standalone keeps only its actor/error facade in `desktop-core`. Window reads carry the host-authorized identity through native collection and recheck current app policy before returning data. Disconnect/reconnect invalidates the session generation, old refs and native mappings; cancelled connect completions and in-flight reads from older sessions cannot publish. Observe again after recovery. Decorated GNOME geometry and Shell-owned keyboard focus are checked explicitly, and KDE scrolling accepts valid fractional native positions.

## License

[MIT](LICENSE).

Native macOS builds require installed Xcode 26+ for the current Metal bridge dependency, even when targeting older supported macOS releases. CI runs the canonical `vendor/aevra/helper/setup-macos.mjs` prerequisite to select and verify the SDK. The repository Cargo config embeds `/usr/lib/swift` as a runtime search path for test and release binaries.

Run the dedicated Windows acceptance with `pwsh -File scripts/test-native-windows.ps1`. It compiles the Rust test and the .NET fixture with the runner account into fresh build output, then passes the prepared fixture directory to the test in the same desktop session. The temporary standard account needs the .NET runtime but does not restore packages or compile. Compiler diagnostics remain visible, and preparation failures stop the required run before account creation. Elevated GitHub runners use a temporary standard-user logon, verified before the suspended child is resumed; account and scoped desktop/build-folder grants are removed afterward. Non-elevated local runs use the existing restricted-token path. Before either path, the launcher opens and reads the actual desktop/window-station ACLs with the required desktop object rights; this probe does not change permissions. Missing fixture/.NET prerequisites fail the required run. Production control still refuses elevated helpers and targets. Keep `.node-version` and `.cargo/config.toml` in the source checkout; CI regression tests depend on these files.
