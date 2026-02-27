use crate::{AppUpdate, config::Config, steam::steam_aoe2_path, utils::desktop_dir};
use anyhow::{Result, bail};
use fs_extra::dir::get_size;
use fs2::available_space;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc::Sender},
};

pub struct Context {
    pub config: Config,
    pub tx: Sender<AppUpdate>,
    sourcedir: Mutex<Option<PathBuf>>,
    outdir: Mutex<PathBuf>,
    current_task: Mutex<Option<Task>>,
    pub step_status: Mutex<[StepStatus; 4]>,
}

impl Context {
    pub fn new(tx: Sender<AppUpdate>) -> Result<Self> {
        let ctx = Self {
            tx,
            config: Config::load()?,
            sourcedir: Mutex::default(),
            outdir: Mutex::default(),
            current_task: Mutex::default(),

            step_status: Mutex::new([const { StepStatus::NotStarted }; 4]),
        };

        if let Some(source) = steam_aoe2_path()? {
            ctx.set_sourcedir(source);
        }

        ctx.set_outdir(desktop_dir()?.join("AoE2"));

        Ok(ctx)
    }

    pub fn sourcedir(&self) -> Option<PathBuf> {
        self.sourcedir.lock().unwrap().clone()
    }

    pub fn outdir(&self) -> PathBuf {
        self.outdir.lock().unwrap().clone()
    }

    pub fn set_sourcedir(&self, path: PathBuf) {
        // Get sizes and check disk space
        if let Ok(dir_size) = get_size(&path) {
            let _ = self.tx.send(AppUpdate::SourceSize(dir_size));
        }

        *self.sourcedir.lock().unwrap() = Some(path);
    }

    pub fn set_outdir(&self, path: PathBuf) {
        if let Ok(disk_size) = available_space(&path) {
            let _ = self.tx.send(AppUpdate::DestDriveAvailable(disk_size));
        } else if let Some(parent) = path.parent()
            && let Ok(disk_size) = available_space(parent)
        {
            let _ = self.tx.send(AppUpdate::DestDriveAvailable(disk_size));
        }

        *self.outdir.lock().unwrap() = path;
    }

    pub fn set_step_status(&self, step: usize, status: StepStatus) {
        if let Ok(mut steps) = self.step_status.lock()
            && step < steps.len()
        {
            steps[step] = status;
        }

        let _ = self.tx.send(AppUpdate::StepStatusChanged);
    }
}

impl Context {
    pub fn set_task(self: &Arc<Self>, task: Task) -> Result<TaskReset> {
        let mut guard = self.current_task.lock().unwrap();
        if let Some(existing_task) = &*guard {
            bail!("Task already running: {existing_task:?}");
        };

        let reset = TaskReset::new(self.clone());
        *guard = Some(task);

        Ok(reset)
    }

    pub fn is_busy(&self) -> bool {
        self.current_task.lock().unwrap().is_some()
    }
}

#[derive(Debug, Clone)]
pub enum Task {
    Copy,
    Goldberg,
    Companion,
    Launcher,
}

pub struct TaskReset {
    ctx: Arc<Context>,
}
impl TaskReset {
    pub fn new(ctx: Arc<Context>) -> Self {
        Self { ctx }
    }
}
impl Drop for TaskReset {
    fn drop(&mut self) {
        *self.ctx.current_task.lock().unwrap() = None;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepStatus {
    NotStarted,
    InProgress,
    Completed,
    Failed(String),
}

