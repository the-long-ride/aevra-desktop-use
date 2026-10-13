# Custom HTTP startup port

Status: implemented in source version **1.0.0**.
Date: **2026-10-19**, Asia/Saigon.

## Scope and syntax

The default GUI mode opens the tray/allowlist window and, when enabled in
Settings, serves MCP on loopback HTTP. It accepts these equivalent startup forms:

```sh
aevra-desktop --port=48900
aevra-desktop --p=48900
aevra-desktop --port 48900
aevra-desktop --p 48900
```

`--port` and `--p` are double-hyphen aliases for one logical option. Supply it
at most once; repeating either alias, including mixed aliases and equal values,
is an error. There is no explicit `run` subcommand or single-hyphen `-p` alias.

Without the option, the GUI uses `http.port` from its saved configuration
(default **47851**). HTTP remains disabled by default. The `mcp` command serves
stdio without binding HTTP and accepts no port flag. Diagnostic `doctor`, help
and version commands retain their existing command-specific arguments.

## Validation and failures

- Accept nonempty ASCII digits representing **1..65535**, including leading zeros.
- Reject missing/empty values, nonnumeric text, fractional values, negative or
  explicitly signed values, surrounding whitespace, zero, overflow and extra
  arguments. Parsing errors print usage and exit with status **2** before startup
  creates configuration or opens listeners.
- If a GUI already owns the user configuration directory, a custom-port launch
  exits with status **1** and explains that the user must quit the running GUI
  from its tray and relaunch with the requested port. It does not silently accept
  an ignored override. Launching again without an override retains the existing
  show-window behavior.
- If HTTP is enabled but the requested port is occupied, the HTTP controller
  reports `HTTP_PORT_IN_USE` in Settings. It does not choose a different HTTP port.

## Invocation override and saved settings

The selected port applies only to this GUI process. It does not enable HTTP and
is never written into the persisted `http.port` by the startup override. The
Settings checkbox and tray HTTP toggle continue to enable or disable the
listener. Saving token, allowlist or other configuration edits preserves the
saved port independently of the invocation override.

The fixed override applies both at startup and after valid configuration reloads.
Edits to the saved HTTP port do not change the effective port while the override
is active. Token rotation still restarts the authenticated listener, and HTTP
enable/disable still starts or stops it. The running MCP server's effective
configuration, HTTP listener, Settings port display and copied HTTP client URL
all use the same invocation-selected port.

Settings displays the effective port and makes its port field and Apply button
read-only while an override is active. An explanation also shows the saved port
and tells the user to quit/relaunch without the flag to edit that saved value.
Relaunching without the option restores normal saved-port behavior.

The single-instance window wake-up socket remains independent: it binds a free
OS-assigned loopback port and records that port in `ui.port`. The HTTP startup
option does not choose or persist the wake-up port. Loopback-only HTTP binding,
bearer-token authorization and existing Host/Origin checks remain in force.

## Verification and boundaries

Parser tests cover both aliases/forms, boundaries, malformed values, duplicate
aliases and rejection on stdio/diagnostic commands. Executable tests prove invalid
input fails before configuration creation and that an existing GUI reports an
actionable custom-port error. Controller tests exercise a real listener,
effective server configuration, config edits, token restart, saved settings and
HTTP snippets.

An explicitly invoked native GUI test proves the selected HTTP port at startup,
disable/re-enable, token/config-port reload, unchanged saved port and a distinct
wake-up socket. It is ignored in the default headless test suite because it opens
a native window; run it with:

```sh
cargo test -p desktop-app --test startup_port -- --include-ignored --nocapture
```

The feature does not add a background-service installer or change native desktop
backend authorization. Independent macOS, GPU/Wayland and native backend
acceptance limits remain documented in the [roadmap](../ROADMAP.md) and
[README](../../README.md).
