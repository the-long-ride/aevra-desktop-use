//! Windows: Start Menu shortcuts and Store packages.
//!
//! Shortcut parsing is pure and compiled on every host; only the Store query
//! and the environment lookups are Windows specific.

use super::{is_launcher, AppEntry};
use desktop_policy::AppIdentity;
use std::path::{Path, PathBuf};

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn ansi_string_at(bytes: &[u8], at: usize) -> Option<String> {
    let tail = bytes.get(at..)?;
    let end = tail.iter().position(|&b| b == 0)?;
    Some(String::from_utf8_lossy(&tail[..end]).into_owned())
}

fn utf16_string_at(bytes: &[u8], at: usize) -> Option<String> {
    let tail = bytes.get(at..)?;
    let units: Vec<u16> = tail
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    // An unterminated string is a truncated file.
    if units.len() * 2 + 2 > tail.len() {
        return None;
    }
    Some(String::from_utf16_lossy(&units))
}

/// The local target path of a Shell Link (`.lnk`), from its `LinkInfo` block.
///
/// This reads only what [MS-SHLLINK] calls `LocalBasePath` plus
/// `CommonPathSuffix`. Shortcuts that carry only an ID list (most Store and
/// system shortcuts) or a network path give `None`, and so do truncated or
/// malformed files.
pub fn parse_lnk_target(bytes: &[u8]) -> Option<String> {
    const HEADER_SIZE: usize = 0x4C;
    if u32_at(bytes, 0)? as usize != HEADER_SIZE {
        return None;
    }
    let flags = u32_at(bytes, 0x14)?;
    let has_id_list = flags & 0x01 != 0;
    let has_link_info = flags & 0x02 != 0;
    if !has_link_info {
        return None;
    }
    let mut at = HEADER_SIZE;
    if has_id_list {
        at = at
            .checked_add(2)?
            .checked_add(u16_at(bytes, at)? as usize)?;
    }
    let tail = bytes.get(at..)?;
    let info_size = u32_at(tail, 0)? as usize;
    let info = tail.get(..info_size)?;

    let header_size = u32_at(info, 4)? as usize;
    let info_flags = u32_at(info, 8)?;
    // Bit 0: VolumeIDAndLocalBasePath. Without it the target is a network path.
    if info_flags & 0x01 == 0 {
        return None;
    }
    let local_base = u32_at(info, 0x10)? as usize;
    let suffix = u32_at(info, 0x18)? as usize;

    let (base, tail_part) = if header_size >= 0x24 {
        let local_base_unicode = u32_at(info, 0x1C)? as usize;
        let suffix_unicode = u32_at(info, 0x20)? as usize;
        let base = if local_base_unicode != 0 {
            utf16_string_at(info, local_base_unicode)?
        } else {
            ansi_string_at(info, local_base)?
        };
        let tail_part = if suffix_unicode != 0 {
            utf16_string_at(info, suffix_unicode)?
        } else {
            ansi_string_at(info, suffix).unwrap_or_default()
        };
        (base, tail_part)
    } else {
        (
            ansi_string_at(info, local_base)?,
            ansi_string_at(info, suffix).unwrap_or_default(),
        )
    };

    let target = format!("{base}{tail_part}");
    // An absolute drive or UNC path only.
    let drive = target.as_bytes().get(1) == Some(&b':');
    if target.is_empty() || !(drive || target.starts_with("\\\\")) {
        return None;
    }
    Some(target)
}

fn collect_shortcuts(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth > 0 {
                collect_shortcuts(&path, depth - 1, out);
            }
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
        {
            out.push(path);
        }
    }
}

/// App entries for the `.lnk` files under `dirs` that point at an executable.
/// With `require_target`, shortcuts whose target no longer exists are skipped.
pub fn scan_shortcuts(dirs: &[PathBuf], require_target: bool) -> Vec<AppEntry> {
    let mut shortcuts = Vec::new();
    for dir in dirs {
        collect_shortcuts(dir, 4, &mut shortcuts);
    }
    let mut entries = Vec::new();
    for shortcut in shortcuts {
        let name = shortcut
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() || name.to_lowercase().contains("uninstall") {
            continue;
        }
        let Some(target) = std::fs::read(&shortcut)
            .ok()
            .and_then(|bytes| parse_lnk_target(&bytes))
        else {
            continue;
        };
        if !target.to_lowercase().ends_with(".exe") || is_launcher(&target) {
            continue;
        }
        if require_target && !Path::new(&target).is_file() {
            continue;
        }
        entries.push(AppEntry {
            name,
            identity: AppIdentity::Exe { path: target },
            icon_path: None,
            running: false,
        });
    }
    entries
}

/// The per-machine and per-user Start Menu program folders.
pub fn start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for var in ["ProgramData", "APPDATA"] {
        if let Some(base) = std::env::var_os(var) {
            dirs.push(
                PathBuf::from(base)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Start Menu")
                    .join("Programs"),
            );
        }
    }
    dirs
}

/// Start Menu shortcuts and Store packages on this machine.
pub fn scan() -> Vec<AppEntry> {
    let mut entries = scan_shortcuts(&start_menu_dirs(), true);
    entries.extend(store_apps());
    entries
}

#[cfg(windows)]
fn store_apps() -> Vec<AppEntry> {
    use windows::ApplicationModel::PackageSignatureKind;
    use windows::Management::Deployment::PackageManager;
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    // The thread may already be initialised in another mode; that is fine.
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let Ok(manager) = PackageManager::new() else {
        return Vec::new();
    };
    let Ok(packages) = manager.FindPackagesByUserSecurityId(&windows::core::HSTRING::new()) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for package in packages {
        if package.IsFramework().unwrap_or(true) || package.IsResourcePackage().unwrap_or(true) {
            continue;
        }
        // System packages are not offered; Store, developer and enterprise are.
        match package.SignatureKind() {
            Ok(PackageSignatureKind::Store)
            | Ok(PackageSignatureKind::Developer)
            | Ok(PackageSignatureKind::Enterprise) => {}
            _ => continue,
        }
        let Ok(id) = package.Id() else { continue };
        let Ok(family) = id.FamilyName() else {
            continue;
        };
        let name = package
            .DisplayName()
            .map(|n| n.to_string())
            .unwrap_or_default();
        if name.trim().is_empty() || name.starts_with("ms-resource:") {
            continue;
        }
        entries.push(AppEntry {
            name,
            identity: AppIdentity::Package {
                family_name: family.to_string(),
            },
            icon_path: None,
            running: false,
        });
    }
    entries
}

#[cfg(not(windows))]
fn store_apps() -> Vec<AppEntry> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/example.lnk");

    /// A shortcut with a Unicode `LinkInfo` and an optional ID list.
    fn build_lnk(local_base: &str, suffix: &str, id_list: bool, unicode: bool) -> Vec<u8> {
        let mut header = vec![0u8; 0x4C];
        header[0..4].copy_from_slice(&0x4Cu32.to_le_bytes());
        header[4..20].copy_from_slice(&[
            0x01, 0x14, 0x02, 0x00, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46,
        ]);
        let flags: u32 = 0x02 | if id_list { 0x01 } else { 0 };
        header[0x14..0x18].copy_from_slice(&flags.to_le_bytes());

        let ansi = |s: &str| {
            let mut b = s.as_bytes().to_vec();
            b.push(0);
            b
        };
        let wide = |s: &str| {
            let mut b: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
            b.extend([0, 0]);
            b
        };
        let header_size: u32 = if unicode { 0x24 } else { 0x1C };
        let volume = {
            let mut v = Vec::new();
            v.extend(17u32.to_le_bytes());
            v.extend(3u32.to_le_bytes());
            v.extend(0u32.to_le_bytes());
            v.extend(0x10u32.to_le_bytes());
            v.push(0);
            v
        };
        let vol_off = header_size;
        let base_off = vol_off + volume.len() as u32;
        let base_bytes = ansi(local_base);
        let suffix_off = base_off + base_bytes.len() as u32;
        let suffix_bytes = ansi(suffix);
        let mut tail = Vec::new();
        let (base_u_off, suffix_u_off);
        let after_ansi = suffix_off + suffix_bytes.len() as u32;
        if unicode {
            base_u_off = after_ansi;
            let base_w = wide(local_base);
            suffix_u_off = base_u_off + base_w.len() as u32;
            tail.extend(base_w);
            tail.extend(wide(suffix));
        } else {
            base_u_off = 0;
            suffix_u_off = 0;
        }
        let size = after_ansi + tail.len() as u32;
        let mut info = Vec::new();
        for v in [size, header_size, 1, vol_off, base_off, 0, suffix_off] {
            info.extend(v.to_le_bytes());
        }
        if unicode {
            info.extend(base_u_off.to_le_bytes());
            info.extend(suffix_u_off.to_le_bytes());
        }
        info.extend(&volume);
        info.extend(base_bytes);
        info.extend(suffix_bytes);
        info.extend(tail);

        let mut out = header;
        if id_list {
            out.extend(4u16.to_le_bytes());
            out.extend([0u8; 4]);
        }
        out.extend(info);
        out
    }

    #[test]
    fn the_fixture_shortcut_resolves_to_its_target() {
        assert_eq!(
            parse_lnk_target(FIXTURE).as_deref(),
            Some(r"C:\Program Files\Example\example.exe")
        );
    }

    #[test]
    fn unicode_and_id_list_variants_resolve() {
        for (id_list, unicode) in [(false, true), (true, false), (true, true)] {
            let bytes = build_lnk(
                r"C:\Program Files\Example",
                r"\example.exe",
                id_list,
                unicode,
            );
            assert_eq!(
                parse_lnk_target(&bytes).as_deref(),
                Some(r"C:\Program Files\Example\example.exe"),
                "id_list={id_list} unicode={unicode}"
            );
        }
    }

    #[test]
    fn unicode_paths_prefer_the_wide_string() {
        let bytes = build_lnk("C:\\Programme\\Über", "\\über.exe", false, true);
        assert_eq!(
            parse_lnk_target(&bytes).as_deref(),
            Some("C:\\Programme\\Über\\über.exe")
        );
    }

    #[test]
    fn unresolvable_or_broken_shortcuts_give_none() {
        assert_eq!(parse_lnk_target(&[]), None);
        assert_eq!(
            parse_lnk_target(b"not a shortcut at all, definitely not"),
            None
        );
        // Truncated anywhere inside the LinkInfo block.
        for cut in [0x10, 0x4C, 0x50, 0x60, FIXTURE.len() - 3] {
            assert_eq!(parse_lnk_target(&FIXTURE[..cut]), None, "cut at {cut}");
        }
        // No LinkInfo flag: an ID-list-only shortcut.
        let mut id_list_only = FIXTURE.to_vec();
        id_list_only[0x14..0x18].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(parse_lnk_target(&id_list_only), None);
        // A relative target is not usable.
        assert_eq!(
            parse_lnk_target(&build_lnk("example", ".exe", false, false)),
            None
        );
    }

    #[test]
    fn hostile_offsets_do_not_panic() {
        // Flip every byte of the fixture in turn: parsing must stay total.
        for i in 0..FIXTURE.len() {
            let mut bytes = FIXTURE.to_vec();
            bytes[i] = 0xFF;
            let _ = parse_lnk_target(&bytes);
        }
    }

    #[test]
    fn scanning_a_start_menu_folder_lists_exe_shortcuts_only() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("Example Suite");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("Example.lnk"), FIXTURE).unwrap();
        std::fs::write(dir.path().join("Uninstall Example.lnk"), FIXTURE).unwrap();
        std::fs::write(dir.path().join("Broken.lnk"), b"junk").unwrap();
        std::fs::write(
            dir.path().join("Docs.lnk"),
            build_lnk(r"C:\Docs", r"\readme.txt", false, false),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Shell.lnk"),
            build_lnk(r"C:\Windows\System32", r"\cmd.exe", false, false),
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();

        let entries = scan_shortcuts(&[dir.path().to_path_buf()], false);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].name, "Example");
        assert_eq!(
            entries[0].identity,
            AppIdentity::Exe {
                path: r"C:\Program Files\Example\example.exe".into()
            }
        );
        // The target does not exist on this machine, so requiring it drops the entry.
        assert!(scan_shortcuts(&[dir.path().to_path_buf()], true).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn the_real_start_menu_and_store_scan_returns() {
        let entries = scan();
        let packages = entries
            .iter()
            .filter(|e| matches!(e.identity, AppIdentity::Package { .. }))
            .count();
        eprintln!(
            "windows scan found {} apps, {packages} of them Store packages",
            entries.len()
        );
        assert!(entries.iter().all(|e| !e.name.is_empty()));
    }
}
