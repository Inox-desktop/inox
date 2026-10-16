# ADR 0009: 配置模型、设计令牌与热重载（L1 配置格式）

- 状态：proposed
- 日期：2026-10-16
- 决策者：架构组

## 背景

PRD §9.1 规定了配置目录布局（`~/.config/inox/` 下的 `config.toml`、`compositor.toml`、`shell.toml`、`notifications.toml` 与 `themes/<name>/theme.toml`），PRD §9.2 规定了主题由**设计令牌**（颜色、排版、间距、圆角、动效时长）构成并要求内置浅色/深色，但没有规定任何一个文件里有哪些字段。根级 `AGENTS.md` §7.1 与 PRD §9.1 另外给出四条硬性要求：TOML + serde、所有字段 `#[serde(default)]` 且提供完整默认值（缺失字段不报错）、解析错误必须给出文件/行号/字段路径、热重载失败时保留旧配置并 `warn!`。

现状与约束：

- 里程碑 M0、M1 已完成，M2 已在 `inox-compositor` 实现（元仓库 README §4）。`inox-config` 仍是 M0 骨架（只有一个 `pub const VERSION`）；合成器目前只解析命令行参数（`inox-compositor/src/config.rs`），**还没有任何组件读取 TOML**。这意味着配置格式现在只在 L1 被定义一次，还没有既成事实需要兼容。
- 但格式的消费方是**所有 L2–L4 组件**：合成器要输出/输入/平铺参数，Shell 与各 L3 组件要面板、启动器、主题令牌，通知守护进程要超时与位置，会话管理器要环境变量。若不在 L1 一次性定下模型，每个仓库都会各自设计一份配置，格式漂移与默认值不一致将无法收敛。
- 本决策需要 ADR 的依据：ADR 索引（`docs/adr/README.md`）把「改变 IPC 或配置格式的破坏性变更」列为必须先写 ADR 的事项；而配置格式是跨仓库（≥5 个仓库）的接口，也是 AI 生成与修改配置时的输入面，schema 的失误会传播到每一个组件，并且往往到对应组件的里程碑才暴露。
- ADR 0002：`inox-config` 属于 L1，依赖只能向下。因此它**不能**依赖 L2 的 `inox-protocols` 去实现 `inoxctl reload` 的服务端；本项目的核心事件循环是 calloop（根级 §4），由各组件拥有，L1 库也不应假定调用者的事件循环模型。
- ADR 0004：只引入 crates.io 上的轻量、纯 Rust、许可证白名单内的依赖，并且只在「首个真实需要它的时刻」引入；`serde` 与 `toml` 已在 M0 骨架中声明于 `inox-config`。
- 根级 §3.3/§3.5：非测试路径禁止 `unwrap()`/`panic!()`；日志统一 `tracing`。根级 §8：跨仓库依赖必须锁定 `tag`。

考虑过但未被采纳的方案：

- **不设 L1 共享模型，各组件自行解析自己的 TOML**：每个仓库重复实现默认值、错误定位与热重载语义，格式必然漂移，且与 M0 骨架已把 `serde`/`toml` 声明在 `inox-config` 的事实矛盾 —— 不成立。
- **改用 JSON / YAML / 自研 DSL**：PRD §4 与技术栈已定 TOML + serde（人类可读、支持注释、AI 易生成）—— 不成立。
- **现在就把四个组件文件（含 `shell.toml`、`notifications.toml`）的 schema 全部定义**：对应组件在 M4/M5 才实现，字段设计没有任何实现反馈，属于臆测；一旦与其真实需求不符，只能改格式（破坏性变更）或永久背负兼容包袱 —— 不成立。
- **未知字段静默忽略（只用 `#[serde(default)]`）**：手写或 AI 生成的配置里出现拼写错误时会静默取默认值，问题以「配置不生效」的形式在运行时出现，违反根级 §13「✗ 吞掉错误」与 PRD §1.3「无静默失败」—— 不成立。
- **在 `inox-config` 内实现 SIGHUP 监听与 `inoxctl reload` 服务端**：前者需要额外的信号依赖并假定调用者的事件循环（本项目的核心循环是 calloop，由各组件拥有）；后者需要 `inox-ipc`（定义于 `inox-protocols`，L2），被 ADR 0002 的依赖方向禁止 —— 不成立。改为在 L1 提供确定性的重载 API，由组件接线。
- **令牌颜色直接使用 Iced 的 `Color` 或引入图形栈类型**：Iced 排在 M4（ADR 0004），L1 不应拉入图形栈；令牌还要服务非 Iced 的消费方（合成器边框、壁纸）—— 不成立。改用自有的 `Rgba` 令牌类型，由消费方在自己的边界转换。

## 决策

1. **文件布局遵循 PRD §9.1。** `config.toml` 是入口：`[environment]` 提供会话环境变量，`[components]` 以**相对配置目录**的文件名指向各组件配置（默认 `compositor.toml`、`shell.toml`、`notifications.toml`），`[theme]` 选择主题。绝对路径与越出配置目录的路径（`..`）一律拒绝。
2. **schema 归 `inox-config` 所有，分两批落地。** 本 ADR 覆盖并实现 `config.toml`、`compositor.toml`、`themes/<name>/theme.toml`；`shell.toml` 与 `notifications.toml` 的字段留到 M4/M5 随对应组件定义，本 ADR 不预设其内容，只保留其路径（此时它们不被解析，因此格式错误也不会在启动期暴露，这是有意的）。
3. **解析规则：缺失字段用默认值，未知字段是错误。** 所有结构体 `#[serde(default)]` 且提供完整默认值；同时 `#[serde(deny_unknown_fields)]`。任何解析失败都必须报出**文件路径 + `行:列` + 字段路径**（实现上由 `toml` 的 span 与 `serde_path_to_error` 提供；确实拿不到 span 时降级为文件 + 字段路径，不得静默）。
4. **默认值必须与既有实现一致。** `compositor.toml` 的输出默认（1280x720@60、scale 1.0）与平铺默认（`master_ratio` 60、`master_count` 1、`gap` 8）同 `inox-compositor` 当前 `src/config.rs` 与 `src/layout.rs` 的常量一致；平铺参数越界（`master_ratio` 不在 10..=90、`master_count` < 1、`gap` < 0）在**解析期报错**，不做静默 clamp。
5. **主题即设计令牌。** `theme.toml` 的字段集合固定为两个可选的**身份**字段 `name`（省略时取目录名）与 `appearance`（`light` / `dark`，省略时继承选择层解析出的外观），以及五个令牌段：`colors`（`background`、`surface`、`surface_variant`、`border`、`text`、`text_muted`、`accent`、`on_accent`、`danger`、`warning`、`success`）、`typography`（`font_family`、`font_mono_family`、`scale`）、`spacing`（`xs`/`sm`/`md`/`lg`/`xl`）、`radii`（`sm`/`md`/`lg`）、`motion`（`fast_ms`/`normal_ms`/`slow_ms`）；颜色写作 `#RRGGBB` 或 `#RRGGBBAA`。内置 `inox-light` 与 `inox-dark`（深色同时是默认值来源）。组件只消费令牌，不得硬编码颜色与尺寸（根级 §4、PRD §9.2）。
6. **主题解析规则。** `[theme] name` 命中内置则用内置，否则读 `themes/<name>/theme.toml`（文件不存在报 `ThemeNotFound`，解析失败报 `Parse`）；`name` 省略时按 `appearance`（`light`/`dark`/`system`，默认 `system`）选择内置。`system` 的实际取值由会话/Shell 决定，并在调用解析时注入；未注入（或注入 `system`）时回退深色。`system` 只属于选择层：主题文件自身只能声明 `light` 或 `dark`，写 `system` 是解析错误。主题文件自带的 `name` / `appearance` 覆盖选择层给出的身份，未声明时继承选择层。自定义主题文件缺失的令牌继承深色默认值。主题名不接受空值、路径分隔符或 `.`/`..` 段。
7. **热重载由 L1 提供语义，接线在组件侧。** `ConfigStore` 持有 `Arc<InoxConfig>`（`RwLock` 保护）：`reload()` 重新读盘，成功则原子替换，失败则**保留旧配置**、`warn!` 记录原因并返回 `Err`（不吞掉错误）。SIGHUP 与未来 `inoxctl reload` 的接线属于组件（calloop `signals`；IPC 服务端属于 L2 的 `inox-protocols` + `inox-ipc`），`inox-config` 不引入信号依赖。
8. **文件缺失不是错误，文件损坏才是。** `config.toml` 或其指向的组件文件不存在时使用默认值并以 `debug!` 记录（首次运行的正常路径）；存在但读不了或解析不了则报错。
9. **依赖面。** `inox-config` 通过锁定 `tag` 的 git 依赖消费 `inox-common`（`v0.2.0` 起提供 `paths` 的 XDG 解析），并按根级 §8 在 `deny.toml` 的 `[sources] allow-git` 登记；运行时依赖限于 `serde`、`toml`、`serde_path_to_error`、`thiserror`、`tracing` 与 `inox-common`。
10. **格式变更必须版本化并另写 ADR。** 删除字段、改名、语义变更或默认值变更都属于破坏性变更：新写 ADR、给出迁移说明，兼容层由组件侧提供，不在 L1 堆积。
11. **路径安全是 L1 的责任。** 主题名与 `[components]` 的路径在进入文件系统前完成校验（拒绝绝对路径、`..`、空值），防止配置把读取范围引出 `~/.config/inox/`；`.` 与 `./x` 不构成逃逸，因此不算非法路径。

## 后果

正面：

- L2–L4 只有一份配置格式与一套默认值来源；新组件「接入配置」等于调用 `inox-config`，而不是重新设计一遍。
- 错误定位（文件 + `行:列` + 字段路径）从第一天就是格式的一部分，人和 AI 改配置时都能自纠。
- `deny_unknown_fields` 把拼写错误变成启动期的明确错误，而不是运行期静默取默认值，符合 PRD §1.3「无静默失败」。
- 依赖方向保持在 L1：`inox-config` 只依赖 `inox-common`，不依赖 L2 的协议或事件循环，符合 ADR 0002。
- 令牌齐备后，M4 的 Iced 组件与合成器、壁纸可以共用同一套颜色与尺寸来源（PRD §9.2 要求「接口先行」）。

负面：

- `deny_unknown_fields` 使「新版本配置 + 旧版本组件」直接报错，跨版本混用需要用户改配置；升级说明必须写清楚。
- 只实现三个文件意味着 L1 的配置能力在 M4/M5 还要再扩展一次（新增 schema 非破坏性，但需同步更新本 ADR 或另写 ADR）。
- 自定义主题缺失的令牌继承深色默认值：只写了一半令牌的浅色主题会拿到深色令牌，必须靠文档明示（`inox-docs` 的主题页）。
- 重载语义的一致性依赖各组件都调用同一个 `ConfigStore`，审查时需要检查；擅自自建配置读取的组件会绕过「失败保留旧配置」的保证。
- `serde_path_to_error` 是本决策引入的依赖（MIT OR Apache-2.0），用于拿到字段路径；它随 `serde` 生态演进，属于额外的供应链面。

影响面：`inox-config`（本次实现）、`inox-common`（`paths`，`v0.2.0`）、后续 `inox-compositor`（接入 `compositor.toml` 时校验默认值一致性）、M4 的 Shell 组件（`shell.toml` 与主题令牌）、M5 的通知（`notifications.toml`）与 `inox-docs` 的配置参考页。元仓库 `compat.toml` 在 `inox-common v0.2.0` 与 `inox-config` 发布后登记。

取代条件：若出现「配置状态需要跨组件实时共享」或「配置必须支持 TOML 之外的来源（D-Bus/Portal 写入）」的强需求，需新写 ADR 取代本文件；届时本 ADR 状态改为 `superseded`，不删除、不重写。
