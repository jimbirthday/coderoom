use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod config;
mod commits;
mod db;
mod scan;
mod web;

#[derive(Parser, Debug)]
#[command(name = "coderoom", version, about = "Local git repo indexer (offline)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 初始化本地数据目录、配置与数据库
    Init,
    /// 扫描一个根目录（可重复执行，增量更新）
    Scan {
        /// 需要扫描的根目录
        #[arg(long)]
        root: String,
        /// 扫描深度限制（默认不限制）
        #[arg(long)]
        max_depth: Option<usize>,
        /// 扫描完成后清理 root 下已删除/移动的仓库记录
        #[arg(long)]
        prune: bool,
    },
    /// 扫描 config.toml 里记录的全部 roots
    ScanAll {
        /// 扫描深度限制（默认不限制）
        #[arg(long)]
        max_depth: Option<usize>,
        /// 扫描完成后清理每个 root 下已删除/移动的仓库记录
        #[arg(long)]
        prune: bool,
    },
    /// 列出已索引仓库
    List {
        /// 按标签过滤
        #[arg(long)]
        tag: Option<String>,
        /// 按最近访问排序
        #[arg(long)]
        recent: bool,
    },
    /// 关键字搜索（仓库名/路径/README 摘要/标签）
    Search {
        query: String,
    },
    /// 标签管理
    Tag {
        #[command(subcommand)]
        command: TagCommand,
    },
    /// 记录一次访问，并输出仓库路径（用于 shell/编辑器集成）
    Open {
        /// 仓库路径（也可以传 name 的子串，但建议用路径更可靠）
        repo: String,
    },
    /// 清理数据库中已不存在的仓库路径
    Prune,
    /// 管理扫描 Roots（写入 ~/.coderoom/config.toml）
    Roots {
        #[command(subcommand)]
        command: RootsCommand,
    },
    /// 启动本地 Web 管理页（默认仅绑定 127.0.0.1）
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8787)]
        port: u16,
    },
    /// 构建/重建提交索引（用于提交内容搜索）
    CommitIndex {
        /// 对所有已索引仓库执行
        #[arg(long)]
        all: bool,
        /// 只对某个仓库执行（路径）
        #[arg(long)]
        repo: Option<String>,
        /// 最近提交的分支数（默认读取 config.toml）
        #[arg(long)]
        branches: Option<usize>,
        /// 每个分支索引的提交数（默认读取 config.toml）
        #[arg(long)]
        commits_per_branch: Option<usize>,
    },
    /// 管理扫描时需要忽略的目录名（写入 ~/.coderoom/config.toml）
    Ignores {
        #[command(subcommand)]
        command: IgnoresCommand,
    },
}

#[derive(Subcommand, Debug)]
enum IgnoresCommand {
    /// 列出忽略目录名
    List,
    /// 添加一个忽略目录名（例如：.cargo_home）
    Add { name: String },
    /// 移除一个忽略目录名
    Remove { name: String },
    /// 重置为默认忽略目录名列表
    Reset,
}

#[derive(Subcommand, Debug)]
enum TagCommand {
    Add {
        /// 仓库路径
        #[arg(long)]
        repo: String,
        /// 标签名
        tag: String,
    },
    Remove {
        /// 仓库路径
        #[arg(long)]
        repo: String,
        /// 标签名
        tag: String,
    },
    List {
        /// 仓库路径（不传则列出全局标签）
        #[arg(long)]
        repo: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum RootsCommand {
    List,
    Add { root: String },
    Remove { root: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let data_dir = config::data_dir()?;
    let cfg_path = config::config_path(&data_dir);
    let db_path = config::db_path(&data_dir);
    config::ensure_data_dir(&data_dir)?;

    match cli.command {
        Command::Init => {
            let cfg = config::Config::load_or_create(&cfg_path)?;
            db::Db::open(&db_path)?.init_schema()?;
            println!("Initialized.");
            println!("Config: {}", cfg_path.display());
            println!("DB: {}", db_path.display());
            if cfg.roots.is_empty() {
                println!("Tip: run `coderoom scan --root <dir>` to index repos.");
            }
        }
        Command::Scan {
            root,
            max_depth,
            prune,
        } => {
            let mut cfg = config::Config::load_or_create(&cfg_path)?;
            let db = db::Db::open(&db_path)?;
            db.init_schema()?;
            let ignore_dir_names: std::collections::HashSet<String> =
                cfg.ignore_dir_names.iter().cloned().collect();

            let root_input = root;
            let root_buf = std::path::PathBuf::from(&root_input);
            let root_path = std::fs::canonicalize(&root_buf).unwrap_or(root_buf);
            let repos = scan::discover_git_repos(&root_path, max_depth, &ignore_dir_names)
                .with_context(|| format!("scan root {}", root_path.display()))?;
            let mut keep = std::collections::HashSet::<String>::new();
            for repo_root in repos {
                let meta = scan::read_repo_metadata(&repo_root)?;
                keep.insert(meta.path.clone());
                db.upsert_repo(&meta)?;
            }
            let pruned = if prune {
                db.prune_under_root(&root_path.to_string_lossy(), &keep)?
            } else {
                0
            };
            cfg.add_root(&root_path);
            cfg.save(&cfg_path)?;
            println!("Indexed {} repos. Pruned {}.", keep.len(), pruned);
        }
        Command::ScanAll { max_depth, prune } => {
            let cfg = config::Config::load_or_create(&cfg_path)?;
            if cfg.roots.is_empty() {
                println!("No roots configured. Use `coderoom roots add <dir>` or `coderoom scan --root <dir>`.");
                return Ok(());
            }
            let db = db::Db::open(&db_path)?;
            db.init_schema()?;
            let ignore_dir_names: std::collections::HashSet<String> =
                cfg.ignore_dir_names.iter().cloned().collect();
            let mut indexed = 0usize;
            let mut pruned = 0usize;
            for root in cfg.roots {
                let root_path = std::fs::canonicalize(std::path::PathBuf::from(&root))
                    .unwrap_or_else(|_| std::path::PathBuf::from(&root));
                let repos = scan::discover_git_repos(&root_path, max_depth, &ignore_dir_names)
                    .with_context(|| format!("scan root {}", root_path.display()))?;
                let mut keep = std::collections::HashSet::<String>::new();
                for repo_root in repos {
                    let meta = scan::read_repo_metadata(&repo_root)?;
                    keep.insert(meta.path.clone());
                    db.upsert_repo(&meta)?;
                }
                indexed += keep.len();
                if prune {
                    pruned += db.prune_under_root(&root_path.to_string_lossy(), &keep)?;
                }
            }
            println!("Indexed {indexed} repos. Pruned {pruned}.");
        }
        Command::List { tag, recent } => {
            let db = db::Db::open(&db_path)?;
            db.init_schema()?;
            let repos = db.list_repos(tag.as_deref(), recent)?;
            for r in repos {
                println!(
                    "{}\t{}\t{}\t{}",
                    r.last_access_ts.unwrap_or(0),
                    r.name,
                    r.default_branch.unwrap_or_else(|| "-".to_string()),
                    r.path
                );
            }
        }
        Command::Search { query } => {
            let db = db::Db::open(&db_path)?;
            db.init_schema()?;
            let repos = db.search_repos(&query)?;
            for r in repos {
                println!("{}\t{}", r.name, r.path);
            }
        }
        Command::Tag { command } => {
            let db = db::Db::open(&db_path)?;
            db.init_schema()?;
            match command {
                TagCommand::Add { repo, tag } => {
                    db.add_tag_to_repo(&repo, &tag)?;
                    println!("OK");
                }
                TagCommand::Remove { repo, tag } => {
                    db.remove_tag_from_repo(&repo, &tag)?;
                    println!("OK");
                }
                TagCommand::List { repo } => {
                    let tags = match repo {
                        Some(repo) => db.list_repo_tags(&repo)?,
                        None => db.list_tags()?,
                    };
                    for t in tags {
                        println!("{t}");
                    }
                }
            }
        }
        Command::Open { repo } => {
            let db = db::Db::open(&db_path)?;
            db.init_schema()?;
            let path = db.resolve_repo_path(&repo)?.context("repo not found")?;
            db.record_access(&path)?;
            println!("{}", path);
        }
        Command::Prune => {
            let db = db::Db::open(&db_path)?;
            db.init_schema()?;
            let deleted = db.prune_missing_paths()?;
            println!("Pruned {deleted} missing repos.");
        }
        Command::Roots { command } => {
            let mut cfg = config::Config::load_or_create(&cfg_path)?;
            match command {
                RootsCommand::List => {
                    for r in cfg.roots {
                        println!("{r}");
                    }
                }
                RootsCommand::Add { root } => {
                    cfg.add_root(std::path::Path::new(&root));
                    cfg.save(&cfg_path)?;
                    println!("OK");
                }
                RootsCommand::Remove { root } => {
                    cfg.remove_root(std::path::Path::new(&root));
                    cfg.save(&cfg_path)?;
                    println!("OK");
                }
            }
        }
        Command::Serve { host, port } => {
            let _cfg = config::Config::load_or_create(&cfg_path)?;
            let db = db::Db::open(&db_path)?;
            db.init_schema()?;
            web::serve(
                web::AppState {
                    cfg_path,
                    db_path,
                },
                host,
                port,
            )
            .await?;
        }
        Command::CommitIndex {
            all,
            repo,
            branches,
            commits_per_branch,
        } => {
            let mut cfg = config::Config::load_or_create(&cfg_path)?;
            if let Some(v) = branches {
                cfg.commit_index_branches = v.max(1).min(200);
            }
            if let Some(v) = commits_per_branch {
                cfg.commit_index_commits_per_branch = v.max(1).min(500);
            }
            cfg.save(&cfg_path)?;

            let db = db::Db::open(&db_path)?;
            db.init_schema()?;

            let targets: Vec<String> = if all || repo.is_none() {
                db.list_repo_paths()?
            } else {
                vec![repo.unwrap()]
            };

            let mut repos_indexed = 0usize;
            for p in targets {
                if !std::path::Path::new(&p).exists() {
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
            println!(
                "Commit index rebuilt for {} repos (branches={}, commits_per_branch={}).",
                repos_indexed, cfg.commit_index_branches, cfg.commit_index_commits_per_branch
            );
        }
        Command::Ignores { command } => {
            let mut cfg = config::Config::load_or_create(&cfg_path)?;
            match command {
                IgnoresCommand::List => {
                    for n in cfg.ignore_dir_names {
                        println!("{n}");
                    }
                }
                IgnoresCommand::Add { name } => {
                    if cfg.add_ignore_dir_name(&name) {
                        cfg.save(&cfg_path)?;
                    }
                    println!("OK");
                }
                IgnoresCommand::Remove { name } => {
                    if cfg.remove_ignore_dir_name(&name) {
                        cfg.save(&cfg_path)?;
                    }
                    println!("OK");
                }
                IgnoresCommand::Reset => {
                    cfg.reset_ignore_dir_names();
                    cfg.save(&cfg_path)?;
                    println!("OK");
                }
            }
        }
    }

    Ok(())
}
