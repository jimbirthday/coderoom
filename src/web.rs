use crate::{commits, config, db, scan};
use anyhow::{Context, Result};
use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use git2::{BranchType, Repository};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone)]
pub struct AppState {
    pub cfg_path: PathBuf,
    pub db_path: PathBuf,
}

pub async fn serve(state: AppState, host: String, port: u16) -> Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/app.css", get(app_css))
        .route("/api/roots", get(api_roots))
        .route("/api/roots/pick", post(api_roots_pick))
        .route("/api/roots/add", post(api_roots_add))
        .route("/api/roots/remove", post(api_roots_remove))
        .route("/api/ignores/add", post(api_ignores_add))
        .route("/api/ignores/remove", post(api_ignores_remove))
        .route("/api/ignores/reset", post(api_ignores_reset))
        .route("/api/scan", post(api_scan))
        .route("/api/prune", post(api_prune))
        .route("/api/repos", get(api_repos))
        .route("/api/search", get(api_search))
        .route("/api/tags", get(api_tags))
        .route("/api/branches", get(api_branches))
        .route("/api/commits", get(api_commits))
        .route("/api/commit_detail", get(api_commit_detail))
        .route("/api/config", get(api_config))
        .route("/api/commit_index/rebuild", post(api_commit_index_rebuild))
        .route("/api/commit_search", get(api_commit_search))
        .route("/api/repos/tag", post(api_tag_add))
        .route("/api/repos/untag", post(api_tag_remove))
        .route("/api/open", post(api_open))
        .with_state(state);

    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .context("invalid bind address")?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("CodeRoom Web: http://{}/", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP_JS,
    )
}

async fn app_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], APP_CSS)
}

#[derive(Serialize)]
struct RootsResponse {
    roots: Vec<String>,
}

async fn api_roots(State(state): State<AppState>) -> Result<Json<RootsResponse>, ApiError> {
    let cfg = config::Config::load_or_create(&state.cfg_path).map_err(ApiError::from)?;
    Ok(Json(RootsResponse { roots: cfg.roots }))
}

#[derive(Serialize)]
struct PickRootResponse {
    root: Option<String>,
}

async fn api_roots_pick(
    State(_state): State<AppState>,
) -> Result<Json<PickRootResponse>, ApiError> {
    let root =
        tokio::task::spawn_blocking(move || -> Result<Option<String>> { pick_root_dialog() })
            .await
            .map_err(|e| ApiError::msg(format!("pick join error: {e}")))?
            .map_err(ApiError::from)?;
    Ok(Json(PickRootResponse { root }))
}

#[derive(Deserialize)]
struct RootBody {
    root: String,
}

async fn api_roots_add(
    State(state): State<AppState>,
    Json(body): Json<RootBody>,
) -> Result<StatusCode, ApiError> {
    let mut cfg = config::Config::load_or_create(&state.cfg_path).map_err(ApiError::from)?;
    cfg.add_root(Path::new(&body.root));
    cfg.save(&state.cfg_path).map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_roots_remove(
    State(state): State<AppState>,
    Json(body): Json<RootBody>,
) -> Result<StatusCode, ApiError> {
    let mut cfg = config::Config::load_or_create(&state.cfg_path).map_err(ApiError::from)?;
    cfg.remove_root(Path::new(&body.root));
    cfg.save(&state.cfg_path).map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct IgnoreBody {
    name: String,
}

async fn api_ignores_add(
    State(state): State<AppState>,
    Json(body): Json<IgnoreBody>,
) -> Result<StatusCode, ApiError> {
    let mut cfg = config::Config::load_or_create(&state.cfg_path).map_err(ApiError::from)?;
    cfg.add_ignore_dir_name(&body.name);
    cfg.save(&state.cfg_path).map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_ignores_remove(
    State(state): State<AppState>,
    Json(body): Json<IgnoreBody>,
) -> Result<StatusCode, ApiError> {
    let mut cfg = config::Config::load_or_create(&state.cfg_path).map_err(ApiError::from)?;
    cfg.remove_ignore_dir_name(&body.name);
    cfg.save(&state.cfg_path).map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_ignores_reset(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    let mut cfg = config::Config::load_or_create(&state.cfg_path).map_err(ApiError::from)?;
    cfg.reset_ignore_dir_names();
    cfg.save(&state.cfg_path).map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ScanBody {
    root: Option<String>,
    all: Option<bool>,
    max_depth: Option<usize>,
    prune: Option<bool>,
}

#[derive(Serialize)]
struct ScanResponse {
    indexed: usize,
    pruned: usize,
}

async fn api_scan(
    State(state): State<AppState>,
    Json(body): Json<ScanBody>,
) -> Result<Json<ScanResponse>, ApiError> {
    let cfg_path = state.cfg_path.clone();
    let db_path = state.db_path.clone();
    let root = body.root.clone();
    let all = body.all.unwrap_or(false);
    let max_depth = body.max_depth;
    let prune = body.prune.unwrap_or(false);

	    let out = tokio::task::spawn_blocking(move || -> Result<ScanResponse> {
	        let mut cfg = config::Config::load_or_create(&cfg_path)?;
	        let db = db::Db::open(&db_path)?;
	        db.init_schema()?;
	        let ignore_dir_names: HashSet<String> = cfg.ignore_dir_names.iter().cloned().collect();

	        let mut indexed = 0usize;
	        let mut pruned = 0usize;

	        if all || root.is_none() {
	            for r in cfg.roots.clone() {
	                let root_path = PathBuf::from(&r);
	                let (i, p) = scan_one_root(&db, &root_path, max_depth, prune, &ignore_dir_names)?;
	                indexed += i;
	                pruned += p;
	            }
	        } else if let Some(root) = root {
	            let root_path = PathBuf::from(&root);
	            let (i, p) = scan_one_root(&db, &root_path, max_depth, prune, &ignore_dir_names)?;
	            indexed += i;
	            pruned += p;
	            cfg.add_root(&root_path);
	        }

        cfg.save(&cfg_path)?;
        Ok(ScanResponse { indexed, pruned })
    })
    .await
    .map_err(|e| ApiError::msg(format!("scan join error: {e}")))?
    .map_err(ApiError::from)?;

    Ok(Json(out))
}

#[derive(Serialize)]
struct PruneResponse {
    deleted: usize,
}

async fn api_prune(State(state): State<AppState>) -> Result<Json<PruneResponse>, ApiError> {
    let db_path = state.db_path.clone();
    let out = tokio::task::spawn_blocking(move || -> Result<PruneResponse> {
        let db = db::Db::open(&db_path)?;
        db.init_schema()?;
        let deleted = db.prune_missing_paths()?;
        Ok(PruneResponse { deleted })
    })
    .await
    .map_err(|e| ApiError::msg(format!("prune join error: {e}")))?
    .map_err(ApiError::from)?;
    Ok(Json(out))
}

#[derive(Deserialize)]
struct ReposQuery {
    tag: Option<String>,
    recent: Option<bool>,
    page: Option<usize>,
    per_page: Option<usize>,
}

#[derive(Serialize)]
struct RepoDto {
    id: i64,
    path: String,
    name: String,
    default_branch: Option<String>,
    last_commit_ts: Option<i64>,
    last_scan_ts: i64,
    last_access_ts: Option<i64>,
    readme_excerpt: Option<String>,
    origin_url: Option<String>,
    tags: Vec<String>,
    matched_in: Option<Vec<String>>,
}

#[derive(Serialize)]
struct PagedReposResponse {
    total: usize,
    page: usize,
    per_page: usize,
    items: Vec<RepoDto>,
}

async fn api_repos(
    State(state): State<AppState>,
    Query(q): Query<ReposQuery>,
) -> Result<Json<PagedReposResponse>, ApiError> {
    let tag = q.tag.clone();
    let recent = q.recent.unwrap_or(false);
    let page = q.page.unwrap_or(1);
    let per_page = q.per_page.unwrap_or(25);
    let db_path = state.db_path.clone();

    let out = tokio::task::spawn_blocking(move || -> Result<PagedReposResponse> {
        let db = db::Db::open(&db_path)?;
        db.init_schema()?;
        let paged = db.list_repos_with_tags_paged(tag.as_deref(), recent, page, per_page)?;
        let items = paged
            .items
            .into_iter()
            .map(|r| RepoDto {
                id: r.repo.id,
                path: r.repo.path,
                name: r.repo.name,
                default_branch: r.repo.default_branch,
                last_commit_ts: r.repo.last_commit_ts,
                last_scan_ts: r.repo.last_scan_ts,
                last_access_ts: r.repo.last_access_ts,
                readme_excerpt: r.repo.readme_excerpt,
                origin_url: r.repo.origin_url,
                tags: r.tags,
                matched_in: None,
            })
            .collect::<Vec<_>>();
        Ok(PagedReposResponse {
            total: paged.total,
            page,
            per_page,
            items,
        })
    })
    .await
    .map_err(|e| ApiError::msg(format!("repos join error: {e}")))?
    .map_err(ApiError::from)?;

    Ok(Json(out))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    page: Option<usize>,
    per_page: Option<usize>,
    in_name: Option<bool>,
    in_path: Option<bool>,
    in_readme: Option<bool>,
    in_tags: Option<bool>,
}

async fn api_search(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<PagedReposResponse>, ApiError> {
    let db_path = state.db_path.clone();
    let query = q.q.clone();
    let page = q.page.unwrap_or(1);
    let per_page = q.per_page.unwrap_or(25);
    let in_name = q.in_name.unwrap_or(true);
    let in_path = q.in_path.unwrap_or(true);
    let in_readme = q.in_readme.unwrap_or(true);
    let in_tags = q.in_tags.unwrap_or(true);

    let out = tokio::task::spawn_blocking(move || -> Result<PagedReposResponse> {
        let db = db::Db::open(&db_path)?;
        db.init_schema()?;
        let paged = db.search_repos_with_tags_paged_filtered(
            &query, in_name, in_path, in_readme, in_tags, page, per_page,
        )?;
        let qlow = query.to_lowercase();
        let items = paged
            .items
            .into_iter()
            .map(|r| {
                let mut matched = Vec::<String>::new();
                if in_name && r.repo.name.to_lowercase().contains(&qlow) {
                    matched.push("name".to_string());
                }
                if in_path && r.repo.path.to_lowercase().contains(&qlow) {
                    matched.push("path".to_string());
                }
                if in_readme {
                    if let Some(ex) = &r.repo.readme_excerpt {
                        if ex.to_lowercase().contains(&qlow) {
                            matched.push("readme".to_string());
                        }
                    }
                }
                if in_tags && r.tags.iter().any(|t| t.to_lowercase().contains(&qlow)) {
                    matched.push("tag".to_string());
                }
                if matched.is_empty() {
                    matched.push("repo".to_string());
                }
                RepoDto {
                    id: r.repo.id,
                    path: r.repo.path,
                    name: r.repo.name,
                    default_branch: r.repo.default_branch,
                    last_commit_ts: r.repo.last_commit_ts,
                    last_scan_ts: r.repo.last_scan_ts,
                    last_access_ts: r.repo.last_access_ts,
                    readme_excerpt: r.repo.readme_excerpt,
                    origin_url: r.repo.origin_url,
                    tags: r.tags,
                    matched_in: Some(matched),
                }
            })
            .collect::<Vec<_>>();
        Ok(PagedReposResponse {
            total: paged.total,
            page,
            per_page,
            items,
        })
    })
    .await
    .map_err(|e| ApiError::msg(format!("search join error: {e}")))?
    .map_err(ApiError::from)?;

    Ok(Json(out))
}

#[derive(Serialize)]
struct TagCountDto {
    name: String,
    count: usize,
}

async fn api_tags(State(state): State<AppState>) -> Result<Json<Vec<TagCountDto>>, ApiError> {
    let db_path = state.db_path.clone();
    let tags = tokio::task::spawn_blocking(move || -> Result<Vec<TagCountDto>> {
        let db = db::Db::open(&db_path)?;
        db.init_schema()?;
        let rows = db.list_tags_with_count()?;
        Ok(rows
            .into_iter()
            .map(|(name, count)| TagCountDto { name, count })
            .collect())
    })
    .await
    .map_err(|e| ApiError::msg(format!("tags join error: {e}")))?
    .map_err(ApiError::from)?;
    Ok(Json(tags))
}

#[derive(Serialize)]
struct ConfigDto {
    commit_index_branches: usize,
    commit_index_commits_per_branch: usize,
    ignore_dir_names: Vec<String>,
}

async fn api_config(State(state): State<AppState>) -> Result<Json<ConfigDto>, ApiError> {
    let cfg = config::Config::load_or_create(&state.cfg_path).map_err(ApiError::from)?;
    Ok(Json(ConfigDto {
        commit_index_branches: cfg.commit_index_branches,
        commit_index_commits_per_branch: cfg.commit_index_commits_per_branch,
        ignore_dir_names: cfg.ignore_dir_names,
    }))
}

#[derive(Deserialize)]
struct CommitIndexRebuildBody {
    repo_path: Option<String>,
    all: Option<bool>,
    commit_index_branches: Option<usize>,
    commit_index_commits_per_branch: Option<usize>,
}

#[derive(Serialize)]
struct CommitIndexRebuildResponse {
    repos_indexed: usize,
    branches: usize,
    commits_per_branch: usize,
}

async fn api_commit_index_rebuild(
    State(state): State<AppState>,
    Json(body): Json<CommitIndexRebuildBody>,
) -> Result<Json<CommitIndexRebuildResponse>, ApiError> {
    let cfg_path = state.cfg_path.clone();
    let db_path = state.db_path.clone();
    let repo_path = body.repo_path.clone();
    let all = body.all.unwrap_or(false);
    let set_branches = body.commit_index_branches;
    let set_commits = body.commit_index_commits_per_branch;

    let out = tokio::task::spawn_blocking(move || -> Result<CommitIndexRebuildResponse> {
        let mut cfg = config::Config::load_or_create(&cfg_path)?;
        if let Some(v) = set_branches {
            cfg.commit_index_branches = v.max(1).min(200);
        }
        if let Some(v) = set_commits {
            cfg.commit_index_commits_per_branch = v.max(1).min(500);
        }
        cfg.save(&cfg_path)?;

        let db = db::Db::open(&db_path)?;
        db.init_schema()?;

        let targets: Vec<String> = if all || repo_path.is_none() {
            db.list_repo_paths()?
        } else {
            vec![repo_path.unwrap()]
        };

        let mut repos_indexed = 0usize;
        for p in targets {
            if !Path::new(&p).exists() {
                continue;
            }
            let (branches, commits) = commits::build_commit_index_for_repo(
                &p,
                cfg.commit_index_branches,
                cfg.commit_index_commits_per_branch,
            )?;
            db.replace_commit_index_for_repo(&p, &branches, &commits)?;
            repos_indexed += 1;
        }

        Ok(CommitIndexRebuildResponse {
            repos_indexed,
            branches: cfg.commit_index_branches,
            commits_per_branch: cfg.commit_index_commits_per_branch,
        })
    })
    .await
    .map_err(|e| ApiError::msg(format!("commit index join error: {e}")))?
    .map_err(ApiError::from)?;

    Ok(Json(out))
}

#[derive(Deserialize)]
struct CommitSearchQuery {
    q: String,
    branch: Option<String>,
    in_summary: Option<bool>,
    in_message: Option<bool>,
    page: Option<usize>,
    per_page: Option<usize>,
}

#[derive(Serialize)]
struct CommitHitDto {
    repo_name: String,
    repo_path: String,
    branch_kind: String,
    branch_name: String,
    refname: String,
    oid: String,
    time: Option<i64>,
    summary: Option<String>,
    snippet: Option<String>,
    matched_in: Vec<String>,
}

#[derive(Serialize)]
struct CommitSearchResponse {
    total: usize,
    page: usize,
    per_page: usize,
    items: Vec<CommitHitDto>,
}

async fn api_commit_search(
    State(state): State<AppState>,
    Query(q): Query<CommitSearchQuery>,
) -> Result<Json<CommitSearchResponse>, ApiError> {
    let db_path = state.db_path.clone();
    let query = q.q.clone();
    let branch = q.branch.clone();
    let in_summary = q.in_summary.unwrap_or(true);
    let in_message = q.in_message.unwrap_or(true);
    let page = q.page.unwrap_or(1);
    let per_page = q.per_page.unwrap_or(25);

    let out = tokio::task::spawn_blocking(move || -> Result<CommitSearchResponse> {
        let db = db::Db::open(&db_path)?;
        db.init_schema()?;
        let paged = db.search_commits_paged(
            &query,
            branch.as_deref(),
            in_summary,
            in_message,
            page,
            per_page,
        )?;
        let qlow = query.to_lowercase();
        Ok(CommitSearchResponse {
            total: paged.total,
            page,
            per_page,
            items: paged
                .items
                .into_iter()
                .map(|c| {
                    let mut matched = Vec::<String>::new();
                    if in_summary {
                        if let Some(s) = &c.summary {
                            if s.to_lowercase().contains(&qlow) {
                                matched.push("summary".to_string());
                            }
                        }
                    }
                    if in_message {
                        if let Some(m) = &c.message {
                            if m.to_lowercase().contains(&qlow) {
                                matched.push("message".to_string());
                            }
                        }
                    }
                    if matched.is_empty() {
                        matched.push("commit".to_string());
                    }

                    let snippet = make_snippet(c.summary.as_deref(), c.message.as_deref(), &qlow);

                    CommitHitDto {
                        repo_name: c.repo_name,
                        repo_path: c.repo_path,
                        branch_kind: c.branch_kind,
                        branch_name: c.branch_name,
                        refname: c.refname,
                        oid: c.oid,
                        time: c.time,
                        summary: c.summary,
                        snippet,
                        matched_in: matched,
                    }
                })
                .collect(),
        })
    })
    .await
    .map_err(|e| ApiError::msg(format!("commit search join error: {e}")))?
    .map_err(ApiError::from)?;

    Ok(Json(out))
}

#[derive(Deserialize)]
struct BranchesQuery {
    repo_path: String,
}

#[derive(Serialize)]
struct BranchDto {
    kind: String,
    name: String,
    refname: String,
}

async fn api_branches(
    State(_state): State<AppState>,
    Query(q): Query<BranchesQuery>,
) -> Result<Json<Vec<BranchDto>>, ApiError> {
    let repo_path = q.repo_path.clone();
    let branches = tokio::task::spawn_blocking(move || -> Result<Vec<BranchDto>> {
        let repo =
            Repository::open(&repo_path).with_context(|| format!("open repo {}", repo_path))?;
        let mut out = Vec::new();

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
                let Some(reference) = branch.get().name().map(|s| s.to_string()) else {
                    continue;
                };
                out.push(BranchDto {
                    kind: kind.to_string(),
                    name,
                    refname: reference,
                });
            }
        }

        out.sort_by(|a, b| {
            (a.kind.as_str(), a.name.as_str()).cmp(&(b.kind.as_str(), b.name.as_str()))
        });
        out.dedup_by(|a, b| a.refname == b.refname);
        Ok(out)
    })
    .await
    .map_err(|e| ApiError::msg(format!("branches join error: {e}")))?
    .map_err(ApiError::from)?;

    Ok(Json(branches))
}

#[derive(Deserialize)]
struct CommitsQuery {
    repo_path: String,
    refname: String,
    page: Option<usize>,
    per_page: Option<usize>,
}

#[derive(Serialize)]
struct CommitDto {
    oid: String,
    summary: String,
    author: String,
    email: String,
    time: i64,
}

#[derive(Serialize)]
struct CommitsResponse {
    page: usize,
    per_page: usize,
    has_more: bool,
    items: Vec<CommitDto>,
}

async fn api_commits(
    State(_state): State<AppState>,
    Query(q): Query<CommitsQuery>,
) -> Result<Json<CommitsResponse>, ApiError> {
    let repo_path = q.repo_path.clone();
    let refname = q.refname.clone();
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * per_page;

    let out = tokio::task::spawn_blocking(move || -> Result<CommitsResponse> {
        let repo =
            Repository::open(&repo_path).with_context(|| format!("open repo {}", repo_path))?;
        let obj = repo
            .revparse_single(&refname)
            .with_context(|| format!("resolve ref {refname}"))?;
        let oid = obj.id();

        let mut walk = repo.revwalk()?;
        walk.set_sorting(git2::Sort::TIME)?;
        walk.push(oid)?;

        let mut items = Vec::new();
        let mut idx = 0usize;
        let mut has_more = false;

        for oid in walk {
            let oid = oid?;
            if idx < offset {
                idx += 1;
                continue;
            }
            if items.len() >= per_page {
                has_more = true;
                break;
            }
            let commit = repo.find_commit(oid)?;
            let author = commit.author();
            items.push(CommitDto {
                oid: oid.to_string(),
                summary: commit.summary().unwrap_or("").to_string(),
                author: author.name().unwrap_or("").to_string(),
                email: author.email().unwrap_or("").to_string(),
                time: commit.time().seconds(),
            });
            idx += 1;
        }

        Ok(CommitsResponse {
            page,
            per_page,
            has_more,
            items,
        })
    })
    .await
    .map_err(|e| ApiError::msg(format!("commits join error: {e}")))?
    .map_err(ApiError::from)?;

    Ok(Json(out))
}

#[derive(Deserialize)]
struct CommitDetailQuery {
    repo_path: String,
    oid: String,
}

#[derive(Serialize)]
struct CommitDetailDto {
    oid: String,
    summary: String,
    message: String,
    author: String,
    email: String,
    time: i64,
    parents: Vec<String>,
}

async fn api_commit_detail(
    State(_state): State<AppState>,
    Query(q): Query<CommitDetailQuery>,
) -> Result<Json<CommitDetailDto>, ApiError> {
    let repo_path = q.repo_path.clone();
    let oid = q.oid.clone();
    let out = tokio::task::spawn_blocking(move || -> Result<CommitDetailDto> {
        let repo =
            Repository::open(&repo_path).with_context(|| format!("open repo {}", repo_path))?;
        let oid = git2::Oid::from_str(&oid).context("invalid oid")?;
        let commit = repo.find_commit(oid)?;
        let author = commit.author();
        let parents = (0..commit.parent_count())
            .filter_map(|i| commit.parent_id(i).ok())
            .map(|o| o.to_string())
            .collect::<Vec<_>>();
        Ok(CommitDetailDto {
            oid: oid.to_string(),
            summary: commit.summary().unwrap_or("").to_string(),
            message: commit.message().unwrap_or("").to_string(),
            author: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
            time: commit.time().seconds(),
            parents,
        })
    })
    .await
    .map_err(|e| ApiError::msg(format!("commit detail join error: {e}")))?
    .map_err(ApiError::from)?;
    Ok(Json(out))
}

#[derive(Deserialize)]
struct TagBody {
    repo_path: String,
    tag: String,
}

async fn api_tag_add(
    State(state): State<AppState>,
    Json(body): Json<TagBody>,
) -> Result<StatusCode, ApiError> {
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let db = db::Db::open(&db_path)?;
        db.init_schema()?;
        db.add_tag_to_repo(&body.repo_path, &body.tag)?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::msg(format!("tag add join error: {e}")))?
    .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_tag_remove(
    State(state): State<AppState>,
    Json(body): Json<TagBody>,
) -> Result<StatusCode, ApiError> {
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let db = db::Db::open(&db_path)?;
        db.init_schema()?;
        db.remove_tag_from_repo(&body.repo_path, &body.tag)?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::msg(format!("tag remove join error: {e}")))?
    .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct OpenBody {
    repo: String,
}

#[derive(Serialize)]
struct OpenResponse {
    path: String,
}

async fn api_open(
    State(state): State<AppState>,
    Json(body): Json<OpenBody>,
) -> Result<Json<OpenResponse>, ApiError> {
    let db_path = state.db_path.clone();
    let input = body.repo.clone();
    let out = tokio::task::spawn_blocking(move || -> Result<OpenResponse> {
        let db = db::Db::open(&db_path)?;
        db.init_schema()?;
        let path = db.resolve_repo_path(&input)?.context("repo not found")?;
        db.record_access(&path)?;
        Ok(OpenResponse { path })
    })
    .await
    .map_err(|e| ApiError::msg(format!("open join error: {e}")))?
    .map_err(ApiError::from)?;
    Ok(Json(out))
}

fn scan_one_root(
    db: &db::Db,
    root: &Path,
    max_depth: Option<usize>,
    prune: bool,
    ignore_dir_names: &HashSet<String>,
) -> Result<(usize, usize)> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let repos = scan::discover_git_repos(&root, max_depth, ignore_dir_names)
        .with_context(|| format!("scan root {}", root.display()))?;
    let mut keep = HashSet::<String>::new();
    for repo_root in repos {
        let meta = scan::read_repo_metadata(&repo_root)?;
        keep.insert(meta.path.clone());
        db.upsert_repo(&meta)?;
    }
    let pruned = if prune {
        db.prune_under_root(&root.to_string_lossy(), &keep)?
    } else {
        0
    };
    Ok((keep.len(), pruned))
}

#[derive(Debug)]
struct ApiError(anyhow::Error);

impl ApiError {
    fn msg(s: String) -> Self {
        Self(anyhow::anyhow!(s))
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({ "error": self.0.to_string() });
        (StatusCode::BAD_REQUEST, Json(body)).into_response()
    }
}

fn pick_root_dialog() -> Result<Option<String>> {
    #[cfg(target_os = "macos")]
    {
        let out = Command::new("osascript")
            .args([
                "-e",
                "try\nPOSIX path of (choose folder with prompt \"Choose a root folder for CodeRoom\")\non error\n\"\"\nend try",
            ])
            .output()
            .context("run osascript")?;
        if !out.status.success() {
            return Ok(None);
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            return Ok(None);
        }
        return Ok(Some(s));
    }

    #[cfg(target_os = "windows")]
    {
        // Best-effort: use built-in PowerShell + WinForms folder picker.
        let script = r#"
Add-Type -AssemblyName System.Windows.Forms;
$f = New-Object System.Windows.Forms.FolderBrowserDialog;
$f.Description = 'Choose a root folder for CodeRoom';
if ($f.ShowDialog() -eq 'OK') { $f.SelectedPath } else { '' }
"#;
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
            .or_else(|_| {
                Command::new("pwsh")
                    .args(["-NoProfile", "-NonInteractive", "-Command", script])
                    .output()
            })
            .context("run powershell folder picker")?;
        if !out.status.success() {
            return Ok(None);
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            return Ok(None);
        }
        return Ok(Some(s));
    }

    #[cfg(target_os = "linux")]
    {
        // Best-effort: prefer common desktop helpers if present.
        let candidates: [(&str, &[&str]); 3] = [
            (
                "zenity",
                &[
                    "--file-selection",
                    "--directory",
                    "--title=Choose a root folder for CodeRoom",
                ],
            ),
            (
                "kdialog",
                &[
                    "--getexistingdirectory",
                    "--title",
                    "Choose a root folder for CodeRoom",
                ],
            ),
            (
                "yad",
                &[
                    "--file-selection",
                    "--directory",
                    "--title=Choose a root folder for CodeRoom",
                ],
            ),
        ];

        for (cmd, args) in candidates {
            let out = Command::new(cmd).args(args).output();
            let out = match out {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !out.status.success() {
                continue;
            }
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() {
                continue;
            }
            return Ok(Some(s));
        }
        Ok(None)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Ok(None)
    }
}

fn make_snippet(summary: Option<&str>, message: Option<&str>, qlow: &str) -> Option<String> {
    let candidates: Vec<&str> = [summary, message].into_iter().flatten().collect();
    for text in candidates {
        let lower = text.to_lowercase();
        if let Some(pos) = lower.find(qlow) {
            let start = pos.saturating_sub(60);
            let end = (pos + qlow.len() + 60).min(text.len());
            let mut s = text[start..end].to_string();
            if start > 0 {
                s = format!("…{s}");
            }
            if end < text.len() {
                s.push('…');
            }
            return Some(s);
        }
    }
    None
}

const INDEX_HTML: &str = r##"<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>CodeRoom</title>
    <link rel="stylesheet" href="/app.css" />
  </head>
  <body>
    <header>
      <div class="topbar container">
        <div class="brand">
          <div class="logo">CR</div>
          <div class="brand-text">
            <div class="title">CodeRoom</div>
            <div class="subtitle" data-i18n="subtitle">本地仓库管理与索引（离线）</div>
          </div>
        </div>
        <div class="actions">
          <button id="btnLang" class="ghost" title="Language / 语言">中文</button>
        </div>
      </div>
    </header>

    <main class="layout container">
      <aside class="sidebar">
        <div class="panel">
          <div class="panel-head">
            <h2 data-i18n="rootsTitle">Roots</h2>
            <div class="panel-actions">
              <button id="btnPickRoot" class="ghost" data-i18n="pickBtn">选择…</button>
              <button id="btnAddRoot" data-i18n="addBtn">添加</button>
            </div>
          </div>
          <div class="hint" data-i18n="rootsHint">只添加常用开发目录，避免扫描系统目录。</div>
          <div class="row">
            <input id="root" placeholder="root 目录（例如：/Users/jim/dev）" />
          </div>
          <ul id="roots" class="list"></ul>
        </div>

        <div class="panel">
          <div class="panel-head">
            <h2 data-i18n="tagsTitle">Tags</h2>
            <button id="btnClearTag" class="ghost" data-i18n="clearFilterBtn">清除筛选</button>
          </div>
          <div id="tags" class="chips"></div>
        </div>

        <div class="panel">
          <div class="panel-head">
            <h2 data-i18n="settingsTitle">设置</h2>
          </div>
          <div class="hint" data-i18n="commitIndexHint">提交搜索依赖本地索引；修改范围后请重建索引。</div>
          <div class="row">
            <label class="meta"><span data-i18n="indexBranches">分支数</span></label>
            <input id="idxBranches" type="number" min="1" max="200" />
          </div>
          <div class="row">
            <label class="meta"><span data-i18n="indexCommits">每分支提交数</span></label>
            <input id="idxCommits" type="number" min="1" max="500" />
          </div>
          <div class="row">
            <button id="rebuildIndex" class="ghost" data-i18n="rebuildIndex">重建索引</button>
            <div id="idxStatus" class="meta"></div>
          </div>

          <div class="hint" style="margin-top:12px;" data-i18n="ignoreHint">扫描时忽略常见依赖/缓存目录（目录名匹配）。</div>
          <div class="row">
            <input id="ignoreName" placeholder=".cargo_home" />
            <button id="ignoreAdd" class="ghost" data-i18n="addBtn">添加</button>
            <button id="ignoreReset" class="ghost" data-i18n="resetBtn">重置</button>
          </div>
          <ul id="ignores" class="list"></ul>
        </div>
      </aside>

      <section class="content">
        <div class="panel">
          <div class="toolbar">
            <div class="search">
              <div class="search-mode-tabs">
                <label class="mode-tab">
                  <input type="radio" name="searchMode" id="scopeRepos" value="repos" checked />
                  <span data-i18n="scopeRepos">仓库</span>
                </label>
                <label class="mode-tab">
                  <input type="radio" name="searchMode" id="scopeCommits" value="commits" />
                  <span data-i18n="scopeCommits">提交</span>
                </label>
              </div>
              <div class="search-filters">
                <div class="filter-group" data-mode="repos">
                  <label class="filter-item">
                    <input id="inName" type="checkbox" checked />
                    <span data-i18n="inName">名称</span>
                  </label>
                  <label class="filter-item">
                    <input id="inPath" type="checkbox" checked />
                    <span data-i18n="inPath">路径</span>
                  </label>
                  <label class="filter-item">
                    <input id="inReadme" type="checkbox" checked />
                    <span data-i18n="inReadme">README</span>
                  </label>
                  <label class="filter-item">
                    <input id="inTags" type="checkbox" checked />
                    <span data-i18n="inTags">标签</span>
                  </label>
                </div>
                <div class="filter-group hidden" data-mode="commits">
                  <label class="filter-item">
                    <input id="inSummary" type="checkbox" checked />
                    <span data-i18n="inSummary">摘要</span>
                  </label>
                  <label class="filter-item">
                    <input id="inMessage" type="checkbox" checked />
                    <span data-i18n="inMessage">正文</span>
                  </label>
                  <input id="branchFilter" class="branch-filter" placeholder="分支（可选）" />
                </div>
              </div>
              <div class="search-input-row">
                <input id="q" placeholder="搜索：仓库名 / 路径 / README / 标签" />
                <button id="btnSearch" data-i18n="searchBtn">搜索</button>
                <button id="btnAll" class="ghost" data-i18n="allBtn">全部</button>
              </div>
            </div>
            <div class="toolbar-right">
              <label class="checkbox"><input id="recent" type="checkbox" /> <span data-i18n="recentFirst">最近访问优先</span></label>
              <label class="checkbox"><input id="prune" type="checkbox" /> <span data-i18n="pruneMoved">清理已删除/移动</span></label>
              <button id="btnScanAll" data-i18n="scanAllBtn">扫描全部</button>
              <button id="btnPrune" class="ghost" data-i18n="pruneMissingBtn">清理缺失</button>
            </div>
          </div>

          <div class="statusbar">
            <div id="status" class="status"></div>
            <div class="status-right">
              <div id="counts" class="meta"></div>
              <div class="meta" data-i18n="hintUsage">提示：选择“提交”后可搜索提交内容（需要先重建索引）。</div>
              <div class="pager">
                <button id="btnBulk" class="ghost small" data-i18n="bulkTagBtn">批量标签</button>
                <span id="bulkCount" class="meta hidden"></span>
                <input id="bulkTag" class="branch-filter hidden" placeholder="tag" />
                <button id="applyBulkTag" class="ghost small hidden" data-i18n="apply">应用</button>
                <button id="clearBulk" class="ghost small hidden" data-i18n="clear">清除</button>
              </div>
              <div class="pager">
                <label class="meta">
                  <span data-i18n="perPage">每页</span>
                  <select id="perPage" class="select">
                    <option value="25">25</option>
                    <option value="50">50</option>
                    <option value="100">100</option>
                  </select>
                </label>
                <button id="prevPage" class="ghost small" data-i18n="prev">上一页</button>
                <div id="pageInfo" class="meta"></div>
                <button id="nextPage" class="ghost small" data-i18n="next">下一页</button>
              </div>
            </div>
          </div>

          <div class="table-wrap">
            <table class="table">
              <thead id="tableHead">
                <tr>
                  <th data-i18n="colName">名称</th>
                  <th data-i18n="colTags">标签</th>
                  <th data-i18n="colBranch">分支</th>
                  <th data-i18n="colAccess">最近访问</th>
                  <th data-i18n="colActions">操作</th>
                </tr>
              </thead>
              <tbody id="repos"></tbody>
            </table>
          </div>
        </div>
      </section>
    </main>

    <div id="toast" class="toast" aria-live="polite"></div>

	    <div id="commitModal" class="modal hidden" role="dialog" aria-modal="true">
      <div class="modal-backdrop" id="commitClose"></div>
      <div class="modal-card">
        <div class="modal-head">
          <div class="modal-title" data-i18n="commitsTitle">Commits</div>
          <button id="commitX" class="ghost small">×</button>
        </div>
        <div class="modal-sub">
          <div id="commitRepo" class="mono truncate"></div>
          <label class="meta">
            <span data-i18n="branch">分支</span>
            <select id="branchSelect" class="select"></select>
          </label>
        </div>
        <div class="modal-body">
          <div id="commitList" class="commit-list"></div>
        </div>
        <div class="modal-foot">
          <button id="commitPrev" class="ghost small" data-i18n="prev">上一页</button>
          <div id="commitPageInfo" class="meta"></div>
          <button id="commitNext" class="ghost small" data-i18n="next">下一页</button>
        </div>
      </div>
	    </div>

	    <div id="repoModal" class="modal hidden" role="dialog" aria-modal="true">
	      <div class="modal-backdrop" id="repoClose"></div>
	      <div class="modal-card">
	        <div class="modal-head">
	          <div class="modal-title" data-i18n="repoTitle">仓库详情</div>
	          <button id="repoX" class="ghost small">×</button>
	        </div>
	        <div class="modal-body">
	          <div id="repoName" class="repo-name"></div>
	          <div id="repoPath" class="mono truncate" style="margin-top:6px;"></div>
	          <div id="repoOrigin" class="mono truncate" style="margin-top:6px;"></div>
	          <div id="repoAbout" class="meta" style="margin-top:10px; white-space: pre-wrap;"></div>
	          <div class="hint" data-i18n="repoTagsHint" style="margin-top:10px;">标签：</div>
	          <div id="repoTags" class="badges"></div>
	        </div>
	        <div class="modal-foot">
	          <button id="repoCopy" class="ghost small" data-i18n="copy">复制</button>
	          <button id="repoCommits" class="ghost small" data-i18n="commitsBtn">提交</button>
	        </div>
	      </div>
	    </div>

	    <div id="commitDetailModal" class="modal hidden" role="dialog" aria-modal="true">
	      <div class="modal-backdrop" id="commitDetailClose"></div>
	      <div class="modal-card">
	        <div class="modal-head">
	          <div class="modal-title" data-i18n="commitDetailTitle">提交详情</div>
	          <button id="commitDetailX" class="ghost small">×</button>
	        </div>
	        <div class="modal-body">
	          <div id="cdSummary" class="commit-msg"></div>
	          <div id="cdMeta" class="commit-meta" style="margin-top:8px;"></div>
	          <pre id="cdMessage" class="mono" style="margin-top:10px; white-space: pre-wrap;"></pre>
	        </div>
	      </div>
	    </div>

	    <script src="/app.js"></script>
	  </body>
</html>
"##;

const APP_CSS: &str = r##"
* { box-sizing: border-box; }
:root {
  --bg: #0b0f19;
  --card: #0f172a;
  --text: #e5e7eb;
  --muted: #9ca3af;
  --border: rgba(255,255,255,0.08);
  --accent: #60a5fa;
  --danger: #fb7185;
  --hover-bg: rgba(255,255,255,0.05);
  --active-bg: rgba(96,165,250,0.12);
}
body {
  font-family: ui-sans-serif, system-ui, -apple-system, sans-serif;
  margin: 0;
  background: radial-gradient(1200px 800px at 20% -10%, rgba(96,165,250,0.25), transparent 60%), var(--bg);
  color: var(--text);
  line-height: 1.5;
}
html, body { overflow-x: hidden; }
mark {
  background: rgba(96,165,250,0.22);
  color: var(--text);
  padding: 0 2px;
  border-radius: 4px;
}
header {
  border-bottom: 1px solid var(--border);
  background: rgba(11,15,25,0.6);
  backdrop-filter: blur(12px);
  position: sticky;
  top: 0;
  z-index: 10;
}
.container { max-width: 1400px; margin: 0 auto; padding: 20px; }
.topbar { display: flex; justify-content: space-between; align-items: center; }
.brand { display: flex; gap: 12px; align-items: center; }
.logo {
  width: 40px; height: 40px; border-radius: 12px;
  display: grid; place-items: center;
  background: linear-gradient(135deg, rgba(96,165,250,0.35), rgba(52,211,153,0.25));
  border: 1px solid var(--border);
  font-weight: 700;
  font-size: 16px;
}
.title { font-weight: 700; font-size: 18px; }
.subtitle { color: var(--muted); font-size: 13px; margin-top: 2px; }

.layout { display: grid; grid-template-columns: 380px 1fr; gap: 20px; padding-top: 20px; }
@media (max-width: 1024px) { .layout { grid-template-columns: 1fr; } }
.sidebar, .content { min-width: 0; }

.panel { border: 1px solid var(--border); background: rgba(15,23,42,0.7); border-radius: 16px; padding: 16px; min-width: 0; overflow: hidden; box-shadow: 0 2px 8px rgba(0,0,0,0.1); }
.panel-head { 
  display: flex; 
  justify-content: space-between; 
  align-items: center; 
  gap: 12px; 
  margin-bottom: 12px;
}
.panel-actions { 
  display: flex; 
  gap: 8px; 
}
h2 { 
  margin: 0; 
  font-size: 15px; 
  font-weight: 600;
  letter-spacing: 0.2px; 
  color: var(--text);
}
.hint { 
  color: var(--muted); 
  font-size: 12px; 
  margin: 8px 0; 
  line-height: 1.5;
}
.row { 
  display: flex; 
  gap: 10px; 
  align-items: center; 
  flex-wrap: wrap; 
  margin-top: 10px; 
}

input {
  padding: 10px 14px;
  width: 100%;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: rgba(2,6,23,0.6);
  color: var(--text);
  outline: none;
  font-size: 13px;
  transition: all 0.2s;
}
input:focus {
  border-color: rgba(96,165,250,0.5);
  background: rgba(2,6,23,0.7);
  box-shadow: 0 0 0 3px rgba(96,165,250,0.1);
}
input::placeholder { 
  color: rgba(156,163,175,0.6); 
}
button {
  padding: 10px 16px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: rgba(96,165,250,0.15);
  color: var(--text);
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  transition: all 0.2s;
  white-space: nowrap;
}
button:hover:not(:disabled) { 
  border-color: rgba(96,165,250,0.5); 
  background: rgba(96,165,250,0.22);
  transform: translateY(-1px);
}
button:active:not(:disabled) {
  transform: translateY(0);
}
button.ghost { 
  background: rgba(255,255,255,0.05); 
}
button.ghost:hover:not(:disabled) {
  background: rgba(255,255,255,0.1);
  border-color: rgba(255,255,255,0.15);
}
button.danger { 
  background: rgba(251,113,133,0.12); 
  border-color: rgba(251,113,133,0.3);
}
button.danger:hover:not(:disabled) {
  background: rgba(251,113,133,0.2);
  border-color: rgba(251,113,133,0.4);
}
button:disabled { 
  opacity: 0.5; 
  cursor: not-allowed; 
  transform: none !important;
}
.checkbox { color: var(--muted); font-size: 13px; user-select: none; display: inline-flex; gap: 8px; align-items: center; }
.checkbox input { width: auto; }

.list { 
  list-style: none; 
  padding: 0; 
  margin: 12px 0 0 0; 
  display: grid; 
  gap: 8px; 
}
.list li { 
  display: flex; 
  align-items: center; 
  justify-content: space-between; 
  gap: 10px; 
  padding: 10px 12px; 
  border: 1px solid var(--border); 
  background: rgba(2,6,23,0.4); 
  border-radius: 10px;
  transition: all 0.15s;
}
.list li:hover {
  background: rgba(2,6,23,0.5);
  border-color: rgba(96,165,250,0.3);
}
.mono { 
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace; 
  font-size: 12px; 
  color: var(--muted);
  line-height: 1.5;
}

.chips { 
  display: flex; 
  flex-wrap: wrap; 
  gap: 8px; 
  margin-top: 12px; 
}
.chip { 
  padding: 8px 14px; 
  border: 1px solid var(--border); 
  border-radius: 8px; 
  background: rgba(2,6,23,0.4); 
  color: var(--text); 
  font-size: 12px; 
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s;
}
.chip:hover {
  background: rgba(2,6,23,0.5);
  border-color: rgba(96,165,250,0.3);
}
.chip.active { 
  border-color: rgba(96,165,250,0.55); 
  background: rgba(96,165,250,0.18);
  color: var(--accent);
}
.search-mode-tabs { display: inline-flex; gap: 0; margin-bottom: 12px; border: 1px solid var(--border); border-radius: 10px; background: rgba(2,6,23,0.35); padding: 2px; }
.mode-tab { display: inline-flex; align-items: center; padding: 8px 16px; border-radius: 8px; font-size: 13px; color: var(--muted); cursor: pointer; user-select: none; transition: all 0.2s; position: relative; }
.mode-tab input[type="radio"] { display: none; }
.mode-tab:has(input:checked) { background: rgba(96,165,250,0.18); color: var(--accent); font-weight: 500; }
.mode-tab:hover:not(:has(input:checked)) { color: var(--text); }
.search-filters {
  margin-bottom: 12px;
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  align-items: center;
}
.filter-group {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
  align-items: center;
}
.filter-item {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text);
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
  line-height: 1.2;
  padding: 2px 0;
}
.filter-item:hover {
  color: var(--accent);
}
.filter-item input[type="checkbox"] {
  width: 14px;
  height: 14px;
  margin: 0;
  padding: 0;
  cursor: pointer;
  accent-color: var(--accent);
  flex-shrink: 0;
  vertical-align: middle;
}
.filter-item span {
  line-height: 1.2;
  font-size: 12px;
  vertical-align: middle;
}
.search-input-row { display: flex; gap: 8px; align-items: center; }
.search-input-row input { flex: 1; min-width: 0; }

.toolbar { 
  display: flex; 
  justify-content: space-between; 
  align-items: flex-start; 
  gap: 16px; 
  flex-wrap: wrap; 
  margin-bottom: 16px;
  padding-bottom: 16px;
  border-bottom: 1px solid var(--border);
}
.search { 
  display: flex; 
  flex-direction: column;
  gap: 12px; 
  flex: 1; 
  min-width: 0;
}
.search input { 
  width: 100%; 
}
.toolbar-right { 
  display: flex; 
  gap: 10px; 
  align-items: center; 
  flex-wrap: wrap; 
}
.statusbar { 
  display: flex; 
  justify-content: space-between; 
  align-items: center; 
  gap: 16px; 
  margin-top: 16px; 
  padding-top: 16px; 
  border-top: 1px solid var(--border); 
  flex-wrap: wrap;
}
.status { 
  color: var(--text); 
  font-size: 13px; 
  font-weight: 500;
}
.meta { 
  color: var(--muted); 
  font-size: 12px; 
  line-height: 1.5;
}
.status-right { 
  display: flex; 
  align-items: center; 
  gap: 16px; 
  flex-wrap: wrap; 
  justify-content: flex-end; 
}
.pager { 
  display: flex; 
  align-items: center; 
  gap: 10px; 
  flex-wrap: wrap; 
}
.select { 
  padding: 8px 12px; 
  border-radius: 10px; 
  border: 1px solid var(--border); 
  background: rgba(2,6,23,0.55); 
  color: var(--text);
  font-size: 12px;
  cursor: pointer;
  transition: border-color 0.15s;
}
.select:hover {
  border-color: rgba(96,165,250,0.35);
}
.hidden { display: none; }
.branch-filter { 
  width: min(220px, 100%); 
  font-size: 12px; 
}

.table-wrap { 
  max-width: 100%; 
  overflow-x: hidden; 
  overflow-y: visible;
  margin-top: 16px; 
  border: 1px solid var(--border); 
  border-radius: 16px; 
  background: rgba(15,23,42,0.5);
  scrollbar-width: thin;
  scrollbar-color: rgba(96,165,250,0.3) transparent;
}
.table-wrap::-webkit-scrollbar {
  height: 8px;
}
.table-wrap::-webkit-scrollbar-track {
  background: transparent;
}
.table-wrap::-webkit-scrollbar-thumb {
  background: rgba(96,165,250,0.3);
  border-radius: 4px;
}
.table-wrap::-webkit-scrollbar-thumb:hover {
  background: rgba(96,165,250,0.5);
}
.table { 
  width: 100%; 
  border-collapse: separate; 
  border-spacing: 0;
}
.table th, .table td { 
  padding: 14px 16px; 
  border-bottom: 1px solid var(--border); 
  vertical-align: top; 
  text-align: left;
}
.table th { 
  font-size: 12px; 
  font-weight: 600;
  color: var(--muted); 
  background: rgba(2,6,23,0.6); 
  position: sticky; 
  top: 0; 
  z-index: 5;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  white-space: nowrap;
}
.table th:first-child { border-top-left-radius: 16px; }
.table th:last-child { border-top-right-radius: 16px; }
.table tbody tr { 
  transition: background-color 0.15s;
}
.table tbody tr:hover { 
  background: var(--hover-bg);
}
.table tbody tr:last-child td { border-bottom: none; }
.table td { 
  font-size: 13px; 
  word-break: break-word;
  overflow-wrap: break-word;
}
.table td:first-child { padding-left: 20px; }
.table td:last-child { padding-right: 20px; }
.repo-name { 
  font-weight: 600; 
  font-size: 14px;
  color: var(--text);
  margin-bottom: 6px;
  line-height: 1.4;
}
.repo-name.repo-link { 
  cursor: pointer;
  transition: color 0.15s;
}
.repo-name.repo-link:hover { 
  color: var(--accent);
}
.truncate { 
  overflow: hidden; 
  text-overflow: ellipsis; 
  white-space: nowrap;
  display: block;
}
.wrap { white-space: normal; overflow-wrap: anywhere; word-break: break-word; }
.clamp2 {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.clamp3 {
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.table td .truncate {
  max-width: 100%;
}
.table td > div:not(.badges):not(.actions-cell) {
  margin-bottom: 4px;
}
.table td > div:last-child {
  margin-bottom: 0;
}
.tags-cell { 
  min-width: 280px;
}
.table td .mono {
  font-size: 12px;
  line-height: 1.5;
}
.branch-name {
  word-break: break-all;
  word-wrap: break-word;
  white-space: normal;
  overflow-wrap: anywhere;
  max-width: 200px;
  display: inline-block;
  line-height: 1.4;
}
.table td .meta {
  font-size: 12px;
  color: var(--muted);
  line-height: 1.4;
  margin-top: 4px;
}
.tag-editor { display: none; }
.tag-editor.on { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; margin-top: 8px; }
.tag-plus { padding: 2px 8px; border-radius: 999px; border: 1px dashed var(--border); background: rgba(255,255,255,0.02); color: var(--muted); cursor: pointer; }
.badges { 
  display: flex; 
  flex-wrap: wrap; 
  gap: 6px; 
  margin: 4px 0;
}
.badge { 
  padding: 4px 10px; 
  border-radius: 6px; 
  border: 1px solid var(--border); 
  background: rgba(96,165,250,0.1); 
  font-size: 11px; 
  color: var(--text);
  font-weight: 500;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.badge button { 
  padding: 0; 
  border: none; 
  background: none; 
  color: var(--muted); 
  cursor: pointer;
  font-size: 14px;
  line-height: 1;
  transition: color 0.15s;
}
.badge button:hover {
  color: var(--danger);
}
.match-badges {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 8px;
}
.match-badge {
  padding: 3px 8px;
  border-radius: 4px;
  background: rgba(96,165,250,0.15);
  border: 1px solid rgba(96,165,250,0.3);
  font-size: 11px;
  color: var(--accent);
  font-weight: 500;
  white-space: nowrap;
}
.commit-content {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.commit-snippet {
  line-height: 1.5;
  color: var(--text);
  font-size: 13px;
}
.actions-cell { 
  display: flex; 
  gap: 6px; 
  align-items: center; 
  flex-wrap: wrap; 
}
.small { 
  padding: 6px 12px; 
  border-radius: 8px; 
  font-size: 12px; 
}

.toast {
  position: fixed;
  bottom: 16px;
  left: 50%;
  transform: translateX(-50%);
  max-width: calc(100vw - 24px);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 10px 12px;
  border-radius: 999px;
  border: 1px solid var(--border);
  background: rgba(2,6,23,0.85);
  color: var(--text);
  font-size: 13px;
  opacity: 0;
  pointer-events: none;
  transition: opacity 180ms ease;
}
.toast.show { opacity: 1; }

.modal.hidden { display: none; }
.modal { position: fixed; inset: 0; display: grid; place-items: center; z-index: 50; }
.modal-backdrop { position: absolute; inset: 0; background: rgba(0,0,0,0.55); }
.modal-card {
  position: relative;
  width: min(920px, calc(100vw - 24px));
  max-height: min(80vh, 760px);
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: 16px;
  background: rgba(2,6,23,0.92);
  box-shadow: 0 20px 60px rgba(0,0,0,0.45);
  display: grid;
  grid-template-rows: auto auto 1fr auto;
}
.modal-head { display: flex; justify-content: space-between; align-items: center; padding: 12px 12px 8px 12px; border-bottom: 1px solid var(--border); }
.modal-title { font-weight: 700; }
.modal-sub { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 10px 12px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
.modal-body { 
  padding: 12px; 
  overflow-y: auto; 
  overflow-x: hidden;
  overscroll-behavior: contain;
}
.modal-body::-webkit-scrollbar {
  width: 8px;
}
.modal-body::-webkit-scrollbar-track {
  background: transparent;
}
.modal-body::-webkit-scrollbar-thumb {
  background: rgba(96,165,250,0.3);
  border-radius: 4px;
}
.modal-body::-webkit-scrollbar-thumb:hover {
  background: rgba(96,165,250,0.5);
}
.modal-foot { display: flex; justify-content: center; align-items: center; gap: 10px; padding: 10px 12px; border-top: 1px solid var(--border); }
.commit-list { display: grid; gap: 8px; }
.commit-item { border: 1px solid var(--border); border-radius: 12px; padding: 10px; background: rgba(255,255,255,0.03); }
.commit-top { display: flex; justify-content: space-between; gap: 10px; align-items: baseline; }
.commit-msg { font-weight: 650; }
.commit-meta { color: var(--muted); font-size: 12px; }
.commit-oid { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; color: var(--muted); font-size: 12px; }
pre { overflow-wrap: anywhere; word-break: break-word; }
"##;

const APP_JS: &str = r##"
const $ = (id) => document.getElementById(id);

const I18N = {
  zh: {
    langBtn: "中文",
    subtitle: "本地仓库管理与索引（离线）",
    qPlaceholder: "搜索：仓库名 / 路径 / README / 标签",
    qPlaceholderCommits: "搜索提交内容（需要先重建索引）",
    rootPlaceholder: "root 目录（例如：/Users/jim/dev）",
    branchFilterPlaceholder: "分支（可选）",
    scopeRepos: "仓库",
    scopeCommits: "提交",
    searchIn: "搜索范围：",
    inName: "名称",
    inPath: "路径",
    inReadme: "README",
    inTags: "标签",
    inSummary: "摘要",
    inMessage: "正文",
    hit_name: "名称",
    hit_path: "路径",
    hit_readme: "README",
    hit_tag: "标签",
    hit_summary: "摘要",
    hit_message: "正文",
    hit_repo: "仓库",
    hit_commit: "提交",
    bulkTagBtn: "批量标签",
    apply: "应用",
    clear: "清除",
    hintUsage: "提示：选择“提交”后可搜索提交内容（需要先重建索引）。",
    settingsTitle: "设置",
    searchBtn: "搜索",
    allBtn: "全部",
    recentFirst: "最近访问优先",
    pruneMoved: "清理已删除/移动",
    scanAllBtn: "扫描全部",
    pruneMissingBtn: "清理缺失",
    rootsTitle: "Roots",
    tagsTitle: "Tags",
    pickBtn: "选择…",
    addBtn: "添加",
    clearFilterBtn: "清除筛选",
    rootsHint: "只添加常用开发目录，避免扫描系统目录。",
    ready: "就绪",
    scanning: "扫描中…",
    pruning: "清理中…",
    scanDone: ({ indexed, pruned }) => `扫描完成：indexed=${indexed} pruned=${pruned}`,
    pruneDone: ({ deleted }) => `清理完成：deleted=${deleted}`,
    filterTag: ({ tag }) => `按标签过滤：${tag}`,
    allRepos: "全部仓库",
    searching: "搜索中…",
    searchResult: ({ q }) => `搜索结果：${q}`,
    accessRecorded: ({ path }) => `已记录访问：${path}`,
    copy: "复制",
    copied: "已复制路径",
    addTagPlaceholder: "添加标签…",
    addTagBtn: "添加",
    remove: "移除",
    scan: "扫描",
    open: "记录访问",
    noRootPicker: "当前平台暂不支持弹窗选择目录，请手动粘贴路径。",
    picked: ({ root }) => `已选择：${root}`,
    counts: ({ from, to, total }) => `显示 ${from}-${to} / ${total}`,
    never: "从未",
    colName: "名称",
    colPath: "路径",
    colBranch: "分支",
    colTags: "标签",
    colAccess: "最近访问",
    colActions: "操作",
    commitsTitle: "提交记录",
    commitsBtn: "提交",
    branch: "分支",
    repoTitle: "仓库详情",
    repoTagsHint: "标签：",
    commitDetailTitle: "提交详情",
    commitSearchBtn: "提交搜索",
    commitSearchTitle: "提交搜索",
    commitIndexHint: "提交搜索依赖本地索引；修改范围后请重建索引。",
    indexBranches: "分支数",
    indexCommits: "每分支提交数",
    rebuildIndex: "重建索引",
    resetBtn: "重置",
    ignoreHint: "扫描时忽略常见依赖/缓存目录（目录名匹配）。",
    perPage: "每页",
    prev: "上一页",
    next: "下一页",
    err: ({ msg }) => `错误：${msg}`,
  },
  en: {
    langBtn: "English",
    subtitle: "Local repo management & index (offline)",
    qPlaceholder: "Search: name / path / README / tag",
    qPlaceholderCommits: "Search commit content (rebuild index first)",
    rootPlaceholder: "Root directory (e.g. /Users/jim/dev)",
    branchFilterPlaceholder: "Branch (optional)",
    scopeRepos: "Repos",
    scopeCommits: "Commits",
    searchIn: "Search in:",
    inName: "Name",
    inPath: "Path",
    inReadme: "README",
    inTags: "Tags",
    inSummary: "Summary",
    inMessage: "Message",
    hit_name: "Name",
    hit_path: "Path",
    hit_readme: "README",
    hit_tag: "Tag",
    hit_summary: "Summary",
    hit_message: "Message",
    hit_repo: "Repo",
    hit_commit: "Commit",
    bulkTagBtn: "Bulk tag",
    apply: "Apply",
    clear: "Clear",
    hintUsage: "Tip: switch to “Commits” to search commit content (rebuild index first).",
    settingsTitle: "Settings",
    searchBtn: "Search",
    allBtn: "All",
    recentFirst: "Recent first",
    pruneMoved: "Prune moved/deleted",
    scanAllBtn: "Scan all",
    pruneMissingBtn: "Prune missing",
    rootsTitle: "Roots",
    tagsTitle: "Tags",
    pickBtn: "Choose…",
    addBtn: "Add",
    clearFilterBtn: "Clear filter",
    rootsHint: "Add only your dev folders; avoid scanning system directories.",
    ready: "Ready",
    scanning: "Scanning…",
    pruning: "Pruning…",
    scanDone: ({ indexed, pruned }) => `Scan done: indexed=${indexed} pruned=${pruned}`,
    pruneDone: ({ deleted }) => `Prune done: deleted=${deleted}`,
    filterTag: ({ tag }) => `Filtered by tag: ${tag}`,
    allRepos: "All repos",
    searching: "Searching…",
    searchResult: ({ q }) => `Search results: ${q}`,
    accessRecorded: ({ path }) => `Access recorded: ${path}`,
    copy: "Copy",
    copied: "Path copied",
    addTagPlaceholder: "Add tag…",
    addTagBtn: "Add",
    remove: "Remove",
    scan: "Scan",
    open: "Record access",
    noRootPicker: "Root picker isn't available on this platform. Paste the path manually.",
    picked: ({ root }) => `Selected: ${root}`,
    counts: ({ from, to, total }) => `Showing ${from}-${to} / ${total}`,
    never: "Never",
    colName: "Name",
    colPath: "Path",
    colBranch: "Branch",
    colTags: "Tags",
    colAccess: "Last access",
    colActions: "Actions",
    commitsTitle: "Commits",
    commitsBtn: "Commits",
    branch: "Branch",
    repoTitle: "Repository",
    repoTagsHint: "Tags:",
    commitDetailTitle: "Commit",
    commitSearchBtn: "Commit search",
    commitSearchTitle: "Commit search",
    commitIndexHint: "Commit search uses a local index; rebuild after changing limits.",
    indexBranches: "Branches",
    indexCommits: "Commits/branch",
    rebuildIndex: "Rebuild index",
    resetBtn: "Reset",
    ignoreHint: "Ignore dependency/cache folders during scan (by directory name).",
    perPage: "Per page",
    prev: "Prev",
    next: "Next",
    err: ({ msg }) => `Error: ${msg}`,
  },
};

function getLang() {
  const v = localStorage.getItem("coderoom.lang");
  return v === "en" ? "en" : "zh";
}

function setLang(lang) {
  localStorage.setItem("coderoom.lang", lang);
}

function t(key, vars = {}) {
  const lang = getLang();
  const v = I18N[lang][key];
  if (typeof v === "function") return v(vars);
  return v ?? key;
}

async function api(path, opts = {}) {
  const res = await fetch(path, {
    headers: { "Content-Type": "application/json" },
    ...opts,
  });
  if (!res.ok) {
    let msg = res.statusText;
    try {
      const j = await res.json();
      msg = j.error || msg;
    } catch {}
    throw new Error(msg);
  }
  if (res.status === 204) return null;
  return await res.json();
}

function setStatus(s) { $("status").textContent = s; }

function toast(msg) {
  const el = $("toast");
  el.textContent = msg;
  el.classList.add("show");
  setTimeout(() => el.classList.remove("show"), 1800);
}

function escapeRegExp(s) {
  return String(s).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function highlightHtml(text, q) {
  const raw = String(text || "");
  const query = String(q || "").trim();
  if (!query) return escapeHtml(raw);
  const esc = escapeHtml(raw);
  const re = new RegExp(escapeRegExp(query), "ig");
  return esc.replace(re, (m) => `<mark>${m}</mark>`);
}

function hitLabel(code) {
  const key = `hit_${code}`;
  const v = t(key);
  return v === key ? code : v;
}

async function copyToClipboard(text) {
  try {
    await navigator.clipboard.writeText(text);
    toast(t("copied"));
  } catch {
    toast(text);
  }
}

function setBusy(b) {
  ["btnAddRoot", "btnPickRoot", "btnScanAll", "btnPrune", "btnSearch", "btnAll"].forEach((id) => {
    const el = $(id);
    if (el) el.disabled = b;
  });
}

async function loadCommitIndexConfig() {
  const cfg = await api("/api/config");
  $("idxBranches").value = cfg.commit_index_branches;
  $("idxCommits").value = cfg.commit_index_commits_per_branch;
  renderIgnores(cfg.ignore_dir_names || []);
}

function renderIgnores(items) {
  const ul = $("ignores");
  if (!ul) return;
  ul.innerHTML = "";
  const list = Array.from(new Set((items || []).map((s) => String(s)))).filter((s) => s.trim().length > 0);
  list.sort();
  for (const n of list) {
    const li = document.createElement("li");
    li.innerHTML = `
      <div class="mono" title="${escapeHtml(n)}" style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escapeHtml(n)}</div>
      <div class="actions-cell">
        <button class="ghost small danger" data-name="${encodeURIComponent(n)}">${t("remove")}</button>
      </div>
    `;
    li.querySelector("button").onclick = async () => {
      await api("/api/ignores/remove", { method: "POST", body: JSON.stringify({ name: n }) });
      await loadCommitIndexConfig();
    };
    ul.appendChild(li);
  }
}

async function rebuildCommitIndexAll() {
  const branches = parseInt($("idxBranches").value, 10);
  const commits = parseInt($("idxCommits").value, 10);
  $("idxStatus").textContent = t("scanning");
  const out = await api("/api/commit_index/rebuild", {
    method: "POST",
    body: JSON.stringify({
      all: true,
      commit_index_branches: branches,
      commit_index_commits_per_branch: commits,
    }),
  });
  toast(`Commit index rebuilt: repos=${out.repos_indexed}`);
  $("idxStatus").textContent = t("ready");
}

let commitRepoPath = "";
let commitRefname = "HEAD";
let commitPage = 1;
let commitPerPage = 50;
let commitHasMore = false;

let modalOpenCount = 0;

function lockBodyScroll(lock) {
  if (lock) {
    modalOpenCount++;
    document.body.style.overflow = "hidden";
  } else {
    modalOpenCount = Math.max(0, modalOpenCount - 1);
    if (modalOpenCount === 0) {
      document.body.style.overflow = "";
    }
  }
}

function showCommitModal(show) {
  const m = $("commitModal");
  if (show) {
    m.classList.remove("hidden");
    lockBodyScroll(true);
  } else {
    m.classList.add("hidden");
    lockBodyScroll(false);
  }
}

function showRepoModal(show) {
  const m = $("repoModal");
  if (show) {
    m.classList.remove("hidden");
    lockBodyScroll(true);
  } else {
    m.classList.add("hidden");
    lockBodyScroll(false);
  }
}

function showCommitDetailModal(show) {
  const m = $("commitDetailModal");
  if (show) {
    m.classList.remove("hidden");
    lockBodyScroll(true);
  } else {
    m.classList.add("hidden");
    lockBodyScroll(false);
  }
}

let repoModalData = null;

function openRepoDetail(repo) {
  repoModalData = repo;
  $("repoName").textContent = repo.name || "";
  $("repoPath").textContent = repo.path || "";
  $("repoOrigin").textContent = repo.origin_url || "";
  const about = (repo.readme_excerpt || "").trim();
  const q = (viewMode === "search" ? currentQuery : "").trim();
  if (q && about) {
    $("repoAbout").innerHTML = highlightHtml(about, q);
  } else {
    $("repoAbout").textContent = about;
  }
  $("repoTags").innerHTML = (repo.tags || []).map((t0) => `<span class="badge">${escapeHtml(t0)}</span>`).join("");
  showRepoModal(true);
}

async function openCommitDetail(repoPath, oid) {
  const out = await api(`/api/commit_detail?repo_path=${encodeURIComponent(repoPath)}&oid=${encodeURIComponent(oid)}`);
  $("cdSummary").textContent = out.summary || "";
  const who = [out.author, out.email].filter(Boolean).join(" ");
  const shortOid = (out.oid || "").slice(0, 8);
  $("cdMeta").textContent = `${shortOid} · ${who} · ${fmtTs(out.time)}`;
  const q = (viewMode === "commit_search" ? currentQuery : "").trim();
  if (q) {
    $("cdMessage").innerHTML = highlightHtml(out.message || "", q);
  } else {
    $("cdMessage").textContent = out.message || "";
  }
  showCommitDetailModal(true);
}

function renderCommitList(items) {
  const box = $("commitList");
  box.innerHTML = "";
  for (const c of items) {
    const shortOid = (c.oid || "").slice(0, 8);
    const who = [c.author, c.email].filter(Boolean).join(" ");
    const el = document.createElement("div");
    el.className = "commit-item";
    el.innerHTML = `
      <div class="commit-top">
        <div class="commit-msg truncate" title="${escapeHtml(c.summary || "")}">${escapeHtml(c.summary || "")}</div>
        <div class="commit-oid" title="${escapeHtml(c.oid || "")}">${escapeHtml(shortOid)}</div>
      </div>
      <div class="commit-meta">${escapeHtml(who)} · ${escapeHtml(fmtTs(c.time))}</div>
    `;
    el.onclick = async () => {
      await openCommitDetail(commitRepoPath, c.oid);
    };
    box.appendChild(el);
  }
}

function updateCommitPager() {
  $("commitPageInfo").textContent = `${commitPage}`;
  $("commitPrev").disabled = commitPage <= 1;
  $("commitNext").disabled = !commitHasMore;
}

async function loadCommits() {
  const out = await api(
    `/api/commits?repo_path=${encodeURIComponent(commitRepoPath)}&refname=${encodeURIComponent(commitRefname)}&page=${commitPage}&per_page=${commitPerPage}`
  );
  commitHasMore = !!out.has_more;
  renderCommitList(out.items || []);
  updateCommitPager();
}

async function openCommits(repoPath, defaultBranch, preferredRefname) {
  commitRepoPath = repoPath;
  commitPage = 1;
  commitRefname = "HEAD";
  $("commitRepo").textContent = repoPath;
  $("commitList").innerHTML = "";
  showCommitModal(true);
  setStatus(t("ready"));

  const branches = await api(`/api/branches?repo_path=${encodeURIComponent(repoPath)}`);
  const sel = $("branchSelect");
  sel.innerHTML = "";

  const optHead = document.createElement("option");
  optHead.value = "HEAD";
  optHead.textContent = "HEAD";
  sel.appendChild(optHead);

  const groups = { local: document.createElement("optgroup"), remote: document.createElement("optgroup") };
  groups.local.label = "local";
  groups.remote.label = "remote";

  for (const b of branches) {
    const o = document.createElement("option");
    o.value = b.refname;
    o.textContent = b.name;
    if (b.kind === "remote") groups.remote.appendChild(o);
    else groups.local.appendChild(o);
  }
  if (groups.local.children.length) sel.appendChild(groups.local);
  if (groups.remote.children.length) sel.appendChild(groups.remote);

  if (preferredRefname) {
    const found = Array.from(sel.options).find((o) => o.value === preferredRefname);
    if (found) sel.value = preferredRefname;
  } else if (defaultBranch) {
    const want = `refs/heads/${defaultBranch}`;
    const found = Array.from(sel.options).find((o) => o.value === want);
    if (found) sel.value = want;
  }
  commitRefname = sel.value;

  sel.onchange = async () => {
    commitRefname = sel.value;
    commitPage = 1;
    await loadCommits();
  };

  await loadCommits();
}

function applyI18n() {
  const lang = getLang();
  $("btnLang").textContent = I18N[lang].langBtn;
  const scopeCommits = $("scopeCommits")?.checked;
  $("q").placeholder = scopeCommits ? t("qPlaceholderCommits") : t("qPlaceholder");
  $("branchFilter").placeholder = t("branchFilterPlaceholder");
  $("root").placeholder = t("rootPlaceholder");
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const k = el.getAttribute("data-i18n");
    if (k) el.textContent = t(k);
  });
}

let activeTag = null;
let viewMode = "list"; // list | search
let currentQuery = "";
let commitBranchFilter = "";
let currentPage = 1;
let perPage = 25;
let lastTotal = 0;
let bulkMode = false;
let bulkSelected = new Set();

function updateBulkUi() {
  $("bulkCount").classList.toggle("hidden", !bulkMode);
  $("bulkTag").classList.toggle("hidden", !bulkMode);
  $("applyBulkTag").classList.toggle("hidden", !bulkMode);
  $("clearBulk").classList.toggle("hidden", !bulkMode);
  $("bulkCount").textContent = `${bulkSelected.size}`;
  $("btnBulk").textContent = bulkMode ? t("bulkTagBtn") + " ✓" : t("bulkTagBtn");
}

function setTableMode(mode) {
  const head = $("tableHead");
  const table = head.closest("table");
  if (mode === "commits") {
    head.innerHTML = `
      <tr>
        <th data-i18n="colName">${t("colName")}</th>
        <th data-i18n="scopeCommits">${t("scopeCommits")}</th>
        <th data-i18n="colBranch">${t("colBranch")}</th>
        <th data-i18n="colAccess">${t("colAccess")}</th>
        <th>OID</th>
        <th data-i18n="colActions">${t("colActions")}</th>
      </tr>
    `;
    table.style.minWidth = "";
  } else {
    const sel = bulkMode ? `<th style="width: 50px;">✓</th>` : "";
    head.innerHTML = `
      <tr>
        ${sel}
        <th data-i18n="colName">${t("colName")}</th>
        <th data-i18n="colTags">${t("colTags")}</th>
        <th data-i18n="colBranch">${t("colBranch")}</th>
        <th data-i18n="colAccess">${t("colAccess")}</th>
        <th data-i18n="colActions">${t("colActions")}</th>
      </tr>
    `;
    table.style.minWidth = "";
  }
}

function renderRoots(roots) {
  const ul = $("roots");
  ul.innerHTML = "";
  for (const r of roots) {
    const li = document.createElement("li");
    li.innerHTML = `
      <div class="mono" title="${escapeHtml(r)}" style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escapeHtml(r)}</div>
      <div class="actions-cell">
        <button class="ghost small" data-scan="${encodeURIComponent(r)}">${t("scan")}</button>
        <button class="ghost small danger" data-root="${encodeURIComponent(r)}">${t("remove")}</button>
      </div>
    `;
    li.querySelector("button.danger").onclick = async () => {
      await api("/api/roots/remove", { method: "POST", body: JSON.stringify({ root: r }) });
      await refresh();
    };
    li.querySelector("button[data-scan]").onclick = async () => {
      setBusy(true);
      try {
        setStatus(t("scanning"));
        const prune = $("prune").checked;
        const out = await api("/api/scan", { method: "POST", body: JSON.stringify({ root: r, prune }) });
        setStatus(t("scanDone", { indexed: out.indexed, pruned: out.pruned }));
        toast(t("scanDone", { indexed: out.indexed, pruned: out.pruned }));
        await refresh();
      } finally {
        setBusy(false);
      }
    };
    ul.appendChild(li);
  }
}

function renderTags(tags) {
  const box = $("tags");
  box.innerHTML = "";
  for (const row of tags) {
    const tag = row.name;
    const count = row.count || 0;
    const c = document.createElement("div");
    c.className = `chip${activeTag === tag ? " active" : ""}`;
    c.textContent = `${tag} (${count})`;
    c.onclick = async () => {
      activeTag = tag;
      viewMode = "list";
      currentQuery = "";
      currentPage = 1;
      $("q").value = "";
      await loadPage();
      setStatus(t("filterTag", { tag }));
    };
    box.appendChild(c);
  }
}

function fmtTs(ts) {
  if (!ts) return t("never");
  const d = new Date(ts * 1000);
  return d.toLocaleString(getLang() === "zh" ? "zh-CN" : "en-US");
}

function renderCommitHits(items) {
  const tbody = $("repos");
  tbody.innerHTML = "";
  setTableMode("commits");
  for (const c of items) {
    const tr = document.createElement("tr");
    const shortOid = (c.oid || "").slice(0, 8);
    const matched = (c.matched_in || []).map((m) => {
      const label = hitLabel(m);
      return `<span class="match-badge">${escapeHtml(label)}</span>`;
    }).join("");
    const snippet = (c.snippet || c.summary || "").trim();
    const hasSummary = c.matched_in && c.matched_in.includes("summary");
    const hasMessage = c.matched_in && c.matched_in.includes("message");
    
    tr.innerHTML = `
      <td>
        <div class="repo-name wrap clamp2" title="${escapeHtml(c.repo_name + "\n" + c.repo_path)}">${highlightHtml(c.repo_name, currentQuery)}</div>
        <div class="mono wrap clamp2" style="margin-top:4px;" title="${escapeHtml(c.repo_path)}">${highlightHtml(c.repo_path, currentQuery)}</div>
      </td>
      <td>
        <div class="commit-content">
          ${matched ? `<div class="match-badges">${matched}</div>` : ""}
          <div class="commit-snippet wrap clamp3" title="${escapeHtml(c.summary || "")}">${highlightHtml(snippet, currentQuery)}</div>
        </div>
      </td>
      <td><span class="mono branch-name" title="${escapeHtml(c.branch_name || "")}">${escapeHtml(c.branch_name || "")}</span></td>
      <td><span class="mono" style="white-space:nowrap;">${escapeHtml(fmtTs(c.time))}</span></td>
      <td><span class="mono" style="white-space:nowrap;">${escapeHtml(shortOid)}</span></td>
      <td>
        <div class="actions-cell">
          <button class="ghost small" data-open-commits="${encodeURIComponent(c.repo_path)}" data-ref="${encodeURIComponent(c.refname)}">${t("commitsBtn")}</button>
          <button class="ghost small" data-copy="${encodeURIComponent(c.repo_path)}">${t("copy")}</button>
        </div>
      </td>
    `;
    tr.querySelector("button[data-open-commits]").onclick = async () => {
      const repoPath = c.repo_path;
      const refname = c.refname;
      await openCommits(repoPath, null, refname);
    };
    tr.querySelector("button[data-copy]").onclick = async () => copyToClipboard(c.repo_path);
    tbody.appendChild(tr);
  }
}

function renderRepos(repos) {
  const tbody = $("repos");
  tbody.innerHTML = "";
  setTableMode("repos");
  for (const r of repos) {
    const tr = document.createElement("tr");
    const tags = (r.tags || [])
      .map(
        (tg) =>
          `<span class="badge">${escapeHtml(tg)}<button title="remove" data-rt="${encodeURIComponent(r.path)}" data-tg="${encodeURIComponent(tg)}">×</button></span>`
      )
      .join("");
    const about = (r.readme_excerpt || "").trim();
    const origin = (r.origin_url || "").trim();
    const matched = (r.matched_in || []).map((m) => `<span class="badge">${escapeHtml(hitLabel(m))}</span>`).join("");
    const doHighlight = viewMode === "search" && currentQuery.trim().length > 0;
    const nameHtml = doHighlight ? highlightHtml(r.name, currentQuery) : escapeHtml(r.name);
    const aboutHtml = doHighlight ? highlightHtml(about, currentQuery) : escapeHtml(about);
    const originHtml = doHighlight ? highlightHtml(origin, currentQuery) : escapeHtml(origin);
    const pathHtml = doHighlight ? highlightHtml(r.path, currentQuery) : escapeHtml(r.path);
    const selCell = bulkMode
      ? `<td><input type="checkbox" class="sel" data-path="${encodeURIComponent(r.path)}" ${bulkSelected.has(r.path) ? "checked" : ""} /></td>`
      : "";
    tr.innerHTML = `
      ${selCell}
      <td>
        <div class="repo-name wrap clamp2 repo-link" title="${escapeHtml(r.name + (r.path ? "\n" + r.path : ""))}">${nameHtml}</div>
        ${matched ? `<div class="badges" style="margin-top:6px;">${matched}</div>` : ""}
        ${about ? `<div class="meta wrap clamp2" title="${escapeHtml(about)}">${aboutHtml}</div>` : ""}
        ${origin ? `<div class="mono wrap clamp2 origin" title="${escapeHtml(origin)}">${originHtml}</div>` : ""}
        <div class="mono wrap clamp2" title="${escapeHtml(r.path)}">${pathHtml}</div>
      </td>
      <td class="tags-cell">
        <div class="badges">${tags}</div>
        <div class="row" style="margin-top:6px;">
          <button class="tag-plus" data-toggle-tag="${encodeURIComponent(r.path)}">＋</button>
        </div>
        <div class="tag-editor" data-editor="${encodeURIComponent(r.path)}">
          <input class="add-tag" placeholder="${escapeHtml(t("addTagPlaceholder"))}" />
          <button class="small" data-add-tag="${encodeURIComponent(r.path)}">${t("addTagBtn")}</button>
          <button class="ghost small" data-cancel-tag="${encodeURIComponent(r.path)}">×</button>
        </div>
      </td>
      <td><span class="mono branch-name" title="${escapeHtml(r.default_branch || "-")}">${escapeHtml(r.default_branch || "-")}</span></td>
      <td><span class="mono" style="white-space:nowrap;">${escapeHtml(fmtTs(r.last_access_ts))}</span></td>
      <td>
        <div class="actions-cell">
          <button class="ghost small" data-commits="${encodeURIComponent(r.path)}">${t("commitsBtn")}</button>
          <button class="ghost small" data-open="${encodeURIComponent(r.path)}">${t("open")}</button>
          <button class="ghost small" data-copy="${encodeURIComponent(r.path)}">${t("copy")}</button>
        </div>
      </td>
    `;

    if (bulkMode) {
      tr.querySelector("input.sel").onchange = (e) => {
        const path = decodeURIComponent(e.target.dataset.path);
        if (e.target.checked) bulkSelected.add(path);
        else bulkSelected.delete(path);
        updateBulkUi();
      };
    }

    tr.querySelector("button[data-copy]").onclick = async () => {
      await copyToClipboard(r.path);
    };

    if (origin) {
      tr.querySelector(".origin").onclick = async () => {
        await copyToClipboard(origin);
      };
    }

    tr.querySelector(".repo-link").onclick = async () => {
      openRepoDetail(r);
    };

    tr.querySelector("button[data-commits]").onclick = async () => {
      await openCommits(r.path, r.default_branch, null);
    };

    tr.querySelector("button[data-open]").onclick = async () => {
      await api("/api/open", { method: "POST", body: JSON.stringify({ repo: r.path }) });
      toast(t("accessRecorded", { path: r.path }));
      await loadPage();
    };

    const editor = tr.querySelector(`.tag-editor[data-editor="${encodeURIComponent(r.path)}"]`);
    tr.querySelector("button[data-toggle-tag]").onclick = async () => {
      editor.classList.toggle("on");
      const input = editor.querySelector("input.add-tag");
      input.focus();
    };
    tr.querySelector("button[data-cancel-tag]").onclick = async () => {
      editor.classList.remove("on");
    };
    tr.querySelector("button[data-add-tag]").onclick = async () => {
      const input = editor.querySelector("input.add-tag");
      const tg = input.value.trim();
      if (!tg) return;
      await api("/api/repos/tag", { method: "POST", body: JSON.stringify({ repo_path: r.path, tag: tg }) });
      input.value = "";
      editor.classList.remove("on");
      await loadPage();
      await refreshSidebars();
    };

    tr.querySelectorAll("span.badge button").forEach((b) => {
      b.onclick = async () => {
        const repo_path = decodeURIComponent(b.dataset.rt || "");
        const tag = decodeURIComponent(b.dataset.tg || "");
        if (!repo_path || !tag) return;
        await api("/api/repos/untag", { method: "POST", body: JSON.stringify({ repo_path, tag }) });
        await loadPage();
        await refreshSidebars();
      };
    });

    tbody.appendChild(tr);
  }
}

function escapeHtml(s) {
  return String(s).replace(/[&<>\"']/g, (c) => ({ "&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;","'":"&#39;" }[c]));
}

function updatePager() {
  const totalPages = Math.max(1, Math.ceil(lastTotal / perPage));
  $("pageInfo").textContent = `${currentPage}/${totalPages}`;
  $("prevPage").disabled = currentPage <= 1;
  $("nextPage").disabled = currentPage >= totalPages;
  $("counts").textContent = t("counts", { total: lastTotal, filtered: lastTotal });
}

async function refreshSidebars() {
  const roots = await api("/api/roots");
  renderRoots(roots.roots || []);
  const tags = await api("/api/tags");
  renderTags(tags);
}

async function loadPage() {
  const recent = $("recent").checked ? "true" : "false";
  if (viewMode === "search") {
    const in_name = $("inName").checked ? "true" : "false";
    const in_path = $("inPath").checked ? "true" : "false";
    const in_readme = $("inReadme").checked ? "true" : "false";
    const in_tags = $("inTags").checked ? "true" : "false";
    const out = await api(
      `/api/search?q=${encodeURIComponent(currentQuery)}&page=${currentPage}&per_page=${perPage}&in_name=${in_name}&in_path=${in_path}&in_readme=${in_readme}&in_tags=${in_tags}`
    );
    lastTotal = out.total;
    renderRepos(out.items || []);
  } else if (viewMode === "commit_search") {
    const b = commitBranchFilter ? `&branch=${encodeURIComponent(commitBranchFilter)}` : "";
    const in_summary = $("inSummary").checked ? "true" : "false";
    const in_message = $("inMessage").checked ? "true" : "false";
    const out = await api(
      `/api/commit_search?q=${encodeURIComponent(currentQuery)}${b}&in_summary=${in_summary}&in_message=${in_message}&page=${currentPage}&per_page=${perPage}`
    );
    lastTotal = out.total;
    renderCommitHits(out.items || []);
  } else {
    const tagPart = activeTag ? `&tag=${encodeURIComponent(activeTag)}` : "";
    const out = await api(`/api/repos?recent=${recent}${tagPart}&page=${currentPage}&per_page=${perPage}`);
    lastTotal = out.total;
    renderRepos(out.items || []);
  }
  const totalPages = Math.max(1, Math.ceil(lastTotal / perPage));
  $("pageInfo").textContent = `${currentPage}/${totalPages}`;
  $("prevPage").disabled = currentPage <= 1;
  $("nextPage").disabled = currentPage >= totalPages;
  if (lastTotal === 0) {
    $("counts").textContent = t("counts", { from: 0, to: 0, total: 0 });
  } else {
    const from = (currentPage - 1) * perPage + 1;
    const to = Math.min(currentPage * perPage, lastTotal);
    $("counts").textContent = t("counts", { from, to, total: lastTotal });
  }
}

async function refresh() {
  await refreshSidebars();
  await loadPage();
  setStatus(t("ready"));
}

$("btnAddRoot").onclick = async () => {
  let root = $("root").value.trim();
  if (!root) {
    setBusy(true);
    try {
      const out = await api("/api/roots/pick", { method: "POST", body: "{}" });
      if (!out || !out.root) {
        toast(t("noRootPicker"));
        return;
      }
      root = out.root;
      $("root").value = root;
    } finally {
      setBusy(false);
    }
  }
  await api("/api/roots/add", { method: "POST", body: JSON.stringify({ root }) });
  $("root").value = "";
  await refresh();
};

$("btnPickRoot").onclick = async () => {
  setBusy(true);
  try {
    const out = await api("/api/roots/pick", { method: "POST", body: "{}" });
    if (!out || !out.root) {
      toast(t("noRootPicker"));
      return;
    }
    $("root").value = out.root;
    toast(t("picked", { root: out.root }));
  } finally {
    setBusy(false);
  }
};

$("ignoreAdd").onclick = async () => {
  const name = $("ignoreName").value.trim();
  if (!name) return;
  await api("/api/ignores/add", { method: "POST", body: JSON.stringify({ name }) });
  $("ignoreName").value = "";
  await loadCommitIndexConfig();
};

$("ignoreReset").onclick = async () => {
  await api("/api/ignores/reset", { method: "POST", body: "{}" });
  await loadCommitIndexConfig();
};

$("btnClearTag").onclick = async () => {
  activeTag = null;
  viewMode = "list";
  currentQuery = "";
  commitBranchFilter = "";
  currentPage = 1;
  $("q").value = "";
  await loadPage();
};

$("btnScanAll").onclick = async () => {
  setBusy(true);
  try {
    setStatus(t("scanning"));
    const prune = $("prune").checked;
    const out = await api("/api/scan", { method: "POST", body: JSON.stringify({ all: true, prune }) });
    setStatus(t("scanDone", { indexed: out.indexed, pruned: out.pruned }));
    toast(t("scanDone", { indexed: out.indexed, pruned: out.pruned }));
    await refresh();
  } finally {
    setBusy(false);
  }
};

$("btnPrune").onclick = async () => {
  setBusy(true);
  try {
    setStatus(t("pruning"));
    const out = await api("/api/prune", { method: "POST", body: "{}" });
    setStatus(t("pruneDone", { deleted: out.deleted }));
    toast(t("pruneDone", { deleted: out.deleted }));
    await refresh();
  } finally {
    setBusy(false);
  }
};

$("btnAll").onclick = async () => {
  activeTag = null;
  $("q").value = "";
  $("branchFilter").value = "";
  viewMode = "list";
  currentQuery = "";
  commitBranchFilter = "";
  currentPage = 1;
  bulkMode = false;
  bulkSelected.clear();
  updateBulkUi();
  await loadPage();
  setStatus(t("allRepos"));
};

$("btnSearch").onclick = async () => {
  const q = $("q").value.trim();
  const commits = $("scopeCommits").checked;
  if (commits) {
    if (!q) return;
    viewMode = "commit_search";
    currentQuery = q;
    commitBranchFilter = $("branchFilter").value.trim();
    currentPage = 1;
    await loadPage();
    return;
  }
  if (!q) {
    viewMode = "list";
    currentQuery = "";
    currentPage = 1;
    await loadPage();
    setStatus(t("allRepos"));
    return;
  }
  setBusy(true);
  try {
    setStatus(t("searching"));
    viewMode = "search";
    currentQuery = q;
    currentPage = 1;
    await loadPage();
    setStatus(t("searchResult", { q }));
  } finally {
    setBusy(false);
  }
};

$("q").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("btnSearch").click();
});

function updateSearchUi() {
  const commits = $("scopeCommits").checked;
  // 显示/隐藏对应的选项组
  document.querySelectorAll(".filter-group").forEach((group) => {
    const mode = group.dataset.mode;
    group.classList.toggle("hidden", (mode === "repos" && commits) || (mode === "commits" && !commits));
  });
  // 更新搜索框placeholder
  $("q").placeholder = commits ? t("qPlaceholderCommits") : t("qPlaceholder");
  applyI18n();
}

document.querySelectorAll('input[name="searchMode"]').forEach((radio) => {
  radio.onchange = () => updateSearchUi();
});

$("q").addEventListener("input", async () => {
  const q = $("q").value.trim();
  const commits = $("scopeCommits").checked;
  if (commits) {
    if (q.length === 0 && viewMode === "commit_search") {
      viewMode = "list";
      currentQuery = "";
      commitBranchFilter = "";
      currentPage = 1;
      await loadPage();
      setStatus(t("allRepos"));
    }
    return;
  }
  if (q.length === 0 && viewMode === "search") {
    viewMode = "list";
    currentQuery = "";
    currentPage = 1;
    await loadPage();
    setStatus(t("allRepos"));
  }
});

$("recent").onchange = async () => {
  currentPage = 1;
  await loadPage();
};

$("perPage").onchange = async () => {
  perPage = parseInt($("perPage").value, 10) || 25;
  currentPage = 1;
  await loadPage();
};

$("prevPage").onclick = async () => {
  if (currentPage <= 1) return;
  currentPage -= 1;
  await loadPage();
};

$("nextPage").onclick = async () => {
  currentPage += 1;
  await loadPage();
};

$("commitClose").onclick = () => showCommitModal(false);
$("commitX").onclick = () => showCommitModal(false);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") showCommitModal(false);
});

$("repoClose").onclick = () => showRepoModal(false);
$("repoX").onclick = () => showRepoModal(false);
$("commitDetailClose").onclick = () => showCommitDetailModal(false);
$("commitDetailX").onclick = () => showCommitDetailModal(false);

$("repoCopy").onclick = async () => {
  if (!repoModalData) return;
  await copyToClipboard(repoModalData.path);
};

$("repoOrigin").onclick = async () => {
  if (!repoModalData || !repoModalData.origin_url) return;
  await copyToClipboard(repoModalData.origin_url);
};

$("repoCommits").onclick = async () => {
  if (!repoModalData) return;
  showRepoModal(false);
  await openCommits(repoModalData.path, repoModalData.default_branch, null);
};

$("commitPrev").onclick = async () => {
  if (commitPage <= 1) return;
  commitPage -= 1;
  await loadCommits();
};

$("commitNext").onclick = async () => {
  if (!commitHasMore) return;
  commitPage += 1;
  await loadCommits();
};

$("btnLang").onclick = async () => {
  const next = getLang() === "zh" ? "en" : "zh";
  setLang(next);
  applyI18n();
  await refresh();
};

$("rebuildIndex").onclick = async () => {
  await rebuildCommitIndexAll();
  await loadCommitIndexConfig();
};

$("btnBulk").onclick = async () => {
  bulkMode = !bulkMode;
  if (!bulkMode) {
    bulkSelected.clear();
  }
  updateBulkUi();
  await loadPage();
};

$("clearBulk").onclick = async () => {
  bulkSelected.clear();
  updateBulkUi();
  await loadPage();
};

$("applyBulkTag").onclick = async () => {
  const tag = $("bulkTag").value.trim();
  if (!tag || bulkSelected.size === 0) return;
  setBusy(true);
  try {
    for (const repo_path of bulkSelected) {
      await api("/api/repos/tag", { method: "POST", body: JSON.stringify({ repo_path, tag }) });
    }
    toast(`${t("apply")}: ${tag} (${bulkSelected.size})`);
    await refresh();
  } finally {
    setBusy(false);
  }
};

applyI18n();
loadCommitIndexConfig().catch(() => {});
updateSearchUi();
updateBulkUi();
refresh().catch((e) => setStatus(t("err", { msg: e.message })));
"##;
