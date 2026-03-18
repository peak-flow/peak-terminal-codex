use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::theme::ThemeDefinition;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub theme: String,
    pub font_size: f32,
    pub font_name: Option<String>,
    pub shell: String,
    pub enable_starship: bool,
    pub custom_themes: Vec<ThemeDefinition>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: "Catppuccin Mocha".to_owned(),
            font_size: 16.0,
            font_name: None,
            shell: default_shell(),
            enable_starship: true,
            custom_themes: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        match Self::load_result() {
            Ok(config) => config,
            Err(error) => {
                eprintln!("Failed to load config: {error:#}");
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = config_file_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory at {}", parent.display())
            })?;
        }

        let contents = toml::to_string_pretty(self).context("Failed to encode config as TOML")?;
        fs::write(&path, contents)
            .with_context(|| format!("Failed to write config to {}", path.display()))?;
        Ok(())
    }

    fn load_result() -> Result<Self> {
        let path = config_file_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;
        let config = toml::from_str::<Self>(&contents)
            .with_context(|| format!("Failed to parse config from {}", path.display()))?;
        Ok(config)
    }
}

pub fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("dev", "Peak", "PeakTerminal")
        .context("Failed to determine application directories")
}

pub fn config_file_path() -> Result<PathBuf> {
    Ok(project_dirs()?.config_dir().join("config.toml"))
}

pub fn runtime_dir() -> Result<PathBuf> {
    Ok(project_dirs()?.cache_dir().join("runtime"))
}

pub fn themes_dir() -> Result<PathBuf> {
    Ok(project_dirs()?.config_dir().join("themes"))
}

pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

pub fn default_shell() -> String {
    env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned())
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("Failed to create directory {}", path.display()))?;
    Ok(())
}
