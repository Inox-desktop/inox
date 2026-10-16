# Inox M0 骨架规范（唯一事实来源 / Single Source of Truth）

本文件由主控代理维护，所有 sub agent **必须完整阅读并严格遵守**。
目标：为 Inox 桌面环境的每个组件仓库生成一致的 M0 骨架。
规范冲突时，以根级 `AGENTS.md` 的硬性规则为准；本文件只是其可执行的落地版本。

> 本文件是临时工件，M0 完成后由主控代理删除，不属于交付物。

---

## 0. 硬性规则提醒（违反将被主控代理打回）

1. **不要本地编译/测试**：禁止运行 `cargo build` / `cargo test` / `cargo clippy` / `cargo run` / `cargo deny`。CI 才是唯一验证途径。
2. 允许的本地操作：`cargo fmt` **仅当工具链已安装 rustfmt**（缺失则跳过）；`git` 操作；创建文件。
3. **禁止 `unsafe`**：每个 crate 在 `Cargo.toml` 的 `[lints.rust]` 设置 `unsafe_code = "forbid"`。
4. 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
5. **日志用 `tracing`**，禁止 `println!` / `eprintln!`。库用 `thiserror`，可执行程序用 `anyhow`。
6. 依赖许可证白名单：仅 BSD/MIT/Apache-2.0/ISC/Zlib 等宽松许可；禁止 GPL/AGPL/SSPL。
7. **代码注释与标识符、docstring 用英文**；`README.md` / `AGENTS.md` 等面向人的文档用**中文**。
8. 提交信息用 **Conventional Commits**：`<type>(<scope>): <subject>`，type ∈ feat/fix/docs/refactor/perf/test/build/ci/chore/revert，scope 用组件短名（如 `common`、`compositor`）。
9. 保持改动聚焦：只创建本规范列出的文件，不额外发挥、不引入未要求的依赖或抽象。
10. `pub` 项必须有文档注释（配合 `missing_docs` lint）。

---

## 1. 目录布局（主控代理已决定，不要更改）

```
/home/rownix/Projects/Inox/          # 元仓库 inox（由 Agent F 处理）
├── AGENTS.md  PRD.md  M0-SPEC.md
├── repos/                            # 所有组件仓库的容器（元仓库 .gitignore 忽略）
│   ├── inox-common/     ← 各自独立的 git 仓库（git init + 一次初始提交）
│   ├── inox-config/
│   ├── ...
│   └── .github/
```

- 每个组件仓库 = `repos/<name>/` 下一个**独立 git 仓库**。
- 由于各 agent 只写自己负责的 `<name>/` 目录，天然无文件冲突，**不使用 git worktree**。
- 不要修改 `repos/` 之外的任何文件（元仓库文件由 Agent F 独占）。

---

## 2. 仓库矩阵（本表是权威清单）

| 仓库 | 类型 | 主要依赖（M0 仅 crates.io，见 §5） |
|------|------|-----------------------------------|
| `inox-common` | lib | `tracing`, `thiserror` |
| `inox-config` | lib | `serde`(derive), `toml`, `thiserror`, `tracing` |
| `inox-docs` | mdbook（无 Rust crate） | — |
| `inox-compositor` | bin | `anyhow`, `tracing`, `tracing-subscriber`(env-filter) |
| `inox-protocols` | lib | `wayland-client`, `wayland-server` |
| `inox-xwayland` | bin | `anyhow`, `tracing`, `tracing-subscriber`(env-filter) |
| `inox-shell` | bin | 同 bin 通用 |
| `inox-panel` | bin | 同 bin 通用 |
| `inox-launcher` | bin | 同 bin 通用 |
| `inox-notifications` | bin | 同 bin 通用 |
| `inox-settings` | bin | 同 bin 通用 |
| `inox-lock` | bin | 同 bin 通用 |
| `inox-wallpaper` | bin | 同 bin 通用 |
| `inox-session` | bin | 同 bin 通用 |
| `inox-idle` | bin | 同 bin 通用 |
| `inox-greet` | bin | 同 bin 通用 |
| `inox-integration` | bin | 同 bin 通用 |
| `.github` | 组织级仓库（无 Rust、无 CI） | — |

**bin 通用依赖**：`anyhow = "1"`, `tracing = "0.1"`, `tracing-subscriber = { version = "0.3", features = ["env-filter"] }`

各仓库的中英文描述：

| 仓库 | package description（英文，用于 Cargo.toml） | README 一句话（中文） |
|------|---------------------------------------------|----------------------|
| inox-common | Shared types, error handling and logging utilities for the Inox desktop | 共享类型、错误处理与日志工具 |
| inox-config | Configuration model and TOML parsing for the Inox desktop | 配置模型与 TOML 解析 |
| inox-compositor | The Inox Wayland compositor, built on Smithay | 基于 Smithay 的 Wayland 合成器 |
| inox-protocols | Wayland protocol bindings and Inox protocol extensions | 标准协议绑定与自定义协议扩展 |
| inox-xwayland | XWayland integration service for the Inox compositor | XWayland 集成服务 |
| inox-shell | The Inox desktop shell framework | Shell 主框架与插件系统 |
| inox-panel | The Inox desktop panel | 面板（任务栏、托盘、时钟） |
| inox-launcher | The Inox application launcher | 应用启动器 |
| inox-notifications | The Inox notification daemon | 通知守护进程 |
| inox-settings | The Inox system settings application | 系统设置应用 |
| inox-lock | The Inox session lock screen | 锁屏 |
| inox-wallpaper | The Inox wallpaper renderer | 壁纸设置与渲染 |
| inox-session | The Inox session manager | 会话管理器 |
| inox-idle | The Inox idle management daemon | 空闲管理 |
| inox-greet | The Inox display manager (greeter) | 显示管理器 |
| inox-integration | End-to-end integration tests for the Inox desktop | 端到端集成测试 |

层级（写入各仓库 AGENTS.md）：

| 仓库 | 层级 | 职责 |
|------|------|------|
| inox-common / inox-config / inox-docs | L1 | 基础设施与共享库 |
| inox-compositor / inox-protocols / inox-xwayland | L2 | 合成器与窗口管理 |
| inox-shell / inox-panel / inox-launcher / inox-notifications / inox-settings / inox-lock / inox-wallpaper | L3 | Shell 与用户界面 |
| inox-session / inox-idle / inox-greet | L4 | 会话与系统集成 |
| inox-integration | 测试 | 端到端集成 |
| .github | 组织 | 组织级模板 |

---

## 3. 每个 Rust 仓库必须包含的文件

```
<repo>/
├── AGENTS.md
├── Cargo.toml
├── LICENSE              # §4.1 逐字复制
├── README.md
├── rust-toolchain.toml  # §4.2
├── deny.toml            # §4.3
├── .gitignore           # §4.4
├── .github/workflows/ci.yml   # §4.5（docs 仓库用 §4.6）
├── Cargo.lock           # 无需手写：由 cargo 生成，但不得提交？→ 见下
└── src/
    ├── lib.rs           # lib 仓库：§5.1
    └── main.rs          # bin 仓库：§5.2
```

> `Cargo.lock`：M0 不生成、不提交（不在本地编译，无法生成）。CI 首次运行会产生；这是预期行为，不需要处理。

---

## 4. 共享文件模板（逐字复制，仅替换 `{{...}}` 占位符）

占位符：`{{NAME}}` = 仓库名（如 `inox-common`）；`{{DESC_EN}}` = 英文描述；`{{DESC_ZH}}` = 中文一句话；`{{LAYER}}` = 层级（如 `L2`）；`{{SCOPE}}` = 提交 scope 短名（如 `common`）。

### 4.1 LICENSE（所有仓库，逐字）

```
BSD 3-Clause License

Copyright (c) 2026, Inox Desktop Contributors
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### 4.2 rust-toolchain.toml（逐字）

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

### 4.3 deny.toml（逐字）

```toml
[advisories]
yanked = "warn"

[licenses]
allow = [
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "MIT",
    "Zlib",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "Unlicense",
    "CC0-1.0",
    "BSL-1.0",
    "MPL-2.0",
]

[bans]
multiple-versions = "warn"
wildcards = "deny"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

> 说明：M0 不引入 Inox 跨仓库 git 依赖，因此 `unknown-git = "deny"` 不会误伤。后续里程碑引入 git 依赖时需在 `[sources] allow-git` 放行。

### 4.4 .gitignore（逐字）

```
/target
```

### 4.5 .github/workflows/ci.yml（Rust 仓库，逐字）

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always
  RUSTDOCFLAGS: -D warnings

jobs:
  ci:
    name: fmt / clippy / test / build / doc / deny
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Cache
        uses: Swatinem/rust-cache@v2

      - name: Format
        run: cargo fmt --all -- --check

      - name: Clippy
        run: cargo clippy --all-targets --all-features -- -D warnings

      - name: Test
        run: cargo test --all-features

      - name: Build
        run: cargo build --release --all-features

      - name: Doc
        run: cargo doc --no-deps --all-features

      - name: Install cargo-deny
        uses: taiki-e/install-action@v2
        with:
          tool: cargo-deny

      - name: Deny
        run: cargo deny check
```

### 4.6 .github/workflows/ci.yml（`inox-docs` 专用，逐字）

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  docs:
    name: mdbook build
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Install mdBook
        uses: peaceiris/actions-mdbook@v2
        with:
          mdbook-version: latest

      - name: Build book
        run: mdbook build
```

### 4.7 README.md 模板

```markdown
# {{NAME}}

{{DESC_ZH}}。属于 Inox 桌面环境的 **{{LAYER}}** 层。

## 状态

M0 骨架阶段：仅包含仓库结构、CI 与文档；功能实现从后续里程碑开始。

## 开发约定

- 本项目遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与仓库级 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，不在本地进行编译验证。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)。

## 许可证

BSD 3-Clause，见 [`LICENSE`](./LICENSE)。
```

### 4.8 AGENTS.md 模板（仓库级，遵循根级 §12 模板）

```markdown
# AGENTS.md — {{NAME}}

本仓库遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充仓库特有内容，不放宽任何根级约束。

## 职责

{{DESC_ZH}}。所属层级：**{{LAYER}}**。

## 仓库特有约束

- 默认禁止 `unsafe`：`Cargo.toml` 中 `[lints.rust] unsafe_code = "forbid"`。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- M0 阶段**不引入 Inox 跨仓库依赖**：跨仓库依赖在首次需要它的里程碑中引入，并按根级 §8 使用锁定 `tag` 的 git 依赖。

## 主要依赖

{{DEPS}}

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

| 目的 | 命令 |
|------|------|
| 格式化 | `cargo fmt --all` |
| 格式检查 | `cargo fmt --all -- --check` |
| 静态检查 | `cargo clippy --all-targets --all-features -- -D warnings` |
| 测试 | `cargo test --all-features` |
| 构建 | `cargo build --release --all-features` |
| 文档 | `cargo doc --no-deps --all-features` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`）。
- 测试代码不受“禁止 unwrap/expect/panic”约束。
- M0 骨架阶段暂无业务逻辑，测试随功能里程碑补充。

## 目录结构

```
src/            源码（lib.rs 库入口 / main.rs 可执行入口）
tests/          集成测试（按需创建）
.github/        CI 工作流
deny.toml       cargo-deny 依赖与许可证策略
```

## 提交信息格式

Conventional Commits，scope 使用 `{{SCOPE}}`。
```

`{{DEPS}}` 的取值：按 §2 表格填写，例如 lib 仓库写 `` `tracing`, `thiserror` ``；bin 仓库写 `` `anyhow`, `tracing`, `tracing-subscriber` ``。

---

## 5. Cargo 与源码模板

### 5.1 Cargo.toml（lib 仓库）

```toml
[package]
name = "{{NAME}}"
description = "{{DESC_EN}}"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
license = "BSD-3-Clause"
repository = "https://github.com/Inox-desktop/{{NAME}}"
readme = "README.md"
keywords = ["wayland", "desktop", "inox"]
categories = ["desktop"]

[dependencies]
{{DEPS_TOML}}

[lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"

[lints.clippy]
all = "warn"
```

### 5.2 Cargo.toml（bin 仓库）

与 §5.1 相同，仅将 `keywords`/`categories` 保留，且依赖固定为：

```toml
[dependencies]
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

### 5.3 src/lib.rs（lib 仓库）

```rust
//! {{DESC_EN}}.
//!
//! Part of the Inox desktop environment. See the repository `AGENTS.md` for
//! contribution and architecture constraints.

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
```

`inox-config` 额外允许（不强制）在 lib.rs 顶部加一行 `//!` 说明其 TOML 职责；不要添加其它代码。

`inox-protocols` 的 lib.rs 同上，仅文档文字不同。

### 5.4 src/main.rs（bin 仓库）

```rust
//! {{DESC_EN}}.

use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    info!("{{NAME}} starting");

    Ok(())
}
```

> 注意：`unwrap_or_else` 属于安全 API，允许使用；禁止的是 `unwrap()` / `expect()`。

### 5.5 Cargo.toml 依赖片段速查

| 仓库类型 | `[dependencies]` |
|----------|------------------|
| inox-common | `tracing = "0.1"` / `thiserror = "2"` |
| inox-config | `tracing = "0.1"` / `thiserror = "2"` / `serde = { version = "1", features = ["derive"] }` / `toml = "0.8"` |
| inox-protocols | `wayland-client = "0.31"` / `wayland-server = "0.31"` |
| 其它 bin | 见 §5.2 |

> M0 允许声明尚未使用的依赖（骨架阶段刻意保留 PRD 中列出的依赖面）。

---

## 6. 非 Rust 仓库模板

### 6.1 inox-docs（mdbook，无 Cargo.toml）

文件：`LICENSE`(§4.1)、`README.md`、`AGENTS.md`(§4.8 变体，命令表格改为 mdbook)、`book.toml`、`.gitignore`（内容 `book/`）、`.github/workflows/ci.yml`(§4.6)、`src/SUMMARY.md`、`src/introduction.md`。

`book.toml`：

```toml
[book]
title = "Inox Docs"
authors = ["Inox Desktop Contributors"]
language = "zh-CN"
src = "src"

[output.html]
git-repository-url = "https://github.com/Inox-desktop/inox-docs"
```

`src/SUMMARY.md`：

```markdown
# Summary

- [简介](./introduction.md)
```

`src/introduction.md`：3–6 句中文，说明 Inox 是什么、文档范围、并链接到 PRD 与根级 AGENTS.md。

`AGENTS.md` 的「命令」表格替换为：

| 目的 | 命令 |
|------|------|
| 构建文档 | `mdbook build` |
| 本地预览 | `mdbook serve` |

### 6.2 `.github`（组织级仓库，无 Rust、无 CI）

文件：
- `LICENSE`（§4.1）
- `README.md`（说明这是组织级模板仓库）
- `AGENTS.md`（简短版：本仓库只放组织级模板/Issue 模板，不放代码）
- `profile/README.md`（组织首页简介，中文，链接 PRD 与根级 AGENTS.md）
- `CONTRIBUTING.md`（中文，说明 AI-first 流程、CI 门禁、Conventional Commits）
- `PULL_REQUEST_TEMPLATE.md`（中文，含：动机 / 改动 / 影响仓库 / 测试方式 / 性能影响 / 关联 ADR）
- `ISSUE_TEMPLATE/bug_report.yml` 与 `ISSUE_TEMPLATE/feature_request.yml`（GitHub Issue Forms，`name`/`description`/`body` 结构，字段中文标签）
- `workflow-templates/rust-ci.yml`（内容 = §4.5 的 ci.yml 去掉 `on:` 之外保持不变，用于组织级复用）
- `workflow-templates/rust-ci.properties.json`：

```json
{
  "name": "Inox Rust CI",
  "description": "Inox 组件仓库的标准 CI：fmt / clippy / test / build / doc / deny。",
  "iconName": "rust",
  "categories": ["Rust", "Continuous integration"]
}
```

---

## 7. 元仓库（Agent F 专用）

在 `/home/rownix/Projects/Inox/` 下创建/维护（**不要动 `repos/`**）：

- `git init`（默认分支 `main`）。
- `.gitignore`（内容 `repos/`）—— 必须在 `git add` 之前写好。
- `LICENSE`（§4.1）。
- `README.md`（中文）：项目简介、架构分层、仓库清单表、里程碑表（M0–M6）、如何贡献、许可证。
- `compat.toml`：组件兼容矩阵，字段见下。
- `docs/adr/README.md`：ADR 索引与流程说明（状态流转 proposed → accepted → superseded）。
- `docs/adr/0000-template.md`：MADR 精简模板。
- `docs/adr/0001-polyrepo-strategy.md`：采用多仓库策略 + 初始仓库清单。
- `docs/adr/0002-layering-and-dependency-direction.md`：L1–L4 分层与依赖只能向下。
- `docs/adr/0003-technology-stack.md`：Rust 2024 + Smithay + calloop + Iced + TOML/serde + tracing。
- `docs/adr/0004-m0-skeleton-and-dependency-policy.md`：M0 骨架不引入跨仓库/重型依赖（Smithay/Iced 延后到 M1/M4），CI 为空提交全绿；记录理由。
- `.github/workflows/docs.yml`：markdown 检查（用 `npx --yes markdownlint-cli2 "**/*.md" "!repos/**"`；并加 `.markdownlint.json` 关闭行长度限制 `MD013`、允许内联 HTML `MD033`）。

ADR 模板/正文用中文，格式：

```markdown
# ADR NNNN: <标题>

- 状态：accepted
- 日期：2026-10-16
- 决策者：架构组

## 背景

## 决策

## 后果
```

`compat.toml` 结构：

```toml
# Inox 组件兼容矩阵（Integration compatibility matrix）
# 由 inox-integration 消费；M0 阶段尚未发布，version 为占位。

[meta]
version = "0.1.0"
updated = "2026-10-16"

[components]
# 组件发布后在此登记：<name> = "<tag>"
```

---

## 8. 提交要求（每个组件仓库）

```bash
cd repos/<name>
git init -b main
git add .
git commit -F - <<'EOF'
chore({{SCOPE}}): 初始化 {{NAME}} 仓库骨架

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

元仓库同理，使用 `chore(meta): 初始化 inox 元仓库（PRD/ADR/兼容矩阵）`。

---

## 9. 交付报告格式（每个 agent 必须返回）

1. 创建的仓库列表，每个仓库逐一列出实际写入的文件路径。
2. 是否有偏离本规范之处；没有就写「无」。
3. 是否执行了 `cargo fmt`（是/否，否的原因）。
4. 每个仓库的 `git log --oneline` 输出。
5. 任何无法完成的事项。
