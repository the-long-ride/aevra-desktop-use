//! The allowlist gate: decides whether a window may be listed, described or
//! acted on, from its verified identity alone.
//!
//! Ported from aevra's `packages/security/src/window-gate.ts`, narrowed to the
//! allowlist case. The rules fail closed: an identity the OS would not give us
//! is never allowed.

use crate::config::{AllowedApp, AppIdentity, Config};
use desktop_core::{VerifiedWindowHost, WindowIdentity, WindowInstance};

/// Which path conventions to apply. Production code always uses
/// [`Flavor::native`]; tests exercise both on any host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Windows,
    Unix,
}

impl Flavor {
    pub const fn native() -> Flavor {
        if cfg!(windows) {
            Flavor::Windows
        } else {
            Flavor::Unix
        }
    }
}

/// Port of aevra's `canonicalExecutablePath`, for the native flavor.
pub fn canonical_path(p: &str) -> String {
    canonical_path_with(p, Flavor::native())
}

/// Windows: trim, `/` to `\`, `\\?\UNC\` to `\\`, strip `\\?\`, strip trailing
/// `\`, lowercase. Unix: trim, strip trailing `/`, case-sensitive.
pub fn canonical_path_with(p: &str, flavor: Flavor) -> String {
    match flavor {
        Flavor::Unix => p.trim().trim_end_matches('/').to_string(),
        Flavor::Windows => {
            let mut s = p.trim().replace('/', "\\");
            let lower = s.to_lowercase();
            if lower.starts_with("\\\\?\\unc\\") {
                s = format!("\\\\{}", &s[8..]);
            } else if lower.starts_with("\\\\?\\") {
                s = s[4..].to_string();
            }
            s.trim_end_matches('\\').to_lowercase()
        }
    }
}

fn separator(flavor: Flavor) -> char {
    match flavor {
        Flavor::Windows => '\\',
        Flavor::Unix => '/',
    }
}

/// The final path segment of an already canonical path.
fn basename(canonical: &str, flavor: Flavor) -> &str {
    canonical
        .rsplit(separator(flavor))
        .next()
        .unwrap_or_default()
}

/// A canonical path that still names something: not empty and with a non-empty
/// final segment. `\\`, `/` and the empty string all canonicalise to nothing.
fn usable(canonical: &str, flavor: Flavor) -> bool {
    !canonical.is_empty() && !basename(canonical, flavor).is_empty()
}

pub struct Target<'a> {
    pub window: &'a WindowIdentity,
    pub instance: Option<&'a WindowInstance>,
    pub host: Option<&'a VerifiedWindowHost>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow { app_id: String },
    Deny { code: &'static str, reason: String },
}

impl Verdict {
    pub fn is_allow(&self) -> bool {
        matches!(self, Verdict::Allow { .. })
    }
}

const NOT_ALLOWED: &str = "DESKTOP_APP_NOT_ALLOWED";
const REFUSED: &str = "DESKTOP_INPUT_REFUSED";

/// Evaluates `t` against the allowlist, with the native path flavor.
pub fn evaluate(cfg: &Config, t: &Target, self_pid: u32) -> Verdict {
    evaluate_with(cfg, t, self_pid, Flavor::native())
}

/// Rules, in order:
/// 1. A window of this very process is refused, allowlisted or not.
/// 2. A WebView2 target is judged by its verified host executable. With no
///    verified host it is refused, never allowed because `msedgewebview2.exe`
///    happens to be listed.
/// 3. No usable executable path means no identity, so refuse.
/// 4. The first allowed app that matches wins.
/// 5. Otherwise refuse.
pub fn evaluate_with(cfg: &Config, t: &Target, self_pid: u32, flavor: Flavor) -> Verdict {
    if t.instance.is_some_and(|i| i.process_id == self_pid) {
        return Verdict::Deny {
            code: REFUSED,
            reason: "aevra-desktop does not control its own window".to_string(),
        };
    }

    let target_exe = t
        .window
        .executable_path
        .as_deref()
        .map(|p| canonical_path_with(p, flavor))
        .filter(|p| usable(p, flavor));

    let effective = match target_exe {
        Some(exe) if basename(&exe, flavor) == "msedgewebview2.exe" => {
            let host_exe = t
                .host
                .map(|h| canonical_path_with(&h.executable_path, flavor))
                .filter(|p| usable(p, flavor))
                .filter(|p| basename(p, flavor) != "msedgewebview2.exe");
            match host_exe {
                Some(host_exe) => host_exe,
                None => {
                    return Verdict::Deny {
                        code: NOT_ALLOWED,
                        reason: "WebView2 host could not be verified".to_string(),
                    }
                }
            }
        }
        Some(exe) => exe,
        None => {
            return Verdict::Deny {
                code: NOT_ALLOWED,
                reason: "window identity is unavailable".to_string(),
            }
        }
    };

    match cfg
        .allowed_apps
        .iter()
        .find(|app| app_matches(app, &effective, flavor))
    {
        Some(app) => Verdict::Allow {
            app_id: app.id.clone(),
        },
        None => Verdict::Deny {
            code: NOT_ALLOWED,
            reason: format!("{} is not in the allowlist", basename(&effective, flavor)),
        },
    }
}

fn app_matches(app: &AllowedApp, exe: &str, flavor: Flavor) -> bool {
    identity_matches(&app.identity, exe, flavor)
}

fn identity_matches(identity: &AppIdentity, exe: &str, flavor: Flavor) -> bool {
    match identity {
        AppIdentity::Exe { path } => {
            let wanted = canonical_path_with(path, flavor);
            usable(&wanted, flavor) && wanted == exe
        }
        AppIdentity::Bundle { path } => {
            let bundle = canonical_path_with(path, flavor);
            if !usable(&bundle, flavor) {
                return false;
            }
            let sep = separator(flavor);
            let prefix = match flavor {
                Flavor::Unix => format!("{bundle}{sep}Contents{sep}"),
                Flavor::Windows => format!("{bundle}{sep}contents{sep}"),
            };
            exe.starts_with(&prefix)
        }
        AppIdentity::Package { family_name } => {
            flavor == Flavor::Windows && package_matches(family_name, exe)
        }
    }
}

/// Whether a process running `exe_path` belongs to `identity`, by the same
/// rules the gate uses. For showing which allowed or installed apps are running.
pub fn identity_matches_path(identity: &AppIdentity, exe_path: &str) -> bool {
    let flavor = Flavor::native();
    identity_matches(identity, &canonical_path_with(exe_path, flavor), flavor)
}

/// Matches a Store package by family name against an executable under
/// `...\WindowsApps\<package full name>\...`.
///
/// A family name is `<Name>_<PublisherId>` and the install directory is
/// `<Name>_<Version>_<Arch>_<ResourceId>_<PublisherId>`. The directory must
/// start with `<name>_` (so `Foo` never matches `FooBar_...`) and end with
/// `_<publisherid>`.
fn package_matches(family_name: &str, canonical_exe: &str) -> bool {
    let family = family_name.trim().to_lowercase();
    if family.is_empty() {
        return false;
    }
    let (name, publisher) = match family.rsplit_once('_') {
        Some((name, publisher)) if !name.is_empty() && !publisher.is_empty() => {
            (name, Some(publisher))
        }
        _ => (family.as_str(), None),
    };
    let mut segments = canonical_exe.split('\\');
    if !segments.any(|s| s == "windowsapps") {
        return false;
    }
    let Some(dir) = segments.next() else {
        return false;
    };
    if !dir.starts_with(&format!("{name}_")) {
        return false;
    }
    match publisher {
        Some(publisher) => dir.ends_with(&format!("_{publisher}")),
        None => true,
    }
}

/// Whether a window may appear in `desktop_windows`. Applies the same rules as
/// [`evaluate`] with no instance or host, so a WebView2 window is hidden
/// because its host cannot be verified here.
pub fn window_visible(cfg: &Config, w: &WindowIdentity, self_pid: u32) -> bool {
    window_visible_with(cfg, w, self_pid, Flavor::native())
}

pub fn window_visible_with(
    cfg: &Config,
    w: &WindowIdentity,
    self_pid: u32,
    flavor: Flavor,
) -> bool {
    evaluate_with(
        cfg,
        &Target {
            window: w,
            instance: None,
            host: None,
        },
        self_pid,
        flavor,
    )
    .is_allow()
}
