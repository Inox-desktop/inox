# AGENTS.md — Inox Desktop

> 本文件是 Inox 项目**面向 AI 代理的根级约束文档**。任何代理在开始工作前必须先完整阅读本文件，再阅读所在 crate 的 `AGENTS.md`。
> crate 级 `AGENTS.md` 只能**追加/收紧**约束，**不得放宽或违反**本文件的硬性规则。
> 目标读者：AI 编码代理（主要）、人类贡献者（次要）。

---

## 0. 项目一句话

Inox 是一个用 **Rust (stable, 2024 edition)** 编写、基于 **Smithay + QML/Qt Quick** 的模块化 Wayland 桌面环境。
采用 **AI-first** 开发：AI 写代码，人类做架构决策、PR 审查与真实硬件验收。**CI（GitHub Actions）是唯一构建与测试验证途径。**

- 组织：https://github.com/Inox-desktop
- 代码组织：**单一仓库（Cargo workspace）**，组件在 `crates/`，文档在 `docs/`（ADR 0016，取代 ADR 0001 的多仓库策略）
- 许可证：LGPL-3.0（仓库根目录有 `LICENSE`；ADR 0034）
- 产品需求以本仓库的 `PRD.md` 为准。
- 架构决策以本仓库的 `docs/adr/` 为准。

---

## 1. 适用范围与优先级

1. 代理必须同时遵守 **根级 `AGENTS.md`（本文件）** 与 **当前 crate 的 `AGENTS.md`**。
2. 冲突时：crate 级规则优先，**但**凡属于第 3 节“硬性规则”的条目，任何 crate 级文档都不得放宽。
3. 用户当次指令与本文档冲突时，以用户当次指令为准，但需在 PR 中说明理由。
4. 若某 crate 没有 `AGENTS.md`，视为仅受本文件约束，并应尽快按 §12 模板补齐。

---

## 2. 工作区 crate 地图（分层，依赖只能向下）

```
L1  inox-common, inox-config
L2  inox-protocols, inox-auth, inox-compositor, inox-xwayland
L3  inox-shell, inox-panel, inox-launcher, inox-notifications,
    inox-settings, inox-lock, inox-wallpaper,
    inox-qml-prototype (ADR 0033/0035 的 QML+Qt Quick 迁移原型)
L4  inox-session, inox-idle, inox-greet, inox-host
测试 inox-integration
```

- 全部组件 crate 都是本仓库 `crates/` 下同一个 Cargo workspace 的成员（ADR 0016，取代 ADR 0001 的多仓库策略）。用户与开发者文档站点 `inox-docs` 位于 `docs/book/`，无 `Cargo.toml`，不参与依赖图，也不受 Rust 类规则约束。
- 随发行版安装的**数据文件**（当前只有内置壁纸）位于仓库根 `data/`，不属于任何 crate，也不参与依赖图：壁纸在 `data/wallpapers/`，安装到 `$XDG_DATA_DIRS/inox/wallpapers/`，其标识与署名清单在 L1 的 `inox-config`（ADR 0030）。
- **层级成员的唯一事实来源**是根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`；本节清单与 PRD §3.1 的组件表都必须与它一致。
- `inox-common` 不得依赖任何其他 Inox crate。
- 合成器与 Shell **不通过 Rust 依赖**耦合，只通过 Wayland 协议 / D-Bus / IPC 通信。
- 禁止反向依赖、同层横向依赖、循环依赖。
- 以上规则**由 CI 机械强制**，不再只靠约定：`cargo test -p inox-integration --test layering`（验收项 B04）读取真实依赖图逐条断言，失败信息指明具体的边与被违反的规则条目。新增 crate 而未在层级表登记会直接失败。

---

## 3. 硬性规则（Hard Rules，任何 crate 不可违反）

违反以下任一条的改动将被直接拒绝：

1. **不在本地编译/测试。** 禁止把本地 `cargo build` / `cargo test` 结果作为验收依据。验收一律以 CI 为准。
   - 允许的本地操作：`cargo fmt`、`cargo check`、`cargo clippy`（作为快速自检，仅作参考，不构成通过依据）。若环境没有工具链，直接跳过，交给 CI。
2. **默认禁止 `unsafe`。** crate 根使用 `#![forbid(unsafe_code)]` 或 `#![deny(unsafe_code)]`。
   - 唯二例外：`inox-compositor`、`inox-xwayland` 中封装系统调用 / FFI 的**最小**胶水代码。例外代码必须：局部化、包裹在安全 API 内、附带 `// SAFETY: <理由>` 注释。
3. **禁止在非测试路径使用 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。**
   - `main` 顶层可返回 `anyhow::Result`。测试与 `#[cfg(test)]` 除外。
4. **错误处理**：库 crate 用 `thiserror` 定义错误类型；可执行程序用 `anyhow`。禁止吞掉错误（`let _ = ...` 需有明确理由）。
5. **日志**：统一使用 `tracing` 宏（`info!`/`debug!`/…）。**禁止 `println!` / `eprintln!`**（测试与示例除外）。默认级别 `info`，禁止高频刷屏。
6. **依赖许可证白名单**：允许宽松许可 + LGPL（`deny.toml` 的 `[licenses] allow`）。项目本身为 **LGPL-3.0**（ADR 0034）。**禁止 AGPL / SSPL** 进入核心依赖；GPL-only 的 Qt 模块（Qt Wayland Compositor、Qt Virtual Keyboard、Qt QML Compiler）无论任何形式都禁止引入（ADR 0033）。由 `cargo-deny` 强制检查 Cargo 依赖图；**系统库许可由独立的 `license-audit` CI job 校验**（`cargo-deny` 管不到系统 Qt `.so`）。
7. **提交信息必须遵循 Conventional Commits**（见 §9）。
8. **禁止绕过 CI**：不得使用 `--no-verify`、不得直接推送 `main`、不得修改或禁用分支保护与检查。
9. **禁止引入静默网络行为/遥测**：不得默认开启网络监听、不得上传用户数据。
10. **面向用户的 UI 文案禁止硬编码**：必须走 Fluent（见 §7.3）。

---

## 4. 架构约束

- 必须使用 **Smithay** 抽象实现合成器，直接操作 DRM/KMS 之外的底层接口需 ADR 批准。
- 必须使用 **calloop** 作为主事件循环；禁止自建线程轮询循环处理核心事件。
- 渲染与输入处理不得阻塞事件循环；耗时任务放入 calloop 的异步/线程机制。
- Shell 组件基于 **QML + Qt Quick**（ADR 0033、ADR 0035，经 cxx-qt 绑定；Iced 已从依赖图移除，见 ADR 0036、ADR 0039）。领域模型与逻辑留在 Rust，UI 交给声明式 QML；合成器始终是 **Smithay**，绝不使用 Qt Wayland Compositor（GPL-only）。
- 迁移后允许在**局部**使用 Qt 私有 ABI 与 Inox 私有合成器协议（ADR 0039），前提是产物保持原生 Qt Quick、私有 ABI 的范围在 crate 级 `AGENTS.md` 记录，且 GPL-only Qt 模块仍然禁止。
- 有 Qt 窗口的 Shell 组件由 Qt 事件循环驱动；`inox-panel` 与 `inox-lock` 保留自己的 calloop 主循环，QML 仅做离屏渲染（ADR 0038），这两处不得退化为普通窗口。
- 只消费主题令牌（design tokens），**禁止在组件中硬编码颜色/尺寸**（见 PRD §9）。
- 新增 Wayland 协议前，必须先在 `inox-protocols` 定义/引用，并说明安全与生命周期约束。
- 需要改变分层、依赖方向、核心依赖或 IPC/配置格式的破坏性变更时，**必须先写 ADR**（PRD §5.4）并获得批准。

---

## 5. 代码风格

- 格式化：`rustfmt`（默认配置，无 `rustfmt.toml` 覆盖时不得自定义）。提交前本地跑 `cargo fmt`。
- Lint：`clippy --all-targets --all-features`，**零警告**（CI 用 `-D warnings`）。
- 命名：
  - crate 名：`inox-<component>`（连字符）。
  - 二进制名：`inox-<component>`（PRD 中的简写 `inox-comp` 为 `inox-compositor` 产出的别名）。
  - 标识符：`snake_case`（函数/变量/模块）、`CamelCase`（类型）、`SCREAMING_SNAKE_CASE`（常量）。
- **代码注释与标识符用英文；面向用户的文档与 PRD 用中文。** 注释默认不写，仅在逻辑不直观时写；不写“复述代码”的注释。
- 所有 `pub` API 必须有文档注释（`///`），`cargo doc` 不得产生缺失文档警告。
- 保持改动聚焦：不顺手重构无关代码，不引入未被要求的抽象/配置/兼容层。

---

## 6. 测试要求

- 每个 crate：对纯逻辑必须有单元测试；核心算法用 `proptest`（窗口管理、布局）。
- 合成器：提供 **headless 集成测试**，软件渲染（`LIBGL_ALWAYS_SOFTWARE=1`），验证输出创建、渲染、输入事件、干净退出。
- Shell / GUI：用 **waydriver** 在虚拟 Wayland 合成器（`weston --backend=headless`）中启动、截图、断言。
- 覆盖率：核心 crate 行覆盖默认 ≥ 70%（可在 crate 级 `AGENTS.md` 收紧，不得放宽到 0）。
- 禁止 mock 掉被测的核心逻辑；只在系统边界（DRM、输入设备、D-Bus）打桩。
- 新增或修改行为必须同时新增/更新对应测试。

---

## 7. 各领域约定

### 7.1 配置（`inox-config`）

- 格式 TOML + serde，所有字段 `#[serde(default)]`，提供完整默认值，缺失字段不报错。
- 解析错误必须包含文件、行号、字段路径。
- 支持 `SIGHUP` / `inoxctl reload` 热重载；重载失败保留旧配置并 `warn!`。
- 配置目录布局见 PRD §9.1。

### 7.2 IPC

- 跨组件通信优先使用标准 Wayland 协议或 D-Bus；Inox 私有通信走 `inox-ipc`（定义于 `inox-protocols`）。**不依赖 Wayland 连接的私有契约**（如登录认证的 `auth-v1`，它必须早于任何合成器存在）同样在 `inox-protocols/protocol/` 下有规范文本，但类型与实现留在说它的 crate 里（ADR 0026、ADR 0028）。
- 私有协议为 unstable，破坏性变更需版本化并在 PRD 协议矩阵更新。

### 7.3 国际化（i18n）

- 所有用户可见字符串走 **Project Fluent**（`fluent-rs`），禁止硬编码。
- 新增文案必须同步默认语言 `en-US`；保持占位符一致。

### 7.4 无障碍（a11y）

- Shell 组件必须保持键盘可导航，并暴露语义树（供 AT-SPI 桥接）。
- 尊重系统字体缩放与高对比度主题令牌。

---

## 8. 依赖管理

- 工作区**内部**的 Inox crate 之间使用 **path 依赖**，统一在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次，成员用 `{ workspace = true }` 引用：

  ```toml
  # 根 Cargo.toml
  [workspace.dependencies]
  inox-common = { path = "crates/inox-common", version = "0.1" }

  # crates/inox-config/Cargo.toml
  [dependencies]
  inox-common.workspace = true
  ```

  path 依赖必须带显式 `version`，否则其隐式 `*` 会被 `deny.toml` 的 `[bans] wildcards = "deny"` 判为通配符依赖而失败。**禁止**再对 Inox crate 使用 git 依赖或 `tag` 锁定。
- 第三方依赖同样只在 `[workspace.dependencies]` 声明版本，成员用 `{ workspace = true }` 引用并按需追加自己的 feature。版本升级只改这一处；禁止成员各自绑定不同版本。
- 工作区**外部**的依赖仍必须锁定 `tag` 或 `rev`；禁止使用未锁定的 `branch = "main"`。
- MSRV 由根 `Cargo.toml` 的 `[workspace.package] rust-version` 统一取值（当前 1.89）：成员一律 `rust-version.workspace = true`，不得各自声明。
- 新增依赖需在 PR 描述中说明用途与许可证；优先选择已有生态中被广泛使用的 crate。
- 不随意升级 major 版本；升级需跑通 CI 并说明破坏性影响。
- 依赖安全：`cargo deny check`（许可证 + RUSTSEC 漏洞 + 重复版本）针对**完整依赖图**必须通过。

---

## 9. 提交与 PR 规范

### 9.1 Commit（Conventional Commits）

格式：`<type>(<scope>): <subject>`

- `type` ∈ `feat` `fix` `docs` `refactor` `perf` `test` `build` `ci` `chore` `revert`。
- `scope` 使用 crate/模块名（省略 `inox-` 前缀），如 `compositor`、`config`、`panel`。
- 示例：
  - `feat(compositor): 支持 headless 后端创建输出`
  - `fix(config): 热重载失败时保留旧配置`
  - `ci: 添加 cargo-deny 检查`
- 破坏性变更在 subject 后加 `!`，并在 `BREAKING CHANGE:` footer 说明。

### 9.2 PR

- 一个 PR 聚焦一件事；描述必须包含：动机、改动、受影响的 crate、测试方式、性能影响（如有）。
- 涉及架构/协议/依赖方向变更时链接对应 ADR。
- 必须通过：分支保护 + CI 全绿 + ≥1 名人类审查。
- 由 AI 修复 CI 失败时，在 PR 追加说明改动了什么。

---

## 10. 命令参考

> 强调：以下命令**仅供参考**，不作为本地验收依据；真正的判定在 CI。

| 目的 | 命令 |
|------|------|
| 格式化 | `cargo fmt --all` |
| 格式检查 | `cargo fmt --all -- --check` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 全量测试 | `cargo test --workspace --all-features` |
| **单个 crate 测试** | `cargo test -p <crate> --all-features` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 构建 | `cargo build --workspace --release --all-features` |
| 文档 | `cargo doc --workspace --no-deps --all-features` |
| 依赖检查 | `cargo deny check` |
| 用户文档站点 | `mdbook build docs/book` |
| headless 渲染 | `LIBGL_ALWAYS_SOFTWARE=1 cargo run -p inox-compositor -- --headless` |

单 crate 命令与 CI 的测试矩阵逐条一致（见下），因此「本地这样跑过」与「CI 上这样跑」是同一件事；但结果仍**不作为验收依据**。

**CI 期望**：`.github/workflows/ci.yml` 的全部 job 通过 —— `fmt`、`clippy`、`cargo-deny`、`doc`、release 构建，以及**按 crate 分片**的测试矩阵、XWayland 端到端与 mdbook 构建；无 clippy 警告。测试矩阵由 `plan` job 从层级表推导（一个 crate 只要登记了层级，就必然被构建与测试），因此分片不会漏 crate，失败也可归因到具体 crate。

---

## 11. AI 代理工作流

1. 阅读本文件 + 当前 crate `AGENTS.md` + 相关 PRD/ADR。
2. 明确任务范围，只改动必要文件。
3. 实现代码并补齐测试/文档/文案（i18n）。
4. 本地仅做轻量自检（`fmt` / `check` / `clippy`，可选），**不把结果当验收**。
5. 推送到功能分支，创建 PR（遵循 §9）。
6. 读取 CI 日志，自动修复，直到全绿；无法解决时在 PR 留言并请求人类介入。
7. 等待人类审查与合并；**不自行合并到 `main`**。

### 常见任务配方

- **新增一个协议**：在 `inox-protocols` 定义 → 合成器集成 → 加 headless 测试 → 更新 PRD 协议矩阵。
- **新增一个组件 crate**：先写 ADR（新增 crate 属破坏性架构变更）→ 在 `crates/<name>` 建骨架（`Cargo.toml`、`src/`、`README.md`、`AGENTS.md`，manifest 字段一律 `*.workspace = true`）→ **在根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]` 登记层级**（不登记则 `layering.rs` 失败）→ 更新根 `AGENTS.md` §2 与 PRD §3.1。
- **在 crate 之间新增依赖**：先确认方向合法（§2）→ 在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明 → 成员 `{ workspace = true }` 引用 → 跑 `cargo test -p inox-integration --test layering`。
- **升级依赖版本**：改根 `Cargo.toml` 的 `[workspace.dependencies]` 一处 → 跑 CI。工作区内部不再有 tag 升级流程。
- **新增 UI 功能**：Rust 投影（`view.rs`）+ QML 组件（`qml/`）→ 走 Fluent 文案 → 用主题令牌 → 投影断言 + `QT_QPA_PLATFORM=offscreen` 的 QML 加载测试；需要真实宿主时用 waydriver 截图测试。

---

## 12. crate 级 `AGENTS.md` 模板

新 crate 请以本模板创建 `crates/<name>/AGENTS.md`，只补充该 crate 特有内容：

```markdown
# AGENTS.md — <crate-name>

本 crate 遵循根级 `AGENTS.md`（https://github.com/Inox-desktop/inox/blob/main/AGENTS.md）的全部硬性规则。

## 职责
<一句话职责 + 在分层中的位置（与根 Cargo.toml 的层级表一致）>

## crate 特有约束
- <例如：本 crate 允许受限 unsafe，范围：...>
- <例如：额外禁止项>

## 主要依赖
<同工作区的 Inox crate（path 依赖）；关键第三方 crate>

## 命令
| 目的 | 命令 |
|------|------|
| ... | ... |

## 测试要求
<本 crate 的测试类型、覆盖率阈值>

## 目录结构
<关键目录说明>
```

---

## 13. 禁止事项速查（Do NOT）

- ✗ 在本地把 build/test 结果当作验收
- ✗ 新增 `unsafe`（除非 §3.2 的例外并加 SAFETY 注释）
- ✗ `unwrap()` / `expect()` / `panic!()` 于非测试路径
- ✗ `println!` / `eprintln!` 记录日志
- ✗ 引入 AGPL/SSPL 依赖，或引入 GPL-only 的 Qt 模块（Qt Wayland Compositor / Qt Virtual Keyboard / Qt QML Compiler）
- ✗ 绕过 CI / 修改分支保护 / 直推 `main`
- ✗ 硬编码用户可见文案或颜色尺寸
- ✗ 未写 ADR 就做架构/依赖方向/协议破坏性变更
- ✗ 顺手重构无关代码或过度抽象
- ✗ 提交无 Conventional Commits 前缀的信息

---

*本文件是活文档；修改硬性规则需经人类批准并同步更新各 crate `AGENTS.md`。*
