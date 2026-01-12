use crate::{
    Context,
    ctx::{StepStatus, Task},
    goldberg::GOLDBERG_SUBDIR,
    utils::{extract_zip, gh_download_url},
};
use anyhow::{Result, bail};
use std::{
    fs,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
};
use tracing::{error, info};

pub fn spawn_install_launcher_companion(ctx: Arc<Context>) -> Result<Receiver<()>> {
    let guard = ctx.set_task(Task::Companion)?;

    let (tx, rx) = mpsc::sync_channel(0);
    std::thread::spawn(move || {
        let _guard = guard;
        ctx.set_step_status(2, StepStatus::InProgress);
        match install_launcher_companion(ctx.clone()) {
            Ok(_) => {
                ctx.set_step_status(2, StepStatus::Completed);
                info!("Companion installed successfully");
                let _ = tx.send(());
            }
            Err(err) => {
                let err_msg = format!("{:#}", err);
                ctx.set_step_status(2, StepStatus::Failed(err_msg.clone()));
                error!("Companion installation failed: {err_msg}");
            }
        }
    });

    Ok(rx)
}

pub fn install_launcher_companion(ctx: Arc<Context>) -> Result<()> {
    let Some(companion_full_url) = launcher_companion_full_url(&ctx)? else {
        bail!("Unable to find latest companion release");
    };

    info!("Downloading launcher companion.");

    let companion = reqwest::blocking::get(companion_full_url)?
        .bytes()?
        .to_vec();

    let goldberg_dir = ctx.outdir().join(GOLDBERG_SUBDIR);
    info!("Extracting launcher companion dlls.");
    for (name, file) in extract_zip(&companion)? {
        let lc_name = name.to_lowercase();
        if !lc_name.contains("age2") && !lc_name.contains("fakehost") {
            continue;
        }

        let outpath = goldberg_dir.join("dlls").join(name);
        fs::write(outpath, file)?;
    }

    info!("Done installing companion.");

    Ok(())
}

fn launcher_companion_full_url(ctx: &Context) -> Result<Option<String>> {
    info!("Getting latest launcher companion release url.");
    gh_download_url(
        &ctx.config.aoe2.gh_companion_user,
        &ctx.config.aoe2.gh_companion_repo,
        None,
        &["_full_"],
    )
}
