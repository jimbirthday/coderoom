use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub roots: Vec<String>,
    #[serde(default = "default_commit_index_branches")]
    pub commit_index_branches: usize,
    #[serde(default = "default_commit_index_commits_per_branch")]
    pub commit_index_commits_per_branch: usize,
}

impl Config {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            let s = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
            let cfg: Config = toml::from_str(&s).with_context(|| format!("parse {}", path.display()))?;
            Ok(cfg)
        } else {
            let cfg = Config::default();
            cfg.save(path)?;
            Ok(cfg)
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let s = toml::to_string_pretty(self)?;
        std::fs::write(path, s).with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }

    pub fn add_root(&mut self, root: &Path) {
        let root = normalize_path(root);
        if !self.roots.iter().any(|r| r == &root) {
            self.roots.push(root);
        }
    }

    pub fn remove_root(&mut self, root: &Path) -> bool {
        let root = normalize_path(root);
        let before = self.roots.len();
        self.roots.retain(|r| r != &root);
        before != self.roots.len()
    }
}

pub fn data_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    Ok(home.join(".coderoom"))
}

pub fn ensure_data_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    Ok(())
}

pub fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("config.toml")
}

pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("coderoom.db")
}

fn normalize_path(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn default_commit_index_branches() -> usize {
    10
}

fn default_commit_index_commits_per_branch() -> usize {
    50
}
