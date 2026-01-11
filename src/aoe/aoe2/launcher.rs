use crate::{
    Context,
    ctx::{StepStatus, Task},
    utils::{extract_zip, gh_download_url},
};
use anyhow::{Result, bail};
use std::{
    fs::{self, read_to_string},
    process::Command,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
};
use tracing::{error, info};

pub fn spawn_install_launcher(ctx: Arc<Context>) -> Result<Receiver<()>> {
    let guard = ctx.set_task(Task::Launcher)?;

    let (tx, rx) = mpsc::sync_channel(0);
    std::thread::spawn(move || {
        let _guard = guard;
        ctx.set_step_status(3, StepStatus::InProgress);
        match install_launcher(ctx.clone()) {
            Ok(_) => {
                ctx.set_step_status(3, StepStatus::Completed);
                info!("Launcher installed successfully");
                let _ = tx.send(());
            }
            Err(err) => {
                let err_msg = format!("{:#}", err);
                ctx.set_step_status(3, StepStatus::Failed(err_msg.clone()));
                error!("Launcher installation failed: {err_msg}");
            }
        }
    });

    Ok(rx)
}

pub fn install_launcher(ctx: Arc<Context>) -> Result<()> {
    let Some(launcher_url) = launcher_full_url(&ctx)? else {
        bail!("Unable to find latest launcher release.");
    };
    info!("Downloading launcher.");

    let launcher_zip = reqwest::blocking::get(launcher_url)?.bytes()?.to_vec();
    let outdir = ctx.outdir();

    info!("Extracting launcher.");

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

    patch_launcher_aoe2_config(&ctx)?;

    info!("Generating certs.");

    let gen_certs_exe = outdir.join("server").join("bin").join("genCert.exe");
    let _ = Command::new(gen_certs_exe).status();

    patch_launcher_main_config(&ctx)?;

    info!("Done installing launcher.");

    Ok(())
}

fn patch_launcher_main_config(ctx: &Context) -> Result<()> {
    let outdir = ctx.outdir();

    info!("Patching launcher config.");
    let launcher_config_path = outdir
        .join("launcher")
        .join("resources")
        .join("config.toml");
    info!("Reading launcher config.");
    let launcher_config = read_to_string(&launcher_config_path)?;
    info!("Patching config.");
    let launcher_config =
        launcher_config.replace("SingleAutoSelect = false", "SingleAutoSelect = true");
    info!("Writing config to file.");
    fs::write(launcher_config_path, launcher_config)?;

    Ok(())
}

fn patch_launcher_aoe2_config(ctx: &Context) -> Result<()> {
    // Set the executable directory.
    let outdir = ctx.outdir();
    info!("Patching launcher aoe2 config.");
    let aoe2_config_path = outdir
        .join("launcher")
        .join("resources")
        .join("config.age2.toml");
    let aoe2_config = read_to_string(&aoe2_config_path)?;
    let aoe2_config = aoe2_config.replace(
        "Executable = 'auto'",
        r#"Executable = "../goldberg/steamclient_loader_x64.exe""#,
    );
    let aoe2_config = aoe2_config.replace("Path = 'auto'", r#"Path = "../AoE2DE""#);
    let aoe2_config = aoe2_config.replace(
        "ExecutableArgs = []",
        r#"ExecutableArgs = ['--overrideHosts="{HostFilePath}"']"#,
    );
    fs::write(aoe2_config_path, aoe2_config)?;

    Ok(())
}

fn launcher_full_url(ctx: &Context) -> Result<Option<String>> {
    info!("Getting latest launcher release url.");
    gh_download_url(
        &ctx.config.aoe2.gh_launcher_user,
        &ctx.config.aoe2.gh_launcher_repo,
        Some(&ctx.config.aoe2.launcher_version),
        &["_full_", "win10_x86-64"],
    )
}
