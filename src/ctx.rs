use crate::{AppUpdate, config::Config, steam::source_path, utils::desktop_dir};
use anyhow::{Result, bail};
use fs_extra::dir::get_size;
use fs2::available_space;
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    fmt,
    hash::Hash,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, mpsc::Sender},
};
use strum::IntoEnumIterator;

pub struct Ctx {
    pub config: Config,
    pub tx: Sender<AppUpdate>,
    pub sources: Mutex<HashMap<GameDisc, Box<dyn GameTrait>>>,
    outdir: Mutex<PathBuf>,
    current_task: Mutex<Option<Task>>,
    pub step_status: Mutex<[StepStatus; 4]>,
}

#[derive(PartialEq, Eq, Hash)]
pub enum GameDisc {
    Aoe1,
    Aoe2,
}

#[derive(strum::EnumIter)]
pub enum Game {
    Aoe1(Aoe1),
    Aoe2(Aoe2),
}

impl<'a> From<&'a Game> for &'a GameDisc {
    fn from(game: &'a Game) -> Self {
        game.disc()
    }
}

#[derive(Default)]
pub struct Aoe1 {
    source: Option<PathBuf>,
}
#[derive(Default)]
pub struct Aoe2 {
    source: Option<PathBuf>,
}

trait GameTrait {
    fn steam_app_id(&self) -> &str;
    fn exe_location(&self) -> &str;
    fn dll_folder(&self) -> Option<&str>;
    fn source(&self) -> Option<&PathBuf>;
}

impl GameTrait for Aoe1 {
    fn steam_app_id(&self) -> &str {
        "1017900"
    }

    fn exe_location(&self) -> &str {
        r#"AoEDE\AoEDE_s.exe"#
    }

    fn dll_folder(&self) -> Option<&str> {
        None
    }
    fn source(&self) -> Option<&PathBuf> {
        self.source.as_ref()
    }
}

impl GameTrait for Aoe2 {
    fn steam_app_id(&self) -> &str {
        "813780"
    }

    fn exe_location(&self) -> &str {
        r#"AoE2DE\AoE2DE_s.exe"#
    }

    fn dll_folder(&self) -> Option<&str> {
        Some("dlls")
    }
    fn source(&self) -> Option<&PathBuf> {
        self.source.as_ref()
    }
}

impl GameDisc {
    pub fn steam_app_id(&self) -> &str {
        match self {
            Self::Aoe1 => "1017900",
            Self::Aoe2 => "813780",
        }
    }

    pub fn exe_location(&self) -> &str {
        match self {
            Self::Aoe1 => r#"AoEDE\AoEDE_s.exe"#,
            Self::Aoe2 => r#"AoE2DE\AoE2DE_s.exe"#,
        }
    }

    pub fn dll_folder(&self) -> Option<&str> {
        match self {
            Self::Aoe1 => None,
            Self::Aoe2 => Some("dlls"),
        }
    }
}

impl Game {
    pub const fn disc(&self) -> &GameDisc {
        match self {
            Self::Aoe1(_) => &GameDisc::Aoe1,
            Self::Aoe2(_) => &GameDisc::Aoe2,
        }
    }

    pub fn steam_app_id(&self) -> &str {
        match self {
            Self::Aoe1(game) => game.steam_app_id(),
            Self::Aoe2(game) => game.steam_app_id(),
        }
    }

    pub fn exe_location(&self) -> &str {
        match self {
            Self::Aoe1(game) => game.exe_location(),
            Self::Aoe2(game) => game.exe_location(),
        }
    }

    pub fn dll_folder(&self) -> Option<&str> {
        match self {
            Self::Aoe1(game) => game.dll_folder(),
            Self::Aoe2(game) => game.dll_folder(),
        }
    }
}

impl fmt::Display for Game {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aoe1(_) => write!(f, "aoe1"),
            Self::Aoe2(_) => write!(f, "aoe2"),
        }
    }
}

impl FromStr for Game {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "aoe1" => Ok(Game::Aoe1(Aoe1::default())),
            "aoe2" => Ok(Game::Aoe2(Aoe2::default())),
            _ => bail!("Unknown game: {s}"),
        }
    }
}
impl Hash for Game {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.disc().hash(state);
    }
}
impl PartialEq for Game {
    fn eq(&self, other: &Self) -> bool {
        self.disc() == other.disc()
    }
}
impl Eq for Game {}

const ORD: &[GameDisc] = &[GameDisc::Aoe1, GameDisc::Aoe2];
impl PartialOrd for Game {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        ORD.iter()
            .position(|g| g == self.disc())
            .unwrap()
            .partial_cmp(&ORD.iter().position(|g| g == other.disc()).unwrap())
    }
}

impl Ctx {
    pub fn new(tx: Sender<AppUpdate>) -> Result<Self> {
        let ctx = Self {
            tx,
            config: Config::load()?,
            sources: Mutex::default(),
            outdir: Mutex::default(),
            current_task: Mutex::default(),

            step_status: Mutex::new([const { StepStatus::NotStarted }; 4]),
        };

        for game in Game::iter() {
            if let Some(source) = source_path(&game)? {
                ctx.set_sourcedir(game, source);
            }
        }

        ctx.set_outdir(desktop_dir()?.join("AoE"));

        Ok(ctx)
    }

    pub fn sourcedir(&self, game: &GameDisc) -> Option<&PathBuf> {
        self.sources.lock().get(game).map(|g| g)
    }

    pub fn outdir(&self) -> PathBuf {
        self.outdir.lock().clone()
    }

    pub fn set_sourcedir(&self, game: Game, path: PathBuf) {
        let mut sources = self.sources.lock();
        sources.insert(game, path);

        let sources_size = sources.values().fold(0, |acc, p| get_size(p).unwrap_or(0));
        let _ = self.tx.send(AppUpdate::SourceSize(sources_size));
    }

    pub fn set_outdir(&self, path: PathBuf) {
        if let Ok(disk_size) = available_space(&path) {
            let _ = self.tx.send(AppUpdate::DestDriveAvailable(disk_size));
        } else if let Some(parent) = path.parent()
            && let Ok(disk_size) = available_space(parent)
        {
            let _ = self.tx.send(AppUpdate::DestDriveAvailable(disk_size));
        }

        *self.outdir.lock() = path;
    }

    pub fn set_step_status(&self, step: usize, status: StepStatus) {
        let mut steps = self.step_status.lock();
        if step < steps.len() {
            steps[step] = status;
        }

        let _ = self.tx.send(AppUpdate::StepStatusChanged);
    }
}

impl Ctx {
    pub fn set_task(self: &Arc<Self>, task: Task) -> Result<TaskReset> {
        let mut guard = self.current_task.lock();
        if let Some(existing_task) = &*guard {
            bail!("Task already running: {existing_task:?}");
        };

        let reset = TaskReset::new(self.clone());
        *guard = Some(task);

        Ok(reset)
    }

    pub fn is_busy(&self) -> bool {
        self.current_task.lock().is_some()
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
    ctx: Arc<Ctx>,
}
impl TaskReset {
    pub fn new(ctx: Arc<Ctx>) -> Self {
        Self { ctx }
    }
}
impl Drop for TaskReset {
    fn drop(&mut self) {
        *self.ctx.current_task.lock() = None;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepStatus {
    NotStarted,
    InProgress,
    Completed,
    Failed(String),
}
