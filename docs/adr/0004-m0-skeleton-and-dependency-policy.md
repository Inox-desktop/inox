# ADR 0004: M0 骨架与依赖策略（不引入跨仓库依赖与重型依赖）

- 状态：accepted
- 日期：2026-10-16
- 决策者：架构组

## 背景

PRD §13 把 M0 的验收标准（DoD）定义为：「所有仓库含骨架、`AGENTS.md`、`LICENSE`、CI；**空提交 CI 全绿**」。也就是说 M0 阶段真正的交付物不是功能，而是一条**可信、可复现、全绿的流水线**加上一致的仓库骨架。后续每个里程碑（M1–M6）都要站在这条流水线之上迭代，因此 M0 的正确性判据只有一条：在尚未实现任何业务逻辑时，CI 必须从第一次运行起就是绿的。

这与 PRD §5.1「CI 是唯一真相」和根级 `AGENTS.md` §3.1「不在本地编译/测试，验收一律以 CI 为准」叠加后，产生了一个关键约束：**M0 阶段没有任何本地编译验证手段**。骨架中的依赖声明无法在本地解析、编译或检查，其正确性完全依赖 CI。因此 M0 的依赖面必须小到「几乎不可能出错」。

候选依赖分三类：

1. **crates.io 上的轻量纯 Rust 依赖**（`anyhow`、`tracing`、`tracing-subscriber`、`thiserror`、`serde`、`toml`、`wayland-client`/`wayland-server`）。可在 M0 引入：解析可靠、无系统库依赖、许可证在白名单内，且能真实检验 `fmt`/`clippy`/`test`/`build`/`doc`/`deny` 全链路。
2. **Inox 跨仓库 Git 依赖**（例如 `inox-config = { git = "https://github.com/Inox-desktop/inox-config", tag = "v0.1.0" }`）。
3. **重型系统依赖**（Smithay、calloop、Iced、libseat、EGL/GL、libinput、DRM/KMS 相关 crate）。

第 2、3 类在 M0 都是**不可接受**的，理由如下。

### 为什么 M0 排除跨仓库 Git 依赖（unresolvable git deps）

- **依赖不可解析**：M0 是所有仓库的初始提交，任何仓库都**尚未发布 tag**（元仓库 `compat.toml` 的 `[components]` 段为空，`version` 仅为占位）。指向 `tag = "v0.1.0"` 的 Git 依赖在 M0 时刻必然解析失败，`cargo metadata` / `cargo build` 会直接报错，CI 首次运行即红 —— 违背 PRD §13 的 DoD。
- **CI 环境没有兄弟仓库**：GitHub Actions 的 `actions/checkout@v4` 只检出当前仓库。Inox 的跨仓库依赖只能经由 GitHub 上的 tag 拉取，而 tag 尚不存在。本地把各仓库聚合在 `repos/` 下的布局是开发便利，不是 CI 现实。
- **与 `deny.toml` 冲突**：M0 骨架的 `deny.toml` 固定包含 `[sources] unknown-git = "deny"`（架子镜像各组件仓库的 `deny.toml`）。任何未在 `[sources] allow-git` 中登记的 Git 源都会被 `cargo deny check` 拒绝。放行它意味着在 M0 就打开一个本不需要的安全口子，显然得不偿失。
- **顺序依赖**：根级 `AGENTS.md` §8 规定的跨仓库升级流程是「先升上游并发布 tag → 再更新下游 Git 依赖 → 最后在 `inox-integration` 验证兼容性」。M0 连第一次发布都还没有，引入 Git 依赖等于跳过整个流程。

### 为什么 M0 排除重型系统依赖（Smithay / Iced / calloop / libseat 等）

- **声明即无用**：M0 的源码是骨架 —— lib 仓库只有一个 `pub const VERSION`，bin 仓库只有 `tracing_subscriber` 初始化 + 一行 `info!`。没有任何代码路径使用 Smithay 或 Iced。声明它们只会让 CI 花时间编译一堆永远不会被链接的代码。
- **系统库与构建面**：Smithay 的 `backend_drm` / `backend_libinput` / EGL 渲染路径需要系统开发库（DRM、libinput、libseat、EGL/GL 头文件与 pkg-config）。Iced 会拉入完整图形栈。这些都会显著扩大 CI 的失败面（缺失系统包、版本漂移、上游 API 变动），而 M0 的目标恰恰是「让流水线尽可能简单、尽可能绿」。
- **cargo-deny 扫描面**：重型依赖带来大量传递依赖，`cargo deny check` 的许可证白名单（BSD/MIT/Apache-2.0/ISC/Zlib，禁止 GPL/AGPL/SSPL）与重复版本检查面随之膨胀。M0 阶段引入等于把依赖治理风险提前到一个还无法验证的时刻。
- **架构决策已由 ADR 0003 覆盖**：Smithay / Iced 的使用是被 PRD §4 与根级 `AGENTS.md` §4 强制要求的（**必须**使用 Smithay 抽象、**必须**使用 calloop、Shell **必须**基于 Iced）。M0 不声明它们不会造成架构漂移 —— 它们不是被否决，只是被排期。
- **延迟的收益为零**：把依赖声明提前到 M0，不会让 M1/M4 的实现更快；它只会让 M0 的 CI 更可能红。M0 的 CI 红会破坏「基线可信」这一前提，使后续所有里程碑无法区分「是我的改动坏了」还是「基线本来就是坏的」。

### 排期（依赖在哪个里程碑到达）

| 依赖 / 能力 | 到达里程碑 | 依据 |
|-------------|-----------|------|
| `anyhow`、`tracing`、`tracing-subscriber`、`thiserror`、`serde`、`toml` | **M0**（crates.io，轻量纯 Rust） | PRD §4；根级 `AGENTS.md` §3.4、§3.5 |
| `wayland-client` / `wayland-server`（`inox-protocols`） | **M0**（crates.io，纯 Rust，无系统库依赖） | PRD §3.1；`inox-protocols` 职责 |
| **Smithay + calloop + libseat + DRM/libinput 后端** | **M1**（headless 合成器） | PRD §13 M1；ADR 0003 |
| Inox 跨仓库 Git 依赖（`inox-common` → `inox-compositor` 等） | **第一个真正需要它的里程碑**（M1 起） | 根级 `AGENTS.md` §8；各仓库 `AGENTS.md` 仓库特有约束 |
| **Iced**（Shell 与 L3 组件） | **M4**（Iced Shell：面板、启动器、设置） | PRD §13 M4；ADR 0003 |
| **xwayland-satellite** | **M3**（XWayland 集成） | PRD §13 M3 |
| **proptest / waydriver / Tracy** | M2 / M4 / M6（首个需要它的里程碑） | PRD §7、§13 |

## 决策

1. **M0 是纯骨架阶段**：每个组件仓库只包含仓库结构（`AGENTS.md`、`Cargo.toml`、`LICENSE`、`README.md`、`rust-toolchain.toml`、`deny.toml`、`.gitignore`、`.github/workflows/ci.yml`、`src/lib.rs` 或 `src/main.rs`）、CI 与文档。不实现任何业务逻辑。
2. **M0 只允许 crates.io 上的轻量、纯 Rust、许可证在白名单内的依赖**：lib 为 `tracing` / `thiserror`（`inox-config` 另加 `serde`(derive) 与 `toml`；`inox-protocols` 为 `wayland-client` / `wayland-server`），bin 为 `anyhow` / `tracing` / `tracing-subscriber`(env-filter)。
3. **M0 禁止 Inox 跨仓库 Git 依赖。** 首个真正需要它的里程碑（M1）才引入，并按根级 `AGENTS.md` §8 使用锁定 `tag` 的 Git 依赖，同时在 `deny.toml` 的 `[sources] allow-git` 中登记来源。
4. **M0 禁止重型系统依赖**：Smithay、calloop、Iced、libseat、DRM/libinput/EGL 相关 crate、xwayland-satellite 等一律排除，分别在 M1（合成器侧）、M4（Shell 侧）、M3（XWayland）引入。
5. **M0 允许声明尚未被使用的依赖**（仅限第 2 条列出的 crates.io 依赖）。这是刻意的：它让 CI 真实执行依赖解析 + 许可证检查，从而在 M0 就验证 `cargo-deny` 与依赖图是通的，而不是等到 M1 才发现白名单配错。
6. **不生成、不提交 `Cargo.lock`**：M0 不在本地编译，无法生成 lock 文件；CI 首次运行会生成它，这是预期行为。
7. **CI 必须在空提交上全绿**：M0 的文件就位后，任一仓库的首次 CI 运行（`fmt` → `clippy -D warnings` → `test` → `build --release` → `doc` → `deny check`）必须全绿且无警告；`inox-docs` 用 mdBook 构建工作流，元仓库 `inox` 用 markdownlint 工作流。这是 PRD §13 M0 的 DoD。
8. **元仓库 `inox` 在 M0 只承载文档与索引**：`PRD.md`、`AGENTS.md`、`LICENSE`、`README.md`、`compat.toml`、`docs/adr/` 与 markdown lint 工作流；不含产品代码，不参与依赖图。

## 后果

正面：

- M0 的 CI 失败面被压到最小，PRD §13 的「空提交 CI 全绿」可达成，为 M1–M6 提供可信基线。
- `cargo-deny` 与依赖解析链路在 M0 就被真实验证，许可证白名单配置错误会在最有价值的时刻暴露。
- 元仓库 `compat.toml` 在 M0 保持 `[components]` 为空，与「尚无发布」的事实一致，避免出现虚假的兼容性声明。
- 每个依赖的引入都绑定到「第一个真正需要它的里程碑」，避免先声明后遗忘的死依赖。

负面：

- M0 的 `Cargo.toml` 与 PRD §3.1 的「主要依赖」列并不一致（后者描述的是最终形态）。这是有意为之：PRD 描述目标依赖面，M0 描述可验证的最小依赖面。各仓库 `AGENTS.md` 的「主要依赖」小节按 M0 实际声明填写，并在 M1/M3/M4 逐个补齐。
- 引入 Smithay（M1）与 Iced（M4）时会伴随 CI 配置的实质变化（新增系统依赖、构建时长上升、可能新增缓存策略），这两个 PR 的风险和体量会明显大于普通 PR。
- 跨仓库 Git 依赖首次引入（M1）需要同时完成「上游打 tag」「下游声明依赖」「`deny.toml` 放行 `allow-git`」三件事，属于跨仓库原子操作，必须谨慎排序。
- M0 阶段无法提前发现 Smithay / Iced 与 edition 2024 / MSRV 1.85 的兼容性问题；这类风险被推迟到 M1/M4 才暴露。

影响面：本决策直接定义 M0（PRD §13）全部仓库的 `Cargo.toml`、`deny.toml` 与 CI 工作流内容；其「排期表」是 M1–M4 引入依赖时的检查清单。

取代条件：若 M0 期间出现必须引入重型依赖才能满足扩大后的 DoD 的情形（例如 DoD 被修订为「必须通过一次真实渲染」），需撰写取代本 ADR 的新 ADR，并同步修订 PRD §13 的 M0 行与各仓库 `AGENTS.md` 的依赖小节。
