use crate::{
    Ctx,
    ctx::{Game, Task},
    utils::{extract_7z, gh_download_url},
};
use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, array::Array},
};
use anyhow::{Context as AnyhowContext, Result, anyhow};
use common::KEY;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock,
        mpsc::{self, Receiver},
    },
};
use strum::IntoEnumIterator;
use tracing::{error, info};

const FILES: &[&str] = &[
    "steamclient.dll",
    "steamclient64.dll",
    "coldclientloader.ini",
    "steamclient_loader_x64.exe",
];
const SUBDIRS: &[&str] = &["dlls", "steam_settings", "saves"];

const STEAM_SETTINGS_FILES_SLICE: &[(&str, &str)] = &[
    (
        "supported_languages.txt",
        include_str!("../assets/aoe2/supported_languages.txt"),
    ),
    (
        "achievements.json",
        include_str!("../assets/aoe2/achievements.json"),
    ),
    (
        "configs.app.ini",
        include_str!("../assets/aoe2/configs.app.ini"),
    ),
    (
        "configs.user.ini",
        include_str!("../assets/aoe2/configs.user.ini"),
    ),
];
static STEAM_SETTINGS_FILES: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    STEAM_SETTINGS_FILES_SLICE
        .iter()
        .map(|(name, content)| (name.to_string(), content.to_string()))
        .collect()
});

pub const GOLDBERG_SUBDIR: &str = "goldberg";

pub fn spawn_apply(ctx: Arc<Ctx>) -> Result<Receiver<()>> {
    let guard = ctx.set_task(Task::Goldberg)?;

    let (tx, rx) = mpsc::sync_channel(0);

    std::thread::spawn(move || {
        let _guard = guard;
        ctx.set_step_status(1, crate::StepStatus::InProgress);
        match apply_goldberg(ctx.clone()) {
            Ok(_) => {
                ctx.set_step_status(1, crate::StepStatus::Completed);
                info!("Goldberg emulator applied successfully");
                let _ = tx.send(());
            }
            Err(err) => {
                let err_msg = format!("{:#}", err);
                ctx.set_step_status(1, crate::StepStatus::Failed(err_msg.clone()));
                error!("Goldberg installation failed: {err_msg}");
            }
        }
    });

    Ok(rx)
}

pub fn apply_goldberg(ctx: Arc<Ctx>) -> Result<()> {
    info!("Downloading Goldberg Emulator");

    let goldberg = &ctx.config.goldberg;
    let dl_url = gh_download_url(
        &goldberg.gh_user,
        &goldberg.gh_repo,
        Some(&goldberg.version),
        &["emu-win-release.7z"],
    )?
    .context("Unable to find goldberg download url")?;

    let goldberg_archive = {
        info!("Downloading goldberg from {}", dl_url);
        let gbe_archive = reqwest::blocking::get(dl_url)?.bytes()?.to_vec();

        info!("Extracting Goldberg Emulator Archive");
        let archive = extract_7z(&gbe_archive)?;
        info!("Extracted {} files from archive", archive.len());
        for path in archive.keys() {
            info!("  Archive contains: {}", path);
        }
        archive
    };

    let goldberg_dir = ctx.outdir().join(GOLDBERG_SUBDIR);
    std::fs::create_dir_all(&goldberg_dir)?;
    info!("Output directory: {}", goldberg_dir.display());

    info!("Patching goldberg into export");
    for (path, mut file) in goldberg_archive {
        const EXPERIMENTAL: &str = "release/steamclient_experimental/";
        if !path.starts_with(EXPERIMENTAL) {
            continue;
        }
        let original_path = path.replace(EXPERIMENTAL, "");
        let path_lower = original_path.to_lowercase();

        if !FILES.contains(&&*path_lower) {
            continue;
        }

        info!("Processing file: {}", original_path);

        // Determine the output filename, preserving case for non-encrypted files
        let output_filename = if path_lower == "steamclient_loader_x64.exe" {
            info!("Encrypting steamclient_loader_x64.exe");
            let key = Array::try_from(&KEY[..32]).expect("Key is always 32 bytes");
            let cipher = Aes256Gcm::new(&key);
            let nonce = Array::try_from([0; 12]).expect("Nonce should always work");
            file = cipher.encrypt(&nonce, &*file).expect("Encryption failure");
            "steamclient_loader_x64.encrypted".to_string()
        } else {
            original_path
        };

        let file_path = goldberg_dir.join(&output_filename);
        info!("Writing file to: {}", file_path.display());

        if let Some(parent) = file_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    anyhow!("Failed to create directory {}: {}", parent.display(), e)
                })?;
            }
        }

        std::fs::write(&file_path, file)
            .map_err(|e| anyhow!("Failed to write file {}: {}", file_path.display(), e))?;
        info!("Successfully wrote: {}", file_path.display());
    }

    for subdir in SUBDIRS {
        let subdir_path = goldberg_dir.join(subdir);
        info!("Creating subdirectory: {}", subdir_path.display());
        std::fs::create_dir_all(&subdir_path)
            .map_err(|e| anyhow!("Failed to create directory {}: {e}", subdir_path.display(),))?;
    }

    // Configure goldberg for AoE2
    info!("Patching goldberg configs");

    // Find the ini file case-insensitively
    let ini_path = std::fs::read_dir(&goldberg_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case("coldclientloader.ini"))
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            anyhow!(
                "ColdClientLoader.ini not found in {}. The file may not have been extracted from the archive.",
                goldberg_dir.display()
            )
        })?;

    info!("Found ini file at: {}", ini_path.display());
    for game in Game::iter() {
        create_cold_client_loader(&ini_path, game);
    }

    for (filename, default_file) in &*STEAM_SETTINGS_FILES {
        let src_path = PathBuf::from("assets").join("aoe2").join(filename);
        let dest_path = goldberg_dir.join("steam_settings").join(filename);
        if std::fs::exists(&src_path)? {
            std::fs::copy(src_path, dest_path)?;
        } else {
            std::fs::write(dest_path, default_file)?;
        }
    }

    let launcher = include_bytes!("../target/release-lto/launch.exe");
    std::fs::write(ctx.outdir().join("launcher.exe"), launcher)?;

    info!("Done installing goldberg");

    Ok(())
}

fn create_cold_client_loader(ini_path: &Path, game: Game) -> Result<()> {
    use ini::Ini;

    info!("Creating {game} goldberg config.");

    let mut conf = Ini::load_from_file(ini_path)
        .map_err(|e| anyhow!("Failed to load {}: {}", ini_path.display(), e))?;

    conf.with_section(Some("SteamClient"))
        .set("Exe", format!(r#"..\{}"#, game.exe_location()))
        .set("AppId", game.steam_app_id());
    if let Some(dlls) = game.dll_folder() {
        conf.with_section(Some("Injection"))
            .set("DllsToInjectFolder", dlls);
    }

    let outpath = ini_path.join(format!(".{game}"));
    info!("Writing ini file to: {}", outpath.display());
    conf.write_to_file(&outpath)
        .map_err(|e| anyhow!("Failed to write {}: {}", outpath.display(), e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{config::Config, utils::gh_download_url};

    #[test]
    fn test_goldberg_download_url() {
        let config = Config::load().unwrap();
        let dl_url = gh_download_url(
            &config.goldberg.gh_user,
            &config.goldberg.gh_repo,
            Some(&config.goldberg.version),
            &["emu-win-release.7z"],
        )
        .unwrap();

        assert!(dl_url.is_some());
    }
}
