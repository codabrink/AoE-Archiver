use crate::{
    Context,
    ctx::{GameId, StepStatus, Task},
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
use tracing::{error, info};

const FILES: &[&str] = &[
    "steamclient.dll",
    "steamclient64.dll",
    "coldclientloader.ini",
    "steamclient_loader_x64.exe",
];
const SUBDIRS: &[&str] = &["dlls", "steam_settings", "saves"];

// ── AoE2 steam settings ──────────────────────────────────────────────────────

const AOE2_STEAM_SETTINGS_SLICE: &[(&str, &str)] = &[
    (
        "supported_languages.txt",
        include_str!("../assets/supported_languages.txt"),
    ),
    (
        "achievements.json",
        include_str!("../assets/achievements.json"),
    ),
    ("configs.app.ini", include_str!("../assets/configs.app.ini")),
    (
        "configs.user.ini",
        include_str!("../assets/configs.user.ini"),
    ),
];
static AOE2_STEAM_SETTINGS: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    AOE2_STEAM_SETTINGS_SLICE
        .iter()
        .map(|(name, content)| (name.to_string(), content.to_string()))
        .collect()
});

// ── AoE1 steam settings ──────────────────────────────────────────────────────

const AOE1_STEAM_SETTINGS_SLICE: &[(&str, &str)] = &[
    (
        "supported_languages.txt",
        include_str!("../assets/supported_languages.txt"),
    ),
    // AoE1 DE has no known public achievements list; use an empty array so
    // Goldberg does not complain about a missing file.
    ("achievements.json", "[]"),
    (
        "configs.app.ini",
        include_str!("../assets/aoe1_configs.app.ini"),
    ),
    (
        "configs.user.ini",
        include_str!("../assets/configs.user.ini"),
    ),
];
static AOE1_STEAM_SETTINGS: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    AOE1_STEAM_SETTINGS_SLICE
        .iter()
        .map(|(name, content)| (name.to_string(), content.to_string()))
        .collect()
});

// ── Per-game static config ────────────────────────────────────────────────────

pub struct GoldbergGameConfig {
    pub app_id: &'static str,
    pub exe_filename: &'static str,
    pub steam_settings: &'static LazyLock<HashMap<String, String>>,
}

pub static AOE2_GOLDBERG: GoldbergGameConfig = GoldbergGameConfig {
    app_id: "813780",
    exe_filename: "AoE2DE_s.exe",
    steam_settings: &AOE2_STEAM_SETTINGS,
};

pub static AOE1_GOLDBERG: GoldbergGameConfig = GoldbergGameConfig {
    app_id: "1017900",
    exe_filename: "AoEDE.exe",
    steam_settings: &AOE1_STEAM_SETTINGS,
};

fn config_for(game: GameId) -> &'static GoldbergGameConfig {
    match game {
        GameId::Aoe2 => &AOE2_GOLDBERG,
        GameId::Aoe1 => &AOE1_GOLDBERG,
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

pub const GOLDBERG_SUBDIR: &str = "goldberg";

/// Spawn goldberg apply in a background thread.  Step index is always 1 for
/// both games (it is the second step in each pipeline).
pub fn spawn_apply(ctx: Arc<Context>, game: GameId) -> Result<Receiver<()>> {
    let guard = ctx.set_task(Task::Goldberg)?;

    let (tx, rx) = mpsc::sync_channel(0);

    std::thread::spawn(move || {
        let _guard = guard;
        ctx.set_game_step_status(game, 1, StepStatus::InProgress);
        match apply_goldberg(ctx.clone(), game) {
            Ok(_) => {
                ctx.set_game_step_status(game, 1, StepStatus::Completed);
                info!("Goldberg emulator applied successfully");
                let _ = tx.send(());
            }
            Err(err) => {
                let err_msg = format!("{:#}", err);
                ctx.set_game_step_status(game, 1, StepStatus::Failed(err_msg.clone()));
                error!("Goldberg installation failed: {err_msg}");
            }
        }
    });

    Ok(rx)
}

pub fn apply_goldberg(ctx: Arc<Context>, game: GameId) -> Result<()> {
    let cfg = config_for(game);

    info!("Downloading Goldberg Emulator");

    let goldberg_cfg = &ctx.config.goldberg;
    let dl_url = gh_download_url(
        &goldberg_cfg.gh_user,
        &goldberg_cfg.gh_repo,
        Some(&goldberg_cfg.version),
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

    let outdir = ctx.game_outdir(game);
    let goldberg_dir = outdir.join(GOLDBERG_SUBDIR);
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
        std::fs::create_dir_all(&subdir_path).map_err(|e| {
            anyhow!(
                "Failed to create directory {}: {}",
                subdir_path.display(),
                e
            )
        })?;
    }

    // ── Configure goldberg ────────────────────────────────────────────────────

    info!("Patching goldberg configs");

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

    // Derive the game folder name from the source directory so it works
    // regardless of how Steam named the install folder.
    let source = ctx
        .game_sourcedir(game)
        .ok_or_else(|| anyhow!("No source directory set for game"))?;
    let game_folder = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("Cannot determine game folder name from source path"))?
        .to_string();

    update_cold_client_loader(&ini_path, cfg.app_id, &game_folder, cfg.exe_filename)?;

    // ── Write steam_settings files ────────────────────────────────────────────

    for (filename, default_content) in cfg.steam_settings.iter() {
        let src_path = PathBuf::from("assets").join(filename);
        let dest_path = goldberg_dir.join("steam_settings").join(filename);
        if std::fs::exists(&src_path)? {
            std::fs::copy(src_path, dest_path)?;
        } else {
            std::fs::write(dest_path, default_content)?;
        }
    }

    let launcher = include_bytes!("../target/release-lto/launch.exe");
    std::fs::write(outdir.join("launcher.exe"), launcher)?;

    info!("Done installing goldberg");

    Ok(())
}

fn update_cold_client_loader(
    ini_path: &Path,
    app_id: &str,
    game_folder: &str,
    exe_filename: &str,
) -> Result<()> {
    use ini::Ini;

    info!("Loading ini file from: {}", ini_path.display());
    let mut conf = Ini::load_from_file(ini_path)
        .map_err(|e| anyhow!("Failed to load {}: {}", ini_path.display(), e))?;

    let exe_path = format!(r#"..\{game_folder}\{exe_filename}"#);
    conf.with_section(Some("SteamClient"))
        .set("Exe", &exe_path)
        .set("AppId", app_id);
    conf.with_section(Some("Injection"))
        .set("DllsToInjectFolder", "dlls");

    info!("Writing updated ini file to: {}", ini_path.display());
    conf.write_to_file(ini_path)
        .map_err(|e| anyhow!("Failed to write {}: {}", ini_path.display(), e))?;

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
