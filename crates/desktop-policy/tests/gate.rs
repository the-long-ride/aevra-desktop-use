use desktop_core::{VerifiedWindowHost, WindowIdentity, WindowInstance};
use desktop_policy::gate::{
    canonical_path_with, evaluate, evaluate_with, window_visible_with, Flavor, Target, Verdict,
};
use desktop_policy::{AllowedApp, AppIdentity, Config};

const SELF_PID: u32 = 4242;

fn cfg(apps: Vec<AllowedApp>) -> Config {
    Config {
        allowed_apps: apps,
        ..Config::default()
    }
}

fn exe_app(id: &str, path: &str) -> AllowedApp {
    AllowedApp {
        id: id.to_string(),
        name: id.to_string(),
        identity: AppIdentity::Exe {
            path: path.to_string(),
        },
    }
}

fn window(path: Option<&str>) -> WindowIdentity {
    WindowIdentity {
        window_id: "100".to_string(),
        process_name: path.map(|p| p.rsplit(['\\', '/']).next().unwrap_or("").to_string()),
        executable_path: path.map(str::to_string),
        title: None,
    }
}

fn instance(pid: u32) -> WindowInstance {
    WindowInstance::new("100", pid, "started")
}

fn verdict(cfg: &Config, w: &WindowIdentity, flavor: Flavor) -> Verdict {
    evaluate_with(
        cfg,
        &Target {
            window: w,
            instance: Some(&instance(10)),
            host: None,
        },
        SELF_PID,
        flavor,
    )
}

const NOTEPAD: &str = r"C:\Windows\System32\notepad.exe";

#[test]
fn allowlisted_exe_is_permitted() {
    let c = cfg(vec![exe_app("app_1", NOTEPAD)]);
    assert_eq!(
        verdict(&c, &window(Some(NOTEPAD)), Flavor::Windows),
        Verdict::Allow {
            app_id: "app_1".into()
        }
    );
}

#[test]
fn exe_match_is_case_insensitive_on_windows() {
    let c = cfg(vec![exe_app("app_1", NOTEPAD)]);
    let shouting = window(Some(r"C:\WINDOWS\system32\NOTEPAD.EXE"));
    assert!(verdict(&c, &shouting, Flavor::Windows).is_allow());
    // Unix paths are case-sensitive.
    let unix = cfg(vec![exe_app("app_1", "/usr/bin/gedit")]);
    assert!(!verdict(&unix, &window(Some("/usr/bin/GEDIT")), Flavor::Unix).is_allow());
    assert!(verdict(&unix, &window(Some("/usr/bin/gedit")), Flavor::Unix).is_allow());
}

#[test]
fn native_flavor_follows_the_host_os() {
    let c = cfg(vec![exe_app("app_1", NOTEPAD)]);
    let shouting = window(Some(r"C:\WINDOWS\system32\NOTEPAD.EXE"));
    let v = evaluate(
        &c,
        &Target {
            window: &shouting,
            instance: None,
            host: None,
        },
        SELF_PID,
    );
    #[cfg(windows)]
    assert!(v.is_allow());
    #[cfg(not(windows))]
    assert!(!v.is_allow());
}

#[test]
fn trailing_separator_does_not_defeat_matching() {
    // FINDING A
    let c = cfg(vec![exe_app("app_1", NOTEPAD)]);
    let slashed = window(Some(r"C:\Windows\System32\notepad.exe\"));
    assert!(verdict(&c, &slashed, Flavor::Windows).is_allow());
    let c = cfg(vec![exe_app("app_1", r"C:\Windows\System32\notepad.exe\")]);
    assert!(verdict(&c, &window(Some(NOTEPAD)), Flavor::Windows).is_allow());
    let unix = cfg(vec![exe_app("app_1", "/usr/bin/gedit")]);
    assert!(verdict(&unix, &window(Some("/usr/bin/gedit/")), Flavor::Unix).is_allow());
}

#[test]
fn empty_executable_path_is_unavailable_and_refused() {
    let c = cfg(vec![exe_app("app_1", NOTEPAD)]);
    for w in [window(None), window(Some("")), window(Some("   "))] {
        match verdict(&c, &w, Flavor::Windows) {
            Verdict::Deny { code, reason } => {
                assert_eq!(code, "DESKTOP_APP_NOT_ALLOWED");
                assert_eq!(reason, "window identity is unavailable");
            }
            other => panic!("expected deny, got {other:?}"),
        }
    }
}

#[test]
fn separators_only_path_is_unavailable() {
    // M2/M3
    let c = cfg(vec![exe_app("app_1", NOTEPAD), exe_app("app_2", "/")]);
    // A backslash is an ordinary file name character on unix, so only `/` is
    // separators-only there.
    let cases = [
        ("\\", Flavor::Windows),
        ("\\\\", Flavor::Windows),
        ("/", Flavor::Windows),
        ("/", Flavor::Unix),
        ("//", Flavor::Unix),
    ];
    for (path, flavor) in cases {
        match verdict(&c, &window(Some(path)), flavor) {
            Verdict::Deny { reason, .. } => {
                assert_eq!(reason, "window identity is unavailable", "{path:?}")
            }
            other => panic!("expected deny for {path:?}, got {other:?}"),
        }
    }
}

#[test]
fn verbatim_and_unc_prefixes_canonicalise() {
    assert_eq!(
        canonical_path_with(r"\\?\C:\Tools\App.EXE", Flavor::Windows),
        r"c:\tools\app.exe"
    );
    assert_eq!(
        canonical_path_with(r"\\?\UNC\Server\Share\App.exe", Flavor::Windows),
        r"\\server\share\app.exe"
    );
    assert_eq!(
        canonical_path_with("C:/Tools/App.exe/", Flavor::Windows),
        r"c:\tools\app.exe"
    );
    let c = cfg(vec![exe_app("app_1", r"C:\Tools\App.exe")]);
    assert!(verdict(&c, &window(Some(r"\\?\C:\Tools\App.exe")), Flavor::Windows).is_allow());
}

#[test]
fn unix_canonicalisation_trims_and_strips_trailing_slash() {
    assert_eq!(
        canonical_path_with("  /Usr/Bin/Gedit// ", Flavor::Unix),
        "/Usr/Bin/Gedit"
    );
}

const WEBVIEW: &str = r"C:\Program Files\Microsoft\EdgeWebView\Application\msedgewebview2.exe";
const HOST_EXE: &str = r"C:\Apps\Quota\quota.exe";

fn host() -> VerifiedWindowHost {
    VerifiedWindowHost {
        executable_path: HOST_EXE.to_string(),
        instance: WindowInstance::new("200", 20, "host-started"),
    }
}

#[test]
fn webview2_target_is_judged_by_its_verified_host() {
    let w = window(Some(WEBVIEW));
    let h = host();
    let target = Target {
        window: &w,
        instance: Some(&instance(10)),
        host: Some(&h),
    };
    // Host allowed, runtime not listed: allowed.
    let c = cfg(vec![exe_app("host", HOST_EXE)]);
    assert_eq!(
        evaluate_with(&c, &target, SELF_PID, Flavor::Windows),
        Verdict::Allow {
            app_id: "host".into()
        }
    );
    // Runtime listed, host not: refused. The runtime's own listing never counts.
    let c = cfg(vec![exe_app("runtime", WEBVIEW)]);
    assert!(!evaluate_with(&c, &target, SELF_PID, Flavor::Windows).is_allow());
}

#[test]
fn unverified_webview2_is_refused() {
    let w = window(Some(WEBVIEW));
    let c = cfg(vec![exe_app("runtime", WEBVIEW)]);
    let v = verdict(&c, &w, Flavor::Windows);
    assert_eq!(
        v,
        Verdict::Deny {
            code: "DESKTOP_APP_NOT_ALLOWED",
            reason: "WebView2 host could not be verified".into()
        }
    );
    assert!(!window_visible_with(&c, &w, SELF_PID, Flavor::Windows));
}

#[test]
fn own_process_is_refused_even_if_allowlisted() {
    // Review Focus 2
    let me = r"C:\Apps\aevra-desktop\aevra-desktop.exe";
    let c = cfg(vec![exe_app("self", me)]);
    let w = window(Some(me));
    let own = instance(SELF_PID);
    let v = evaluate_with(
        &c,
        &Target {
            window: &w,
            instance: Some(&own),
            host: None,
        },
        SELF_PID,
        Flavor::Windows,
    );
    assert_eq!(
        v,
        Verdict::Deny {
            code: "DESKTOP_INPUT_REFUSED",
            reason: "aevra-desktop does not control its own window".into()
        }
    );
}

#[test]
fn bundle_matches_executable_inside_contents() {
    let c = cfg(vec![AllowedApp {
        id: "mac".into(),
        name: "TextEdit".into(),
        identity: AppIdentity::Bundle {
            path: "/Applications/TextEdit.app/".into(),
        },
    }]);
    let inside = window(Some("/Applications/TextEdit.app/Contents/MacOS/TextEdit"));
    assert!(verdict(&c, &inside, Flavor::Unix).is_allow());
    let lookalike = window(Some("/Applications/TextEdit.app.evil/Contents/MacOS/x"));
    assert!(!verdict(&c, &lookalike, Flavor::Unix).is_allow());
    let outside = window(Some("/Applications/TextEdit.app/Resources/helper"));
    assert!(!verdict(&c, &outside, Flavor::Unix).is_allow());
}

#[test]
fn package_matches_windowsapps_family_dir_and_not_a_lookalike_prefix() {
    let c = cfg(vec![AllowedApp {
        id: "pkg".into(),
        name: "Foo".into(),
        identity: AppIdentity::Package {
            family_name: "Foo_8wekyb3d8bbwe".into(),
        },
    }]);
    let real = window(Some(
        r"C:\Program Files\WindowsApps\Foo_1.2.3.0_x64__8wekyb3d8bbwe\Foo.exe",
    ));
    assert!(verdict(&c, &real, Flavor::Windows).is_allow());
    let lookalike = window(Some(
        r"C:\Program Files\WindowsApps\FooBar_1.2.3.0_x64__8wekyb3d8bbwe\Foo.exe",
    ));
    assert!(!verdict(&c, &lookalike, Flavor::Windows).is_allow());
    let other_publisher = window(Some(
        r"C:\Program Files\WindowsApps\Foo_1.2.3.0_x64__otherpublisher\Foo.exe",
    ));
    assert!(!verdict(&c, &other_publisher, Flavor::Windows).is_allow());
    let elsewhere = window(Some(r"C:\Users\x\Foo_1.2.3.0_x64__8wekyb3d8bbwe\Foo.exe"));
    assert!(!verdict(&c, &elsewhere, Flavor::Windows).is_allow());
}

#[test]
fn empty_allowlist_refuses_everything() {
    let c = cfg(vec![]);
    let w = window(Some(NOTEPAD));
    match verdict(&c, &w, Flavor::Windows) {
        Verdict::Deny { code, reason } => {
            assert_eq!(code, "DESKTOP_APP_NOT_ALLOWED");
            assert_eq!(reason, "notepad.exe is not in the allowlist");
        }
        other => panic!("expected deny, got {other:?}"),
    }
    assert!(!window_visible_with(&c, &w, SELF_PID, Flavor::Windows));
}

#[test]
fn window_visible_filters_by_the_same_rules() {
    let c = cfg(vec![exe_app("app_1", NOTEPAD)]);
    assert!(window_visible_with(
        &c,
        &window(Some(NOTEPAD)),
        SELF_PID,
        Flavor::Windows
    ));
    assert!(!window_visible_with(
        &c,
        &window(Some(r"C:\Windows\explorer.exe")),
        SELF_PID,
        Flavor::Windows
    ));
    assert!(!window_visible_with(
        &c,
        &window(None),
        SELF_PID,
        Flavor::Windows
    ));
}

#[test]
fn identity_matches_path_uses_the_gate_rules_for_every_identity_kind() {
    use desktop_policy::gate::identity_matches_path;
    let exe = AppIdentity::Exe {
        path: if cfg!(windows) {
            r"C:\Apps\Example\example.exe".into()
        } else {
            "/opt/example/example".into()
        },
    };
    let same = if cfg!(windows) {
        r"c:\apps\example\EXAMPLE.EXE"
    } else {
        "/opt/example/example"
    };
    assert!(identity_matches_path(&exe, same));
    assert!(!identity_matches_path(&exe, "/opt/other/other"));
    assert!(!identity_matches_path(&exe, ""));

    let bundle = AppIdentity::Bundle {
        path: "/Applications/Example.app".into(),
    };
    if !cfg!(windows) {
        assert!(identity_matches_path(
            &bundle,
            "/Applications/Example.app/Contents/MacOS/Example"
        ));
        assert!(!identity_matches_path(
            &bundle,
            "/Applications/Example.app.evil/Contents/MacOS/Example"
        ));
    }

    let package = AppIdentity::Package {
        family_name: "Example.App_abc123".into(),
    };
    assert_eq!(
        identity_matches_path(
            &package,
            r"C:\Program Files\WindowsApps\Example.App_1.0.0.0_x64__abc123\app.exe"
        ),
        cfg!(windows)
    );
}
