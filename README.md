# CodeRoom

[中文 README](README.zh-CN.md)

CodeRoom is an **offline local Git repository manager**. It scans directories for Git repos, builds a local **SQLite index**, and provides fast search, tagging, recent access tracking, and commit browsing/search via an embedded web UI.

## Install

```bash
# Build locally
cargo build --release
./target/release/coderoom --help

# Or install to ~/.cargo/bin
cargo install --path .
coderoom --help
```

Data directory: `~/.coderoom/` (contains `config.toml` and `coderoom.db`).

## Quick Start

```bash
coderoom init
coderoom scan --root ~/dev --prune
coderoom serve
```

Open the URL printed in the terminal (default: `http://127.0.0.1:8787/`).

## Common CLI Commands

```bash
# Roots (persisted in ~/.coderoom/config.toml)
coderoom roots list
coderoom roots add ~/dev
coderoom roots remove ~/dev

# Scan / cleanup
coderoom scan --root ~/dev --prune
coderoom scan-all --prune
coderoom prune

# List / search
coderoom list --recent
coderoom list --tag backend
coderoom search "agent"

# Tags
coderoom tag add --repo ~/dev/my-repo backend
coderoom tag remove --repo ~/dev/my-repo backend
coderoom tag list
coderoom tag list --repo ~/dev/my-repo

# Commit index (required for commit-content search)
coderoom commit-index --all --branches 10 --commits-per-branch 50

# Scan ignore list (directory names)
coderoom ignores list
coderoom ignores add .cargo_home
coderoom ignores remove .cargo_home
coderoom ignores reset
```

## Web UI (Embedded)

Key features:

- Language toggle (中文/English) with remembered preference
- Roots management + scan/prune operations
- Repo list with pagination and “recent first”
- Repo overview: name, README excerpt, path, and `origin` remote (click to copy)
- Tags: sidebar shows only tags that still have repos (with counts); inline add/remove + bulk tag
- Commits: browse commits per branch (local + remote), paginated; click for details
- Search:
  - Repo search scopes: name/path/README/tags
  - Commit search scopes: summary/message + optional branch filter
  - Shows “matched in” badges and highlights the query

![index](/static/index.png "index")

## Folder Picker (Cross-platform, Best-effort)

Browsers cannot provide absolute local paths, so the folder picker is implemented server-side:

- macOS: `osascript` (built-in)
- Windows: `powershell`/`pwsh` (WinForms folder dialog)
- Linux: tries `zenity` / `kdialog` / `yad` (if unavailable, paste the path manually)

## Scan Ignore Rules

CodeRoom ignores folders by **directory name match** (configurable in the Web Settings panel or via `coderoom ignores …`). This prevents indexing dependency caches (e.g. `.cargo_home/git/checkouts`).
