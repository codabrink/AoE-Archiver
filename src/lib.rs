mod aoe;
mod config;
mod ctx;
mod goldberg;
mod steam;
mod ui;
pub mod utils;

mod slint_ui {
    slint::include_modules!();
}

use crate::aoe::{aoe1, aoe2};
use crate::ctx::{Context, GameId, StepStatus, Task};
use crate::ui::UiLayer;
use crate::utils::{validate_aoe1_source, validate_aoe2_source};
use anyhow::{bail, Context as AnyhowContext, Result};
use fs_extra::copy_items;
use fs_extra::dir::{get_size, CopyOptions};
use slint::{ComponentHandle, Model, SharedString, VecModel};
use slint_ui::MainWindow;
use slint_ui::StepInfo;
use slint_ui::StepStatus as UiStepStatus;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvError};
use std::sync::{mpsc, Arc};
use std::thread::sleep;
use std::time::Duration;
use tracing::{error, info};
use tracing_subscriber::layer::SubscriberExt;

enum AppUpdate {
    // AoE2
    Progress(Option<(String, f32)>),
    StepStatusChanged,
    SourceSize(u64),
    DestDriveAvailable(u64),
    // AoE1
    Aoe1Progress(Option<(String, f32)>),
    Aoe1StepStatusChanged,
    Aoe1SourceSize(u64),
    Aoe1DestDriveAvailable(u64),
    // Shared
    Log(String),
}

pub fn launch() -> Result<()> {
    let (update_tx, update_rx) = channel::<AppUpdate>();

    // Set up tracing to pipe logs to the UI
    let ui_layer = UiLayer {
        tx: update_tx.clone(),
    };

    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .finish()
        .with(ui_layer);

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");

    let ctx = Arc::new(Context::new(update_tx)?);

    let ui = MainWindow::new()?;

    // Shared log model (newest-first via insert at 0)
    let log_model = Rc::new(VecModel::<SharedString>::default());
    ui.set_logs(log_model.clone().into());

    // ── AoE2 steps model ──────────────────────────────────────────────────────
    let aoe2_steps_model = Rc::new(VecModel::<StepInfo>::from(vec![
        StepInfo { status: UiStepStatus::NotStarted, label: "1. Copy".into() },
        StepInfo { status: UiStepStatus::NotStarted, label: "2. Goldberg".into() },
        StepInfo { status: UiStepStatus::NotStarted, label: "3. Companion".into() },
        StepInfo { status: UiStepStatus::NotStarted, label: "4. Launcher".into() },
    ]));
    ui.set_steps(aoe2_steps_model.clone().into());

    // ── AoE1 steps model ──────────────────────────────────────────────────────
    let aoe1_steps_model = Rc::new(VecModel::<StepInfo>::from(vec![
        StepInfo { status: UiStepStatus::NotStarted, label: "1. Copy".into() },
        StepInfo { status: UiStepStatus::NotStarted, label: "2. Goldberg".into() },
        StepInfo { status: UiStepStatus::NotStarted, label: "3. Companion".into() },
        StepInfo { status: UiStepStatus::NotStarted, label: "4. Launcher".into() },
    ]));
    ui.set_aoe1_steps(aoe1_steps_model.clone().into());

    // ── Initialize paths from context ─────────────────────────────────────────
    if let Some(src) = ctx.sourcedir() {
        ui.set_source_dir(src.to_string_lossy().as_ref().into());
    }
    ui.set_out_dir(ctx.outdir().to_string_lossy().as_ref().into());

    if let Some(src) = ctx.aoe1_sourcedir() {
        ui.set_aoe1_source_dir(src.to_string_lossy().as_ref().into());
    }
    ui.set_aoe1_out_dir(ctx.aoe1_outdir().to_string_lossy().as_ref().into());

    // ── Initial can_run_all ───────────────────────────────────────────────────
    {
        let statuses = ctx.step_status.lock().unwrap();
        let can_run = ctx.sourcedir().is_some()
            && !ctx.is_busy()
            && statuses.iter().all(|s| matches!(s, StepStatus::NotStarted));
        ui.set_can_run_all(can_run);
    }
    {
        let statuses = ctx.aoe1_step_status.lock().unwrap();
        let can_run = ctx.aoe1_sourcedir().is_some()
            && !ctx.is_busy()
            && statuses.iter().all(|s| matches!(s, StepStatus::NotStarted));
        ui.set_aoe1_can_run_all(can_run);
    }

    // ── AoE2: select source folder ────────────────────────────────────────────
    ui.on_select_source_folder({
        let ctx = ctx.clone();
        let ui_weak = ui.as_weak();
        move || {
            let current = ctx.sourcedir();
            let mut dialog = rfd::FileDialog::new();
            if let Some(ref p) = current {
                dialog = dialog.set_directory(p);
            }
            if let Some(new_dir) = dialog.pick_folder() {
                info!("User selected AoE2 source directory: {}", new_dir.display());
                if let Err(e) = validate_aoe2_source(&new_dir) {
                    rfd::MessageDialog::new()
                        .set_title("Invalid Directory")
                        .set_description(&format!("{e}"))
                        .set_buttons(rfd::MessageButtons::Ok)
                        .show();
                    return;
                }
                ctx.set_sourcedir(new_dir.clone());
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_source_dir(new_dir.to_string_lossy().as_ref().into());
                    let statuses = ctx.step_status.lock().unwrap();
                    let can_run = !ctx.is_busy()
                        && statuses.iter().all(|s| matches!(s, StepStatus::NotStarted));
                    ui.set_can_run_all(can_run);
                }
            }
        }
    });

    // ── AoE2: select destination folder ──────────────────────────────────────
    ui.on_select_out_folder({
        let ctx = ctx.clone();
        let ui_weak = ui.as_weak();
        move || {
            let current = ctx.outdir();
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.set_directory(&current);
            if let Some(new_dir) = dialog.pick_folder() {
                info!("Selected AoE2 destination directory: {}", new_dir.display());
                ctx.set_outdir(new_dir.clone());
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_out_dir(new_dir.to_string_lossy().as_ref().into());
                }
            }
        }
    });

    // ── AoE2: run all steps ───────────────────────────────────────────────────
    ui.on_run_all({
        let ctx = ctx.clone();
        let ui_weak = ui.as_weak();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_can_run_all(false);
            }
            run_all_steps(ctx.clone(), GameId::Aoe2);
        }
    });

    // ── AoE1: select source folder ────────────────────────────────────────────
    ui.on_select_aoe1_source_folder({
        let ctx = ctx.clone();
        let ui_weak = ui.as_weak();
        move || {
            let current = ctx.aoe1_sourcedir();
            let mut dialog = rfd::FileDialog::new();
            if let Some(ref p) = current {
                dialog = dialog.set_directory(p);
            }
            if let Some(new_dir) = dialog.pick_folder() {
                info!("User selected AoE1 source directory: {}", new_dir.display());
                if let Err(e) = validate_aoe1_source(&new_dir) {
                    rfd::MessageDialog::new()
                        .set_title("Invalid Directory")
                        .set_description(&format!("{e}"))
                        .set_buttons(rfd::MessageButtons::Ok)
                        .show();
                    return;
                }
                ctx.set_aoe1_sourcedir(new_dir.clone());
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_aoe1_source_dir(new_dir.to_string_lossy().as_ref().into());
                    let statuses = ctx.aoe1_step_status.lock().unwrap();
                    let can_run = !ctx.is_busy()
                        && statuses.iter().all(|s| matches!(s, StepStatus::NotStarted));
                    ui.set_aoe1_can_run_all(can_run);
                }
            }
        }
    });

    // ── AoE1: select destination folder ──────────────────────────────────────
    ui.on_select_aoe1_out_folder({
        let ctx = ctx.clone();
        let ui_weak = ui.as_weak();
        move || {
            let current = ctx.aoe1_outdir();
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.set_directory(&current);
            if let Some(new_dir) = dialog.pick_folder() {
                info!("Selected AoE1 destination directory: {}", new_dir.display());
                ctx.set_aoe1_outdir(new_dir.clone());
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_aoe1_out_dir(new_dir.to_string_lossy().as_ref().into());
                }
            }
        }
    });

    // ── AoE1: run all steps ───────────────────────────────────────────────────
    ui.on_run_aoe1_all({
        let ctx = ctx.clone();
        let ui_weak = ui.as_weak();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_aoe1_can_run_all(false);
            }
            run_all_steps(ctx.clone(), GameId::Aoe1);
        }
    });

    // ── Disk space state (tracked across timer ticks) ─────────────────────────
    let aoe2_required_gb = Rc::new(Cell::new(0.0_f64));
    let aoe2_available_gb = Rc::new(Cell::new(0.0_f64));
    let aoe1_required_gb = Rc::new(Cell::new(0.0_f64));
    let aoe1_available_gb = Rc::new(Cell::new(0.0_f64));

    // ── 50ms polling timer ────────────────────────────────────────────────────
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(50),
        {
            let ui_weak = ui.as_weak();
            let log_model = log_model.clone();
            let aoe2_steps_model = aoe2_steps_model.clone();
            let aoe1_steps_model = aoe1_steps_model.clone();
            let ctx = ctx.clone();
            let aoe2_required_gb = aoe2_required_gb.clone();
            let aoe2_available_gb = aoe2_available_gb.clone();
            let aoe1_required_gb = aoe1_required_gb.clone();
            let aoe1_available_gb = aoe1_available_gb.clone();
            move || {
                let Some(ui) = ui_weak.upgrade() else { return };

                while let Ok(update) = update_rx.try_recv() {
                    match update {
                        // ── AoE2 ────────────────────────────────────────────
                        AppUpdate::Progress(Some((text, value))) => {
                            ui.set_has_progress(true);
                            ui.set_progress_text(text.as_str().into());
                            ui.set_progress_value(value);
                        }
                        AppUpdate::Progress(None) => {
                            ui.set_has_progress(false);
                        }
                        AppUpdate::SourceSize(n) => {
                            aoe2_required_gb.set(n as f64 / 1_073_741_824.0);
                            update_disk_space_aoe2(
                                &ui,
                                aoe2_required_gb.get(),
                                aoe2_available_gb.get(),
                            );
                        }
                        AppUpdate::DestDriveAvailable(n) => {
                            aoe2_available_gb.set(n as f64 / 1_073_741_824.0);
                            update_disk_space_aoe2(
                                &ui,
                                aoe2_required_gb.get(),
                                aoe2_available_gb.get(),
                            );
                        }
                        AppUpdate::StepStatusChanged => {
                            let statuses = ctx.step_status.lock().unwrap();
                            for (i, status) in statuses.iter().enumerate() {
                                if let Some(mut step) = aoe2_steps_model.row_data(i) {
                                    step.status = to_ui_status(status);
                                    aoe2_steps_model.set_row_data(i, step);
                                }
                            }
                            let can_run = ctx.sourcedir().is_some()
                                && !ctx.is_busy()
                                && statuses.iter().all(|s| matches!(s, StepStatus::NotStarted));
                            ui.set_can_run_all(can_run);
                        }
                        // ── AoE1 ────────────────────────────────────────────
                        AppUpdate::Aoe1Progress(Some((text, value))) => {
                            ui.set_aoe1_has_progress(true);
                            ui.set_aoe1_progress_text(text.as_str().into());
                            ui.set_aoe1_progress_value(value);
                        }
                        AppUpdate::Aoe1Progress(None) => {
                            ui.set_aoe1_has_progress(false);
                        }
                        AppUpdate::Aoe1SourceSize(n) => {
                            aoe1_required_gb.set(n as f64 / 1_073_741_824.0);
                            update_disk_space_aoe1(
                                &ui,
                                aoe1_required_gb.get(),
                                aoe1_available_gb.get(),
                            );
                        }
                        AppUpdate::Aoe1DestDriveAvailable(n) => {
                            aoe1_available_gb.set(n as f64 / 1_073_741_824.0);
                            update_disk_space_aoe1(
                                &ui,
                                aoe1_required_gb.get(),
                                aoe1_available_gb.get(),
                            );
                        }
                        AppUpdate::Aoe1StepStatusChanged => {
                            let statuses = ctx.aoe1_step_status.lock().unwrap();
                            for (i, status) in statuses.iter().enumerate() {
                                if let Some(mut step) = aoe1_steps_model.row_data(i) {
                                    step.status = to_ui_status(status);
                                    aoe1_steps_model.set_row_data(i, step);
                                }
                            }
                            let can_run = ctx.aoe1_sourcedir().is_some()
                                && !ctx.is_busy()
                                && statuses.iter().all(|s| matches!(s, StepStatus::NotStarted));
                            ui.set_aoe1_can_run_all(can_run);
                        }
                        // ── Shared ──────────────────────────────────────────
                        AppUpdate::Log(msg) => {
                            log_model.insert(0, msg.as_str().into());
                            if log_model.row_count() > 100 {
                                log_model.remove(log_model.row_count() - 1);
                            }
                        }
                    }
                }
            }
        },
    );

    ui.run()?;

    Ok(())
}

// ── Disk-space helpers ────────────────────────────────────────────────────────

fn update_disk_space_aoe2(ui: &MainWindow, required: f64, available: f64) {
    let text = format!("{:.2} GB required, {:.2} GB available", required, available);
    ui.set_disk_space_text(text.as_str().into());
    ui.set_disk_space_ok(available > required);
}

fn update_disk_space_aoe1(ui: &MainWindow, required: f64, available: f64) {
    let text = format!("{:.2} GB required, {:.2} GB available", required, available);
    ui.set_aoe1_disk_space_text(text.as_str().into());
    ui.set_aoe1_disk_space_ok(available > required);
}

fn to_ui_status(status: &StepStatus) -> UiStepStatus {
    match status {
        StepStatus::NotStarted => UiStepStatus::NotStarted,
        StepStatus::InProgress => UiStepStatus::InProgress,
        StepStatus::Completed => UiStepStatus::Completed,
        StepStatus::Failed(_) => UiStepStatus::Failed,
    }
}

// ── Copy game folder ──────────────────────────────────────────────────────────

fn spawn_copy_game_folder(ctx: Arc<Context>, game: GameId) -> Result<Receiver<()>> {
    let guard = ctx.set_task(Task::Copy)?;

    let (tx, rx) = mpsc::sync_channel(0);

    if ctx.game_sourcedir(game).is_none() {
        bail!("No source directory selected");
    }

    std::thread::spawn({
        move || {
            let _guard = guard;
            ctx.set_game_step_status(game, 0, StepStatus::InProgress);

            match copy_game_folder(ctx.clone(), game) {
                Ok(_) => {
                    ctx.set_game_step_status(game, 0, StepStatus::Completed);
                    info!("Copy completed successfully");
                    let _ = tx.send(());
                }
                Err(err) => {
                    let err_msg = format!("{:#}", err);
                    ctx.set_game_step_status(game, 0, StepStatus::Failed(err_msg.clone()));
                    error!("Copy failed: {err_msg}");
                }
            }
        }
    });

    Ok(rx)
}

fn copy_game_folder(ctx: Arc<Context>, game: GameId) -> Result<()> {
    let game_label = match game {
        GameId::Aoe2 => "AoE2",
        GameId::Aoe1 => "AoE1",
    };
    info!("Preparing to copy {game_label} files");

    let outdir = ctx.game_outdir(game);
    let source_dir = ctx
        .game_sourcedir(game)
        .ok_or_else(|| anyhow::anyhow!("No source directory"))?;

    // Validate source
    match game {
        GameId::Aoe2 => validate_aoe2_source(&source_dir).context("Source validation failed")?,
        GameId::Aoe1 => validate_aoe1_source(&source_dir).context("Source validation failed")?,
    }

    let dir_size = get_size(&source_dir).context("Failed to get source directory size")?;

    info!(
        "Copying from {} ({:.2} GB)",
        source_dir.display(),
        dir_size as f64 / 1_073_741_824.0
    );

    std::fs::create_dir_all(&outdir).context("Failed to create destination directory")?;

    let complete = Arc::new(AtomicBool::new(false));

    // Progress monitoring thread
    std::thread::spawn({
        let ctx = ctx.clone();
        let outdir = outdir.clone();
        let complete = complete.clone();
        move || loop {
            if complete.load(Ordering::Relaxed) {
                break;
            }
            if let Ok(dest_size) = get_size(&outdir) {
                let pct = (dest_size as f64 / dir_size as f64).min(1.0) as f32;
                let progress_update = match game {
                    GameId::Aoe2 => AppUpdate::Progress(Some((
                        format!("Copying... {:.1}%", pct * 100.0),
                        pct,
                    ))),
                    GameId::Aoe1 => AppUpdate::Aoe1Progress(Some((
                        format!("Copying... {:.1}%", pct * 100.0),
                        pct,
                    ))),
                };
                let _ = ctx.tx.send(progress_update);
            }
            sleep(Duration::from_millis(500));
        }
    });

    // Perform the copy
    let copy_options = CopyOptions::new();
    let from_paths = vec![source_dir];
    copy_items(&from_paths, &outdir, &copy_options).context("Failed to copy files")?;

    complete.store(true, Ordering::Relaxed);
    let done_update = match game {
        GameId::Aoe2 => AppUpdate::Progress(None),
        GameId::Aoe1 => AppUpdate::Aoe1Progress(None),
    };
    ctx.tx.send(done_update).ok();

    info!("{game_label} copy completed successfully");

    Ok(())
}

// ── Run-all pipeline ──────────────────────────────────────────────────────────

fn run_all_steps(ctx: Arc<Context>, game: GameId) {
    std::thread::spawn(move || {
        if let Err(err) = run_all_steps_inner(ctx, game) {
            let Err(err) = err.downcast::<RecvError>() else {
                return;
            };
            error!("{err:?}");
        }
    });
}

fn run_all_steps_inner(ctx: Arc<Context>, game: GameId) -> Result<()> {
    let label = match game {
        GameId::Aoe2 => "AoE2",
        GameId::Aoe1 => "AoE1",
    };

    // Step 1: Copy
    let rx = spawn_copy_game_folder(ctx.clone(), game)?;
    rx.recv()?;
    info!("Step 1/4 completed [{label}]: Game files copied");

    // Step 2: Goldberg
    let rx = goldberg::spawn_apply(ctx.clone(), game)?;
    rx.recv()?;
    info!("Step 2/4 completed [{label}]: Goldberg installed");

    // Step 3: Companion
    let rx = match game {
        GameId::Aoe2 => aoe2::companion::spawn_install_launcher_companion(ctx.clone())?,
        GameId::Aoe1 => aoe1::spawn_install_launcher_companion(ctx.clone())?,
    };
    rx.recv()?;
    info!("Step 3/4 completed [{label}]: Launcher Companion installed");

    // Step 4: Launcher
    let rx = match game {
        GameId::Aoe2 => aoe2::launcher::spawn_install_launcher(ctx.clone())?,
        GameId::Aoe1 => aoe1::spawn_install_launcher(ctx.clone())?,
    };
    rx.recv()?;
    info!("Step 4/4 completed [{label}]: Launcher installed");

    Ok(())
}
