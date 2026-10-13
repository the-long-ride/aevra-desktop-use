//! macOS: `.app` bundles in the standard application folders.
//!
//! Plist reading is a minimal XML scan, compiled everywhere. Binary plists are
//! not parsed; the bundle's file name is used instead.

use super::AppEntry;
use desktop_policy::AppIdentity;
use std::path::{Path, PathBuf};

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The `<string>` value of `key` in the top level `<dict>` of an XML plist.
pub fn plist_string(xml: &str, key: &str) -> Option<String> {
    if xml.trim_start().starts_with("bplist") {
        return None;
    }
    let marker = format!("<key>{key}</key>");
    let after = &xml[xml.find(&marker)? + marker.len()..];
    let after = after.trim_start();
    let body = after.strip_prefix("<string>")?;
    let value = &body[..body.find("</string>")?];
    let value = unescape(value.trim());
    (!value.is_empty()).then_some(value)
}

/// `CFBundleDisplayName`, else `CFBundleName`.
pub fn plist_app_name(xml: &str) -> Option<String> {
    plist_string(xml, "CFBundleDisplayName").or_else(|| plist_string(xml, "CFBundleName"))
}

fn bundle_entry(bundle: &Path) -> Option<AppEntry> {
    let stem = bundle.file_stem()?.to_string_lossy().into_owned();
    let plist = std::fs::read_to_string(bundle.join("Contents").join("Info.plist")).ok();
    let name = plist
        .as_deref()
        .and_then(plist_app_name)
        .unwrap_or_else(|| stem.clone());
    let icon_path = plist
        .as_deref()
        .and_then(|p| plist_string(p, "CFBundleIconFile"))
        .map(|icon| {
            let file = if icon.contains('.') {
                icon
            } else {
                format!("{icon}.icns")
            };
            bundle.join("Contents").join("Resources").join(file)
        })
        .filter(|p| p.is_file());
    Some(AppEntry {
        name,
        identity: AppIdentity::Bundle {
            path: bundle.to_string_lossy().into_owned(),
        },
        icon_path,
        running: false,
    })
}

fn is_bundle(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "app") && path.is_dir()
}

/// Bundles in `roots`, looking at most two levels down (`/Applications/Utilities/X.app`).
pub fn scan_bundles(roots: &[PathBuf]) -> Vec<AppEntry> {
    let mut entries = Vec::new();
    for root in roots {
        let Ok(read) = std::fs::read_dir(root) else {
            continue;
        };
        for child in read.flatten().map(|e| e.path()) {
            if is_bundle(&child) {
                entries.extend(bundle_entry(&child));
            } else if child.is_dir() {
                let Ok(inner) = std::fs::read_dir(&child) else {
                    continue;
                };
                for grandchild in inner.flatten().map(|e| e.path()) {
                    if is_bundle(&grandchild) {
                        entries.extend(bundle_entry(&grandchild));
                    }
                }
            }
        }
    }
    entries
}

pub fn application_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    roots
}

pub fn scan() -> Vec<AppEntry> {
    scan_bundles(&application_roots())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLIST: &str = include_str!("../../tests/fixtures/Info.plist");
    const NAME_ONLY: &str = include_str!("../../tests/fixtures/Info-name-only.plist");

    #[test]
    fn the_display_name_wins_over_the_bundle_name() {
        assert_eq!(plist_app_name(PLIST).as_deref(), Some("Example"));
    }

    #[test]
    fn the_bundle_name_is_the_fallback_and_entities_are_decoded() {
        assert_eq!(plist_app_name(NAME_ONLY).as_deref(), Some("Fish & Chips"));
    }

    #[test]
    fn missing_keys_and_binary_plists_give_none() {
        assert_eq!(plist_app_name("<plist><dict></dict></plist>"), None);
        assert_eq!(plist_app_name("bplist00\u{1}\u{2}"), None);
        assert_eq!(
            plist_string(
                "<key>CFBundleName</key><integer>3</integer>",
                "CFBundleName"
            ),
            None
        );
        assert_eq!(
            plist_string("<key>CFBundleName</key><string></string>", "CFBundleName"),
            None
        );
    }

    fn make_bundle(root: &Path, relative: &str, plist: Option<&str>) -> PathBuf {
        let bundle = root.join(relative);
        let contents = bundle.join("Contents");
        std::fs::create_dir_all(contents.join("Resources")).unwrap();
        if let Some(plist) = plist {
            std::fs::write(contents.join("Info.plist"), plist).unwrap();
        }
        bundle
    }

    #[test]
    fn bundles_are_found_two_levels_deep_with_names_and_icons() {
        let dir = tempfile::tempdir().unwrap();
        let example = make_bundle(dir.path(), "Example.app", Some(PLIST));
        std::fs::write(example.join("Contents/Resources/AppIcon.icns"), b"icns").unwrap();
        make_bundle(dir.path(), "Utilities/Tool.app", None);
        make_bundle(dir.path(), "Too/Deep/Hidden.app", Some(PLIST));
        std::fs::create_dir_all(dir.path().join("Folder")).unwrap();
        std::fs::write(dir.path().join("plain.txt"), b"x").unwrap();

        let mut entries = scan_bundles(&[dir.path().to_path_buf()]);
        entries.sort_by_key(|e| e.name.clone());
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0].name, "Example");
        assert!(entries[0].icon_path.is_some());
        assert_eq!(
            entries[0].identity,
            AppIdentity::Bundle {
                path: example.to_string_lossy().into_owned()
            }
        );
        // No Info.plist: the file stem is the name.
        assert_eq!(entries[1].name, "Tool");
        assert!(entries[1].icon_path.is_none());
    }

    #[test]
    fn missing_roots_are_skipped() {
        assert!(scan_bundles(&[PathBuf::from("/definitely/not/here")]).is_empty());
    }
}
