# AGENTS.md — inox-panel

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

面板：任务栏、系统托盘与时钟（PRD §3.1）。所属层级：**L3**。它不拥有窗口状态、也不拥有托盘状态：窗口状态来自合成器，托盘项来自各自的应用程序，面板只订阅并呈现（ADR 0013）。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`；默认级别 `info`，不得每帧打印。
- **窗口身份只来自服务端**：`WindowKey` = 服务端分配的 `WindowId` + `SessionGeneration`。禁止用 PID、标题或列表下标识别窗口，也禁止用它们做分组（ADR 0013 决策 1）。分组只按服务端给出的 `app_id`。
- **代次不匹配的状态一律丢弃**：跨代次的窗口状态返回 `Applied::Stale`，代次变化必须先 `TaskList::resync`，不得把两个会话拼在一起。
- **颜色/尺寸/圆角/动效只来自令牌**（`inox_config::Theme`）：`src/theme.rs` 是唯一把令牌变成颜色与样式的地方，`src/view.rs` 只调用它。文字尺寸由面板高度与 `typography.scale` 推导（令牌集没有字号），不得写死像素。
- **文案只来自 Fluent**：只能出现消息 id，不得在代码里写用户可见字符串（产品名与组件名除外）。新增 id 必须同时进 `en-US` 与 `zh-CN`，占位符保持一致（`tests/catalog.rs` 断言）。
- **摆放策略是数据，不是已生效的行为**：`src/placement.rs` 计算锚点/exclusive zone/键盘交互等，但当前没有 layer-shell 宿主可应用它（ADR 0018）。启动时只允许**一条** `warn!` 说明表面是普通窗口，不得声称摆放已生效，也不得引入 `iced_layershell`。
- **启动必须自检**：没有可用的 Wayland/X11 显示时，`main` 必须以可读错误与非零退出码结束，不得把控制权交给第三方 crate 后收获内部 panic（ADR 0018）。`zwlr_layer_shell_v1` 的存在性查询尚未实现：面板不与合成器建连，该自检随 layer-shell 宿主一并落地。
- **窗口服务与 SNI 传输不在本 crate 内**：两者的协议仍不实现。配置重载由二进制 `reload.rs` 通过会话 D-Bus 与 calloop 信号源接入（ADR 0026），不接管窗口或托盘状态。`update` 不做 I/O，只把意图放进 `State` 的 `OutgoingCommand` 队列；`Message` 是传输层要喂进来的接口。新增 `Message` 必须同时给出测试。
- **任务区是弹性区域**：任务按钮放在 `Length::Fill` + `clip` 的容器里，时钟与托盘项是 `Shrink` 子项（iced 先排非弹性子项），因此长标题只会被截断，不会把时钟挤出面板。显示数量受 `panel.max_task_buttons` 限制，不得用滚动或溢出指示替代（尚未实现）。
- **普通展示不夺焦点**：`keyboard_interactivity = None`；时钟与托盘项是纯展示容器，不套 `button`。显式导航焦点只能落在可点击（enabled）的条目上，且至多一个（D01）。
- 新增依赖前先确认依赖方向合法（根级 §2）：本 crate 只依赖 L1（`inox-common`、`inox-config`），不得引入同层或合成器依赖。

## 主要依赖

- 上游 Inox crate：`inox-common`（i18n 目录加载）、`inox-config`（shell 配置、设计令牌）。
- 第三方：`iced` 0.14（`tiny-skia` 渲染器）、`chrono`（本地时间）、`thiserror`、`tracing`、`tracing-subscriber`、`anyhow`（仅 `main`）。
- 测试：`iced_core` / `iced_runtime` / `iced_tiny_skia`（离屏渲染）、`image`（截图产物）、`proptest`。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-panel --all-features` |
| 本 crate 构建 | `cargo build -p inox-panel --release --all-features` |
| 本 crate 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-panel --no-deps --all-features` |
| 静态检查 | `cargo clippy -p inox-panel --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt -p inox-panel` |
| 渲染截图 | `INOX_SCREENSHOT_DIR=<dir> cargo test -p inox-panel --test render` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`），测试代码不受「禁止 unwrap/expect/panic」约束。
- **属性测试**（`proptest!`，`ProptestConfig::with_cases(1000)`）覆盖任务模型：
  - 分组是划分——没有窗口丢失或重复，分组内窗口的 `app_id` 与分组一致；
  - 至多一个窗口处于 focused（因此每组至多一个激活项）；
  - 每组的最后一个窗口离开后分组消失；
  - 顺序稳定：无关窗口更新不改变任何其它窗口的相对次序；
  - 每一步之后都断言 `order` 与窗口集合一致（无重复、无幽灵）。
- 托盘：`register` 幂等、未注册条目的事件被拒绝（`TrayError::UnknownItem`）、菜单项可跨子菜单查找、**同一快照应用两次不产生任何变化**、快照删除已消失条目并加入新条目。
- 时钟：`CLOCK_DIRECTIVES` 的**每个**指令都被渲染，`%Q` 之类报错（含位置），结尾孤立 `%` 报错，zh-CN 的星期/月份与 `%p` 与 en-US 不同，`%Z` 为 UTC 偏移。
- 投影：D05 的 starting / empty / unavailable（消息 + 重试）各有断言；禁用条目必须给出**可读原因**；超长标题被 `MAX_LABEL_CHARS` 截断；导航顺序只含 enabled 条目。
- `tests/catalog.rs`：`compare_catalogs(en-US, zh-CN)` 必须干净（Q03），id 前缀为 `panel-` 且不含点号，未翻译的 id 必须回退到默认语言。
- `tests/render.rs`：1280×32 下至少浅色、深色、空态与不可用态四个场景（Q02），断言缓冲区长度、背景像素等于 `surface` 令牌、启用与禁用条目渲染不同、超长标题后时钟仍在右侧，以及点击任务按钮产生 `Message::TaskActivated` 并且只产生一次激活请求。`INOX_SCREENSHOT_DIR` 有值时写出 `inox-panel-<场景>.png`。
- 覆盖率目标：核心纯逻辑（`tasks`、`tray`、`clock`、`placement`、`view_model`、`contract`）≥70%（E1）。

## 目录结构

```
src/lib.rs          crate 文档、模块声明与再导出
src/tasks.rs        窗口/任务分组模型（纯）
src/tray.rs         StatusNotifierItem 与 DBusMenu 模型（纯）
src/clock.rs        时钟格式展开（纯 + Fluent）
src/contract.rs     服务状态、请求关联与去重（纯）
src/placement.rs    layer-shell 摆放策略（数据，未应用）
src/view_model.rs   状态 → 视图投影（纯）
src/app.rs          State / Message / update（Elm）/ view / subscription
src/view.rs         Iced 控件与 PanelGeometry
src/theme.rs        令牌 → iced 主题与样式
src/i18n.rs         内嵌 Fluent 目录
src/main.rs         日志、配置、目录装配，然后运行 iced 应用
i18n/en-US/panel.ftl, i18n/zh-CN/panel.ftl
tests/support/mod.rs  离屏渲染与点击驱动
tests/render.rs       渲染与点击测试
tests/catalog.rs      目录一致性
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `panel`。
