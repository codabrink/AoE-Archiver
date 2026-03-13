use anyhow::Result;
use serde::Deserialize;
use std::fs::read_to_string;

const DEFAULT_CONFIG: &str = include_str!("../config.toml");

#[derive(Deserialize)]
pub struct Config {
    pub goldberg: Goldberg,
    pub aoe2: AoE2,
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_str = if std::fs::exists("config.toml")? {
            read_to_string("config.toml")?
        } else {
            DEFAULT_CONFIG.to_string()
        };
        Ok(toml::from_str(&config_str)?)
    }
}

#[derive(Deserialize)]
pub struct Goldberg {
    pub gh_user: String,
    pub gh_repo: String,
    pub version: String,
}

#[derive(Deserialize)]
pub struct AoE2 {
    pub gh_companion_user: String,
    pub gh_companion_repo: String,
    pub gh_launcher_user: String,
    pub gh_launcher_repo: String,
    pub launcher_version: String,
}
