use crate::{
    Ctx,
    ctx::{StepStatus, Task},
    utils::{extract_tar_gz, extract_zip, gh_download_url},
};
use anyhow::{Result, bail};
use std::{
    env::consts::{ARCH, OS},
    fs::{self, read_to_string},
    process::Command,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
};
use tracing::{error, info};

pub fn spawn_install_launcher(ctx: Arc<Ctx>) -> Result<Receiver<()>> {
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

pub fn install_launcher(ctx: Arc<Ctx>) -> Result<()> {
    let Some(launcher_url) = launcher_full_url(&ctx)? else {
        bail!("Unable to find latest launcher release.");
    };
    info!("Downloading launcher.");

    let launcher_archive = reqwest::blocking::get(&launcher_url)?.bytes()?.to_vec();
    let outdir = ctx.outdir();

    info!("Extracting launcher.");

    let extract_fn = if launcher_url.contains(".zip") {
        extract_zip
    } else if launcher_url.contains(".tar.gz") {
        extract_tar_gz
    } else {
        bail!("Unable to extract archive");
    };

    for (name, file) in extract_fn(&launcher_archive)? {
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

fn patch_launcher_main_config(ctx: &Ctx) -> Result<()> {
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

fn patch_launcher_aoe2_config(ctx: &Ctx) -> Result<()> {
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

const ARCH_LEXICON: &[(&str, &str)] =
    &[("x86", "_x86-32"), ("x86_64", "_x86-64"), ("arm", "_arm64")];
const OS_LEXICON: &[(&str, &str)] = &[("windows", "_win"), ("linux", "_linux")];

fn launcher_full_url(ctx: &Ctx) -> Result<Option<String>> {
    info!("Getting latest launcher release url.");

    let arch = ARCH_LEXICON
        .iter()
        .find(|(arch, _)| k == ARCH)
        .map(|(_, url)| *url)
        .context(format!("{ARCH} arch unsupported"))?;
    let os = OS_LEXICON
        .iter()
        .find(|(os, _)| os == OS)
        .map(|(_, url)| *url)
        .context("{OS} operating system unsupported")?;

    gh_download_url(
        &ctx.config.aoe2.gh_launcher_user,
        &ctx.config.aoe2.gh_launcher_repo,
        Some(&ctx.config.aoe2.launcher_version),
        &["_full_", os, arch],
    )
}
