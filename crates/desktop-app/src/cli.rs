//! Command line parsing for GUI startup and the stdio/diagnostic commands.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Tray icon and allowlist window, with an invocation-only HTTP port.
    Run {
        port: Option<u16>,
    },
    /// Serve MCP over stdio, hot-reloading the allowlist.
    Mcp,
    Doctor {
        json: bool,
    },
    Version,
    Help,
}

pub const USAGE: &str = "\
aevra-desktop: let MCP clients drive the desktop apps you allow.

USAGE:
    aevra-desktop [--port PORT]    open the tray icon and the allowlist window
    aevra-desktop mcp              serve MCP over stdio (for MCP clients to launch)
    aevra-desktop doctor [--json]  check the platform, config and HTTP port
    aevra-desktop --version
    aevra-desktop --help

GUI STARTUP OPTIONS:
    --port PORT, --p PORT          HTTP listener port, digits only, 1..65535
    --port=PORT, --p=PORT          equivalent forms; only one port option allowed
    The override lasts for this invocation and HTTP must still be enabled in
    Settings. Quit any running GUI first. The wake-up socket stays automatic.
    The stdio mcp command does not bind HTTP and accepts no port option.

GNOME WAYLAND:
    Install the bundled aevra-desktop@the-long-ride.shell-extension.zip with
    gnome-extensions install, enable the extension, then allow access from its
    panel menu and approve the helper path after connecting. Reconnect after
    approval. Locking or pausing revokes access. Use doctor to check readiness.

KDE WAYLAND (KWin 6.6):
    Build the bundled aevra-wayland-kde-source.zip against your exact KWin version,
    explicitly enable the native plugin and create the private helper-path grant
    described in its README. Lock/restart needs a fresh grant and reconnect.
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(pub String);

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error: {}\n\n{USAGE}", self.0)
    }
}

impl std::error::Error for CliError {}

/// Parses the arguments after the program name.
pub fn parse<I, S>(args: I) -> Result<Command, CliError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().map(|a| a.as_ref().to_string()).collect();
    let mut words = args.iter().map(String::as_str);
    let Some(first) = words.next() else {
        return Ok(Command::Run { port: None });
    };
    let rest: Vec<&str> = words.collect();
    let no_extra = |command: Command| {
        if rest.is_empty() {
            Ok(command)
        } else {
            Err(CliError(format!(
                "unexpected argument after {first}: {}",
                rest.join(" ")
            )))
        }
    };
    match first {
        "--port" | "--p" => parse_startup_port(&args),
        value if value.starts_with("--port=") || value.starts_with("--p=") => {
            parse_startup_port(&args)
        }
        "mcp" => no_extra(Command::Mcp),
        "doctor" => match rest.as_slice() {
            [] => Ok(Command::Doctor { json: false }),
            ["--json"] => Ok(Command::Doctor { json: true }),
            other => Err(CliError(format!(
                "unexpected argument after doctor: {}",
                other.join(" ")
            ))),
        },
        "--version" | "-V" => no_extra(Command::Version),
        "--help" | "-h" | "help" => no_extra(Command::Help),
        other => Err(CliError(format!("unknown command: {other}"))),
    }
}

fn parse_startup_port(args: &[String]) -> Result<Command, CliError> {
    let (flag, inline) = args[0]
        .split_once('=')
        .map_or((args[0].as_str(), None), |(flag, value)| {
            (flag, Some(value))
        });
    let value = inline.or_else(|| args.get(1).map(String::as_str));
    let Some(value) = value else {
        return Err(CliError(format!("{flag} requires a port number")));
    };
    if args.len() != if inline.is_some() { 1 } else { 2 } {
        return Err(CliError(
            "only one startup port option is allowed; remove duplicate options or extra arguments"
                .into(),
        ));
    }
    let port = value
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .filter(|_| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| CliError(format!("{flag} must be digits only, between 1 and 65535")))?;
    Ok(Command::Run { port: Some(port) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_start_the_tray() {
        assert_eq!(parse(Vec::<String>::new()), Ok(Command::Run { port: None }));
    }

    #[test]
    fn gui_accepts_both_port_aliases_with_equals_or_separate_values() {
        for (args, port) in [
            (vec!["--port=1"], 1),
            (vec!["--p=65535"], 65535),
            (vec!["--port", "48900"], 48900),
            (vec!["--p", "0048900"], 48900),
        ] {
            assert_eq!(
                parse(&args),
                Ok(Command::Run { port: Some(port) }),
                "{args:?}"
            );
        }
    }

    #[test]
    fn invalid_ports_and_duplicate_aliases_are_rejected() {
        for value in [
            "",
            "zero",
            "1.5",
            "-1",
            "0",
            "65536",
            "99999999999999999999",
            "+1",
            " 1",
            "1 ",
        ] {
            assert!(parse([format!("--port={value}")]).is_err(), "{value:?}");
            assert!(parse(["--p", value]).is_err(), "{value:?}");
        }
        for args in [
            vec!["--port"],
            vec!["--p"],
            vec!["--port=48900", "--port=48901"],
            vec!["--p", "48900", "--port=48900"],
            vec!["--p=48900", "--port", "48901"],
            vec!["mcp", "--port=48900"],
            vec!["doctor", "--p=48900"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn the_known_commands_parse() {
        assert_eq!(parse(["mcp"]), Ok(Command::Mcp));
        assert_eq!(parse(["doctor"]), Ok(Command::Doctor { json: false }));
        assert_eq!(
            parse(["doctor", "--json"]),
            Ok(Command::Doctor { json: true })
        );
        assert_eq!(parse(["--version"]), Ok(Command::Version));
        assert_eq!(parse(["-V"]), Ok(Command::Version));
        assert_eq!(parse(["--help"]), Ok(Command::Help));
        assert_eq!(parse(["-h"]), Ok(Command::Help));
    }

    #[test]
    fn unknown_commands_and_extra_arguments_are_errors_with_usage() {
        let err = parse(["frobnicate"]).unwrap_err();
        assert!(err.0.contains("frobnicate"));
        let shown = err.to_string();
        assert!(shown.contains("USAGE"));
        assert!(shown.contains("aevra-desktop mcp"));

        assert!(parse(["mcp", "--verbose"]).is_err());
        assert!(parse(["doctor", "--yaml"]).is_err());
        assert!(parse(["doctor", "--json", "extra"]).is_err());
        assert!(parse(["--version", "now"]).is_err());
    }
}
