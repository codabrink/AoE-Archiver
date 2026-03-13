use crate::{
    Context,
    ctx::{StepStatus, Task},
    goldberg::GOLDBERG_SUBDIR,
    utils::{extract_zip, gh_download_url},
};
use anyhow::{Result, bail};
use std::{
    fs,
    fs::read_to_string,
    process::Command,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
};
use tracing::{error, info};

// ── Companion ─────────────────────────────────────────────────────────────────

pub fn spawn_install_launcher_companion(ctx: Arc<Context>) -> Result<Receiver<()>> {
    let guard = ctx.set_task(Task::Companion)?;

    let (tx, rx) = mpsc::sync_channel(0);
    std::thread::spawn(move || {
        let _guard = guard;
        ctx.set_aoe1_step_status(2, StepStatus::InProgress);
        match install_launcher_companion(ctx.clone()) {
            Ok(_) => {
                ctx.set_aoe1_step_status(2, StepStatus::Completed);
                info!("AoE1 companion installed successfully");
                let _ = tx.send(());
            }
            Err(err) => {
                let err_msg = format!("{:#}", err);
                ctx.set_aoe1_step_status(2, StepStatus::Failed(err_msg.clone()));
                error!("AoE1 companion installation failed: {err_msg}");
            }
        }
    });

    Ok(rx)
}

pub fn install_launcher_companion(ctx: Arc<Context>) -> Result<()> {
    let Some(companion_full_url) = launcher_companion_full_url(&ctx)? else {
        bail!("Unable to find latest AoE1 companion release");
    };

    info!("Downloading AoE1 launcher companion.");

    let companion = reqwest::blocking::get(companion_full_url)?
        .bytes()?
        .to_vec();

    let goldberg_dir = ctx.aoe1_outdir().join(GOLDBERG_SUBDIR);
    info!("Extracting AoE1 launcher companion dlls.");
    for (name, file) in extract_zip(&companion)? {
        let lc_name = name.to_lowercase();
        // Include files relevant to AoE1 or the shared fakehost component.
        if !lc_name.contains("age1") && !lc_name.contains("fakehost") {
            continue;
        }

        let outpath = goldberg_dir.join("dlls").join(&name);
        fs::write(outpath, file)?;
    }

    info!("Done installing AoE1 companion.");

    Ok(())
}

fn launcher_companion_full_url(ctx: &Context) -> Result<Option<String>> {
    info!("Getting latest AoE1 launcher companion release url.");
    gh_download_url(
        &ctx.config.aoe1.gh_companion_user,
        &ctx.config.aoe1.gh_companion_repo,
        None,
        &["_full_"],
    )
}

// ── Launcher ──────────────────────────────────────────────────────────────────

pub fn spawn_install_launcher(ctx: Arc<Context>) -> Result<Receiver<()>> {
    let guard = ctx.set_task(Task::Launcher)?;

    let (tx, rx) = mpsc::sync_channel(0);
    std::thread::spawn(move || {
        let _guard = guard;
        ctx.set_aoe1_step_status(3, StepStatus::InProgress);
        match install_launcher(ctx.clone()) {
            Ok(_) => {
                ctx.set_aoe1_step_status(3, StepStatus::Completed);
                info!("AoE1 launcher installed successfully");
                let _ = tx.send(());
            }
            Err(err) => {
                let err_msg = format!("{:#}", err);
                ctx.set_aoe1_step_status(3, StepStatus::Failed(err_msg.clone()));
                error!("AoE1 launcher installation failed: {err_msg}");
            }
        }
    });

    Ok(rx)
}

pub fn install_launcher(ctx: Arc<Context>) -> Result<()> {
    let Some(launcher_url) = launcher_full_url(&ctx)? else {
        bail!("Unable to find latest AoE1 launcher release.");
    };
    info!("Downloading AoE1 launcher.");

    let launcher_zip = reqwest::blocking::get(launcher_url)?.bytes()?.to_vec();
    let outdir = ctx.aoe1_outdir();

    info!("Extracting AoE1 launcher.");

    for (name, file) in extract_zip(&launcher_zip)? {
        let mut outpath = outdir.to_path_buf();
        name.split("/").for_each(|c| outpath = outpath.join(c));

        if let Some(parent) = outpath.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(outpath, file)?;
    }

    patch_launcher_aoe1_config(&ctx)?;

    info!("Generating AoE1 certs.");

    let gen_certs_exe = outdir.join("server").join("bin").join("genCert.exe");
    let _ = Command::new(gen_certs_exe).status();

    patch_launcher_main_config(&ctx)?;

    info!("Done installing AoE1 launcher.");

    Ok(())
}

fn patch_launcher_main_config(ctx: &Context) -> Result<()> {
    let outdir = ctx.aoe1_outdir();

    info!("Patching AoE1 launcher main config.");
    let launcher_config_path = outdir
        .join("launcher")
        .join("resources")
        .join("config.toml");
    let launcher_config = read_to_string(&launcher_config_path)?;
    let launcher_config =
        launcher_config.replace("SingleAutoSelect = false", "SingleAutoSelect = true");
    fs::write(launcher_config_path, launcher_config)?;

    Ok(())
}

fn patch_launcher_aoe1_config(ctx: &Context) -> Result<()> {
    let outdir = ctx.aoe1_outdir();

    // Derive the game folder name from the source path so it works regardless
    // of how Steam named the install directory on this machine.
    let source = ctx
        .aoe1_sourcedir()
        .ok_or_else(|| anyhow::anyhow!("No AoE1 source directory set"))?;
    let game_folder = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("Cannot determine game folder name from AoE1 source path"))?
        .to_string();

    info!("Patching launcher AoE1 config (game folder: {game_folder}).");
    let aoe1_config_path = outdir
        .join("launcher")
        .join("resources")
        .join("config.age1.toml");
    let aoe1_config = read_to_string(&aoe1_config_path)?;
    let aoe1_config = aoe1_config.replace(
        "Executable = 'auto'",
        r#"Executable = "../goldberg/steamclient_loader_x64.exe""#,
    );
    let aoe1_config = aoe1_config.replace(
        "Path = 'auto'",
        &format!(r#"Path = "../{game_folder}""#),
    );
    let aoe1_config = aoe1_config.replace(
        "ExecutableArgs = []",
        r#"ExecutableArgs = ['--overrideHosts="{HostFilePath}"']"#,
    );
    fs::write(aoe1_config_path, aoe1_config)?;

    Ok(())
}

fn launcher_full_url(ctx: &Context) -> Result<Option<String>> {
    info!("Getting latest AoE1 launcher release url.");
    gh_download_url(
        &ctx.config.aoe1.gh_launcher_user,
        &ctx.config.aoe1.gh_launcher_repo,
        Some(&ctx.config.aoe1.launcher_version),
        &["_full_", "win10_x86-64"],
    )
}
