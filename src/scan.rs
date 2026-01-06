use crate::db::RepoMeta;
use anyhow::{Context, Result};
use chrono::Utc;
use git2::Repository;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn discover_git_repos(
    root: &Path,
    max_depth: Option<usize>,
    ignore_dir_names: &HashSet<String>,
) -> Result<Vec<PathBuf>> {
    let mut repos = HashSet::<PathBuf>::new();

    let mut walker = WalkDir::new(root).follow_links(false);
    if let Some(d) = max_depth {
        walker = walker.max_depth(d);
    }

    let mut it = walker.into_iter();
    while let Some(entry) = it.next() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy();
        if name == ".git" {
            if let Some(repo_root) = entry.path().parent() {
                repos.insert(repo_root.to_path_buf());
            }
            it.skip_current_dir();
            continue;
        }

        if ignore_dir_names.contains(name.as_ref()) {
            it.skip_current_dir();
            continue;
        }
    }

    let mut repos: Vec<_> = repos.into_iter().collect();
    repos.sort();
    Ok(repos)
}

pub fn read_repo_metadata(repo_root: &Path) -> Result<RepoMeta> {
    let repo_root = std::fs::canonicalize(repo_root).unwrap_or_else(|_| repo_root.to_path_buf());
    let name = repo_root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| repo_root.to_string_lossy().to_string());

    let mut default_branch: Option<String> = None;
    let mut last_commit_ts: Option<i64> = None;
    let mut origin_url: Option<String> = None;

    if let Ok(repo) = Repository::open(&repo_root) {
        if let Ok(remote) = repo.find_remote("origin") {
            origin_url = remote.url().map(|s| s.to_string());
        } else if let Ok(remotes) = repo.remotes() {
            for name in remotes.iter().flatten() {
                if let Ok(remote) = repo.find_remote(name) {
                    origin_url = remote.url().map(|s| s.to_string());
                    if origin_url.is_some() {
                        break;
                    }
                }
            }
        }
        if let Ok(head) = repo.head() {
            if head.is_branch() {
                if let Some(s) = head.shorthand() {
                    default_branch = Some(s.to_string());
                }
            }
            if let Ok(commit) = head.peel_to_commit() {
                last_commit_ts = Some(commit.time().seconds());
            }
        }
    }

    let readme_excerpt = read_readme_excerpt(&repo_root).ok();
    let now = Utc::now().timestamp();

    Ok(RepoMeta {
        path: repo_root.to_string_lossy().to_string(),
        name,
        default_branch,
        last_commit_ts,
        last_scan_ts: now,
        readme_excerpt,
        origin_url,
    })
}

fn read_readme_excerpt(repo_root: &Path) -> Result<String> {
    let candidates = ["README.md", "Readme.md", "README.MD", "README"];
    let readme = candidates
        .iter()
        .map(|n| repo_root.join(n))
        .find(|p| p.exists())
        .context("no readme")?;

    let s = std::fs::read_to_string(&readme).with_context(|| format!("read {}", readme.display()))?;
    let excerpt = s
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(10)
        .collect::<Vec<_>>()
        .join(" ");
    Ok(excerpt.chars().take(280).collect())
}
