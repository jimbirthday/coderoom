# CodeRoom

本项目是一个本地仓库管理器：扫描目录里的 Git 仓库，建立本地索引（SQLite），提供搜索、标签、最近访问等能力；完全离线。

## 快速开始

```bash
cargo run -- init
cargo run -- scan --root ~/dev
cargo run -- list
cargo run -- search "agent"
cargo run -- tag add --repo ~/dev/my-repo backend
cargo run -- list --tag backend
cargo run -- serve
```

默认数据目录：`~/.coderoom/`（包含 `config.toml` 与 `coderoom.db`）。

## 常用命令

```bash
# Roots 管理
cargo run -- roots list
cargo run -- roots add ~/dev
cargo run -- roots remove ~/dev

# 扫描/清理
cargo run -- scan-all --prune
cargo run -- prune

# Web 管理页
cargo run -- serve --host 127.0.0.1 --port 8787
```

Web 页面能力：
- 中英双语切换（右上角按钮，自动记忆）
- Roots 管理 + 扫描/清理
- 仓库搜索、标签筛选、标签增删、最近访问记录
- 一键复制仓库路径
- 分支（本地/远程）选择 + 提交记录（分页）
- 提交内容搜索（依赖本地提交索引，可在页面里重建索引）

目录选择说明：
- Web 页面无法直接访问本机文件系统；因此我们通过后端调用系统原生对话框。
- 当前仅 macOS 支持“选择…”按钮（基于 `osascript choose folder`），其他平台会提示你手动粘贴路径。

提交索引默认策略：
- 默认索引：最近有提交的 `10` 个分支 × 每分支 `50` 个提交（可在页面“提交搜索”里修改并重建）。
- CLI 也可重建：`cargo run -- commit-index --all --branches 10 --commits-per-branch 50`

Web 使用提示：
- 搜索支持勾选范围：`仓库/提交`，以及仓库字段（名/路/README/标）。
- 搜索命中会在列表里显示 `name/path/readme/tag` 等标识，帮助判断命中来源。
