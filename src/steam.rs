use anyhow::Result;
use std::path::PathBuf;
use winreg::RegKey;
use winreg::enums::*;

use crate::ctx::GameDisc;

pub fn source_path<'a>(game: impl Into<&'a GameDisc>) -> Result<Option<PathBuf>> {
    let game: &'a GameDisc = game.into();
    steam_install_location(&format!("Steam App {}", game.steam_app_id()))
}

pub fn steam_install_location(app_name: &str) -> Result<Option<PathBuf>> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);

    // Try the most common location first (64-bit systems)
    const ROOTS: &[&str] = &[
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\",
        "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\",
    ];

    for root in ROOTS {
        let mut registry_path = root.to_string();
        registry_path.push_str(app_name);

        if let Ok(key) = hklm.open_subkey(registry_path) {
            if let Ok(install_path) = key.get_value::<String, _>("InstallLocation") {
                return Ok(Some(PathBuf::from(install_path)));
            }
        }
    }

    Ok(None)
}
