//! Command line parsing. Hand-rolled: there are four commands and two flags.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// No arguments: tray icon and allowlist window.
    Run,
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
    aevra-desktop                  open the tray icon and the allowlist window
    aevra-desktop mcp              serve MCP over stdio (for MCP clients to launch)
    aevra-desktop doctor [--json]  check the platform, config and HTTP port
    aevra-desktop --version
    aevra-desktop --help

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
        return Ok(Command::Run);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_start_the_tray() {
        assert_eq!(parse(Vec::<String>::new()), Ok(Command::Run));
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
