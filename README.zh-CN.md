# CodeRoom

[English README](README.md)

CodeRoom 是一个**离线的本地 Git 仓库管理工具**：扫描目录里的 Git 仓库，建立本地 **SQLite 索引**，提供搜索、标签、最近访问记录，以及提交浏览/提交内容搜索（内置 Web 管理页）。

## 安装

```bash
# 本地构建
cargo build --release
./target/release/coderoom --help

# 或安装到 ~/.cargo/bin
cargo install --path .
coderoom --help
```

默认数据目录：`~/.coderoom/`（包含 `config.toml` 与 `coderoom.db`）。

## 快速开始

```bash
coderoom init
coderoom scan --root ~/dev --prune
coderoom serve
```

浏览器打开终端输出的地址（默认：`http://127.0.0.1:8787/`）。

## 常用 CLI 命令

```bash
# Roots 管理（写入 ~/.coderoom/config.toml）
coderoom roots list
coderoom roots add ~/dev
coderoom roots remove ~/dev

# 扫描/清理
coderoom scan --root ~/dev --prune
coderoom scan-all --prune
coderoom prune

# 列表/搜索
coderoom list --recent
coderoom list --tag backend
coderoom search "agent"

# 标签
coderoom tag add --repo ~/dev/my-repo backend
coderoom tag remove --repo ~/dev/my-repo backend
coderoom tag list
coderoom tag list --repo ~/dev/my-repo

# 提交索引（提交内容搜索依赖）
coderoom commit-index --all --branches 10 --commits-per-branch 50

# 扫描忽略列表（按“目录名”匹配）
coderoom ignores list
coderoom ignores add .cargo_home
coderoom ignores remove .cargo_home
coderoom ignores reset
```

## Web 管理页（内置）

主要能力：

- 中英双语切换（记忆上次选择）
- Roots 管理 + 扫描/清理
- 仓库列表分页、每页大小可选；支持最近访问优先
- 仓库信息：名称、README 摘要、路径、`origin` remote（点击可复制）
- 标签：左侧按“有仓库的标签”聚合显示（带数量）；列表内支持添加/删除与批量打标签
- 提交：按分支查看提交列表（本地/远程，分页）；点击提交可看详情
- 搜索：
  - 仓库搜索：可勾选 名称/路径/README/标签
  - 提交搜索：可勾选 摘要/正文，支持分支过滤
  - 列表会标记命中来源，并高亮关键词

![index](/static/index.png "index")

## 目录选择（跨平台 best-effort）

浏览器无法直接获取本机“绝对路径”，因此目录选择由后端调用系统对话框完成：

- macOS：`osascript`（系统自带）
- Windows：`powershell` / `pwsh`（WinForms 目录选择框）
- Linux：优先尝试 `zenity` / `kdialog` / `yad`（未安装则回退为手动粘贴路径）

## 扫描忽略规则

CodeRoom 按“目录名”匹配来跳过扫描（可在 Web 设置面板或 `coderoom ignores …` 里配置），用于避免把依赖缓存目录（例如 `.cargo_home/git/checkouts`）当作项目仓库。
