use crate::db;
use anyhow::{Context, Result};
use git2::{BranchType, Repository};

pub fn build_commit_index_for_repo(
    repo_path: &str,
    branches_limit: usize,
    commits_per_branch: usize,
) -> Result<(Vec<db::CommitBranch>, Vec<db::CommitIndexRow>)> {
    let repo = Repository::open(repo_path).with_context(|| format!("open repo {}", repo_path))?;

    #[derive(Clone)]
    struct Tip {
        kind: String,
        name: String,
        refname: String,
        tip_time: Option<i64>,
        tip_oid: Option<git2::Oid>,
    }

    let mut tips: Vec<Tip> = Vec::new();
    for (kind, bt) in [("local", BranchType::Local), ("remote", BranchType::Remote)] {
        let iter = repo.branches(Some(bt))?;
        for b in iter {
            let (branch, _) = b?;
            let Some(name) = branch.name()?.map(|s| s.to_string()) else {
                continue;
            };
            if kind == "remote" && (name.ends_with("/HEAD") || name == "HEAD") {
                continue;
            }
            let Some(refname) = branch.get().name().map(|s| s.to_string()) else {
                continue;
            };
            let tip_oid = branch.get().target();
            let tip_time = tip_oid
                .and_then(|oid| repo.find_commit(oid).ok())
                .map(|c| c.time().seconds());
            tips.push(Tip {
                kind: kind.to_string(),
                name,
                refname,
                tip_time,
                tip_oid,
            });
        }
    }

    tips.sort_by(|a, b| b.tip_time.unwrap_or(0).cmp(&a.tip_time.unwrap_or(0)));
    tips.truncate(branches_limit.max(1));

    let branches = tips
        .iter()
        .map(|t| db::CommitBranch {
            kind: t.kind.clone(),
            name: t.name.clone(),
            refname: t.refname.clone(),
            tip_time: t.tip_time,
        })
        .collect::<Vec<_>>();

    let mut commits = Vec::new();
    for t in tips {
        let Some(oid) = t.tip_oid else { continue };
        let mut walk = repo.revwalk()?;
        walk.set_sorting(git2::Sort::TIME)?;
        walk.push(oid)?;
        for (i, oid) in walk.enumerate() {
            if i >= commits_per_branch.max(1) {
                break;
            }
            let oid = oid?;
            let commit = repo.find_commit(oid)?;
            let author = commit.author();
            commits.push(db::CommitIndexRow {
                refname: t.refname.clone(),
                branch_kind: t.kind.clone(),
                branch_name: t.name.clone(),
                oid: oid.to_string(),
                time: Some(commit.time().seconds()),
                author: author.name().map(|s| s.to_string()),
                email: author.email().map(|s| s.to_string()),
                summary: commit.summary().map(|s| s.to_string()),
                message: commit.message().map(|s| s.to_string()),
            });
        }
    }

    Ok((branches, commits))
}

