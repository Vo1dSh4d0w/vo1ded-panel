//! WARNING: VOODOO STEAM SHENANIGANS
//!
//! This file exists because Steam does not believe in things like
//! "stable APIs", "simple metadata", or "making sense".
//!
//! What follows is a delicate archaeological dig through:
//!  - window class guesses
//!  - /proc spelunking
//!  - demonic VDF parsing
//!  - icon caches that may or may not exist today
//!
//! If this breaks:
//!  - It is probably Steam's fault.
//!  - It is definitely not deterministic.
//!  - Debugging it will make you question your life choices.
//!
//! If you are thinking "I'll just clean this up real quick":
//!  - No you won't.
//!  - You will make it worse.
//!  - Steam will change something tomorrow anyway.
//!
//! Touch this code only if:
//!  - You enjoy pain
//!  - You have accepted that the correct fix may be "it works on my machine"
//!
//! You have been warned. Proceed at your own risk.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use gtk::glib;
use regex::Regex;

static PATH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""path"\s+"([^"]+)""#).unwrap());
static INSTALLDIR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""installdir"\s+"([^"]+)""#).unwrap());
static APPID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""appid"\s+"([^"]+)""#).unwrap());

fn find_steam_icon(app_id: &str) -> Option<PathBuf> {
    let hicolor = glib::home_dir().join(".local/share/icons/hicolor");
    fs::read_dir(&hicolor).ok()?.flatten().find_map(|size| {
        let icon = size.path().join(format!("apps/steam_icon_{app_id}.png"));
        icon.exists().then_some(icon)
    })
}

pub fn try_grab_steam_icon(pid: i32, client_class: &str) -> Option<PathBuf> {
    if let Some(app_id) = client_class.strip_prefix("steam_app_") {
        return find_steam_icon(app_id);
    }

    let exe = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let exe = exe.to_str()?;

    let lib_folders = glib::home_dir().join(".local/share/Steam/config/libraryfolders.vdf");
    let lib_folders = fs::read_to_string(lib_folders).ok()?;

    let steam_dir = PATH_RE
        .captures_iter(&lib_folders)
        .map(|c| c.get(1).unwrap().as_str())
        .find(|path| exe.starts_with(path))?;

    let common = format!("{steam_dir}/steamapps/common/");
    let install_dir_suffix = exe.replace(&common, "");
    let install_dir = install_dir_suffix.split('/').next()?;

    let steamapps = Path::new(steam_dir).join("steamapps");
    for entry in fs::read_dir(&steamapps).ok()?.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("appmanifest") {
            continue;
        }
        let Ok(manifest) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let manifest_install_dir = INSTALLDIR_RE.captures(&manifest).map(|c| c[1].to_owned());
        if manifest_install_dir.as_deref() == Some(install_dir)
            && let Some(app_id) = APPID_RE.captures(&manifest)
        {
            return find_steam_icon(&app_id[1]);
        }
    }

    None
}
