# CodeRoom

[中文 README](README.zh-CN.md)

CodeRoom is an **offline local Git repository manager**. It scans directories for Git repos, builds a local **SQLite index**, and provides fast search, tagging, commit browsing/search, and a modern embedded Web UI.

## What you can do

- Index local repos under one or more “roots”
- View repo overview (README excerpt, origin remote, last commit time)
- Tag repos, filter by tag, bulk tag, and auto-hide empty tags
- Browse commits by branch (local + remote), paginated, with commit details
- Search:
  - Repo search scopes: name/path/README/tags
  - Commit search scopes: summary/message + optional branch filter (requires commit index)
- Configure scan ignore rules (directory-name match) to avoid dependency caches

## Requirements

- Rust toolchain (stable): https://rustup.rs

## Clone

```bash
git clone <REPO_URL>   # replace with the GitHub URL of this repo (or your fork)
cd coderoom
```

## Build / Install

```bash
# Build locally (debug)
cargo build
./target/debug/coderoom --help

# Build locally (release)
cargo build --release
./target/release/coderoom --help

# Install to ~/.cargo/bin
cargo install --path .
coderoom --help
```

## Data & Files

- Data directory: `~/.coderoom/`
  - `config.toml`: roots + scan ignores + commit index limits
  - `coderoom.db`: SQLite index (repos/tags/commit index)

## Quick Start (end-to-end)

```bash
coderoom init
coderoom scan --root ~/dev --prune   # index repos under a root
coderoom serve
```

Open the URL printed in the terminal (default: `http://127.0.0.1:8787/`).

## Using the Web UI

1. Start the server: `coderoom serve`
2. Add a root on the left:
   - Use “Pick…” when available (best-effort; see below), or paste a path and click “Add”
3. Scan:
   - Click “Scan” on a root, or “Scan all”
   - If you want the DB to drop repos that were deleted/moved, enable “Prune moved”
4. Manage repos:
   - Click a repo name for details (tags, origin, README excerpt)
   - Click `origin` to copy remote URL
   - Use tags to filter; use “Bulk tag” to tag many repos quickly
5. Commits:
   - Click “Commits”, pick a branch (local/remote), browse commits and open details
6. Commit search:
   - Switch search mode to “Commits”
   - Adjust commit index limits in Settings, then “Rebuild index”
   - Search commit summary/message; optionally set a branch filter

Screenshots:

![index](/static/index.png "index")

![index](/static/index1.png "index")

## Using the CLI

### Common commands

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

![list](/static/list.png "list")

## Configuration

Config file: `~/.coderoom/config.toml`

Key fields:

- `roots`: directories to scan
- `ignore_dir_names`: directory names to skip during scanning (directory-name match)
- `commit_index_branches`: number of recently-updated branches to index per repo
- `commit_index_commits_per_branch`: commits per branch to index

Example:

```toml
roots = ["/Users/me/dev"]
ignore_dir_names = [".cargo_home", "node_modules", "target"]
commit_index_branches = 10
commit_index_commits_per_branch = 50
```

## Folder picker (cross-platform, best-effort)

Browsers cannot provide absolute local paths, so the folder picker is implemented server-side:

- macOS: `osascript` (built-in)
- Windows: `powershell`/`pwsh` (WinForms folder dialog)
- Linux: tries `zenity` / `kdialog` / `yad` (if unavailable, paste the path manually)

## Scan ignore rules

CodeRoom ignores folders by **directory name match** (configurable in Web Settings or via `coderoom ignores …`). This prevents indexing dependency caches (e.g. `.cargo_home/git/checkouts`).

After changing ignores, re-scan with pruning:

```bash
coderoom scan-all --prune
```

## Commit index (for commit-content search)

Commit-content search uses a local index. Rebuild when:

- You change `commit_index_branches` / `commit_index_commits_per_branch`
- You add many repos and want commit search to cover them

```bash
coderoom commit-index --all
```

## Troubleshooting

- Dependency/cache repos show up:
  - Add their directory name to `ignore_dir_names` and run `coderoom scan-all --prune`
- Folder picker not available:
  - It is best-effort; paste the path manually if the helper command is missing

## Security / Privacy

- Offline-first: no network calls are required for core features.
- `coderoom serve` is intended for localhost usage; avoid exposing it publicly.
