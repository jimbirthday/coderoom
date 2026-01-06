# CodeRoom

[English README](README.md)

CodeRoom 是一个**离线的本地 Git 仓库管理工具**：扫描目录里的 Git 仓库，建立本地 **SQLite 索引**，提供搜索、标签、提交浏览/提交内容搜索，并内置现代化的 Web 管理页。

## 你可以用它做什么

- 扫描一个或多个 root 目录，索引本机 Git 仓库
- 查看仓库概览（README 摘要、origin remote、最近提交时间）
- 标签管理：添加/删除、按标签筛选、批量打标签、无仓库的标签自动隐藏
- 提交浏览：按分支查看（本地/远程），分页展示，支持查看提交详情
- 搜索：
  - 仓库搜索范围：名称/路径/README/标签
  - 提交搜索范围：摘要/正文 + 可选分支过滤（依赖提交索引）
- 可配置扫描忽略规则（按“目录名”匹配），避免把依赖缓存误当仓库

## 环境要求

- Rust 工具链（stable）：https://rustup.rs

## 代码获取（Clone）

```bash
git clone <REPO_URL>   # 替换为本仓库的 GitHub 地址（或你的 fork）
cd coderoom
```

## 编译 / 安装

```bash
# 本地构建（debug）
cargo build
./target/debug/coderoom --help

# 本地构建（release）
cargo build --release
./target/release/coderoom --help

# 安装到 ~/.cargo/bin
cargo install --path .
coderoom --help
```

## 数据与文件

- 默认数据目录：`~/.coderoom/`
  - `config.toml`：roots、扫描忽略、提交索引参数
  - `coderoom.db`：SQLite 索引（仓库/标签/提交索引等）

## 快速开始（完整流程）

```bash
coderoom init
coderoom scan --root ~/dev --prune   # 索引 root 下的仓库
coderoom serve
```

浏览器打开终端输出的地址（默认：`http://127.0.0.1:8787/`）。

## Web 管理页使用方式

1. 启动服务：`coderoom serve`
2. 左侧添加 root：
   - 优先用“选择…”（best-effort，见下文），或者直接粘贴路径后点“添加”
3. 扫描：
   - 对某个 root 点“扫描”，或点“扫描全部”
   - 如果希望数据库自动清理已删除/移动的仓库记录，勾选“清理已删除/移动”
4. 管理仓库：
   - 点击仓库名打开详情（标签、origin、README 摘要）
   - 点击 `origin` 可复制远程地址
   - 标签可筛选；“批量标签”用于快速给多个仓库打同一个标签
5. 提交：
   - 点击“提交”，选择本地/远程分支，分页浏览提交列表，点开查看详情
6. 提交搜索：
   - 切换到“提交”搜索模式
   - 在设置面板调整索引范围并“重建索引”
   - 搜索提交摘要/正文，可选分支过滤

截图：

![index](/static/index.png "index")

![index](/static/index1.png "index")

## CLI 使用方式

### 常用命令

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

![list](/static/list.png "list")

## 配置说明

配置文件：`~/.coderoom/config.toml`

关键字段：

- `roots`：需要扫描的目录列表
- `ignore_dir_names`：扫描时需要跳过的目录名（按目录名匹配）
- `commit_index_branches`：每个仓库索引最近更新的分支数
- `commit_index_commits_per_branch`：每个分支索引的提交数

示例：

```toml
roots = ["/Users/me/dev"]
ignore_dir_names = [".cargo_home", "node_modules", "target"]
commit_index_branches = 10
commit_index_commits_per_branch = 50
```

## 目录选择（跨平台 best-effort）

浏览器无法直接获取本机“绝对路径”，因此目录选择由后端调用系统对话框完成：

- macOS：`osascript`（系统自带）
- Windows：`powershell` / `pwsh`（WinForms 目录选择框）
- Linux：优先尝试 `zenity` / `kdialog` / `yad`（未安装则回退为手动粘贴路径）

## 扫描忽略规则

CodeRoom 按“目录名”匹配来跳过扫描（可在 Web 设置面板或 `coderoom ignores …` 里配置），用于避免把依赖缓存目录（例如 `.cargo_home/git/checkouts`）当作项目仓库。

修改忽略规则后，建议带清理重扫一次：

```bash
coderoom scan-all --prune
```

## 提交索引（用于提交内容搜索）

提交内容搜索依赖本地索引。建议在以下情况重建：

- 修改了 `commit_index_branches` / `commit_index_commits_per_branch`
- 新增了很多仓库，需要提交搜索覆盖它们

```bash
coderoom commit-index --all
```

## 常见问题

- 扫描出了依赖/缓存仓库：
  - 把对应“目录名”加入 `ignore_dir_names`，然后执行 `coderoom scan-all --prune`
- Linux/Windows 无法弹出目录选择：
  - 该功能是 best-effort；如果系统缺少相关命令，请手动粘贴路径

## 安全与隐私

- 离线优先：核心功能不依赖网络请求。
- `coderoom serve` 主要面向本机使用，避免直接暴露在公网。
