# AGENTS.md — inox-wallpaper

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

壁纸资源加载与显示：解析 `[wallpaper]` 配置、解码图像、计算几何、把帧画到表面上。所属层级：**L3**（背景层客户端，[ADR 0013](../../docs/adr/0013-shell-service-and-interaction-contracts.md)；当前实际表面见下）。

## 表面模式（迁移后仍然是「普通窗口」）

`QGuiApplication` + `QQmlApplicationEngine`，QML 根是 `Window`（`qml/Main.qml`）。这不是临时状态，而是**当前唯一能成立的形态**：`inox-compositor` 不广告 `wlr-layer-shell-unstable-v1`，因此根本创建不出 layer 表面（[ADR 0018](../../docs/adr/0018-l3-component-architecture.md)）。二进制启动时记录一条 `warn!` 说明这点，`placement` 的策略被计算、被测试，但**不被应用**。**不得**在文档或 PR 中把它说成「壁纸已作为背景层生效」；将来接入真实 layer 表面时，`src/placement.rs` 的数据形状是给宿主的输入，不是装饰。

离屏渲染（`ext_session_lock` / `zwlr_layer_surface_v1` 的那种宿主）属于 [ADR 0038](../../docs/adr/0038-offscreen-qml-for-layer-and-lock-surfaces.md)，与本 crate 无关：本 crate 没有、也不应该有离屏 C++ 宿主。

## crate 特有约束

- **`unsafe`**：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）。本 crate 的 `Cargo.toml` 把该 lint 覆盖为 `allow`，唯一原因是 `#[cxx_qt::bridge]` 宏自己会生成 `unsafe extern "C++"` 块；**手写 Rust 里不得出现 `unsafe`**。cxx-qt 的私有 ABI 使用范围由根级 §4 与 [ADR 0035](../../docs/adr/0035-cxx-qt-binding.md) 授权。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- **不建立到 `inox-compositor` 或任何 L3 crate 的 Cargo 依赖**（[ADR 0002](../../docs/adr/0002-layering-and-dependency-direction.md) 规则 1、3、6；ADR 0013 提议取消 wallpaper 直连合成器的例外——本 crate 没有使用该例外）。与合成器只通过 `wlr-layer-shell`、`xdg-shell` 与 `inox-ipc` 协作。新增依赖前先确认方向合法（根级 §2）。
- **`update` 不做 I/O**：解码与文件读取是 `Task::perform`，由 `runtime::Runtime` 在自己的线程上执行，结果作为消息回到 `update`（ADR 0013：图像解码不得阻塞交互循环）。
- **QML↔Rust 只有一个边界**：`src/cxxqt_object.rs` 的 `#[cxx_qt::bridge]`。契约 §3 的形状是规则——**一份 JSON 模型 + 窄动作通道**：
  - 只读属性 `model_json` 是 `src/view.rs` 的整份投影，每次状态变化**整体重发**；
  - `dispatch(action)` 收一行 JSON 动作（目前只有 `{"type":"reload"}`），由 `view::parse_action` 解析，未知动作 `warn!` 后丢弃；
  - `pump()` 排空后台线程送来的消息、跑 `update`、必要时重发投影；
  - `reportReady(title, accessibleItems, imageReady)` 是树回报自检结论的唯一通道。
  **不得**为单个字段新增 `#[qproperty]`：那等于在 QML 边界上再写一份 UI 定义。QML 侧只做 `JSON.parse(app.modelJson)` 与绑定。
- **颜色/尺寸/文案只在 Rust 投影里**（契约 §4）：`qml/` 下不得出现颜色字面量、像素字面量或用户可见英文串。颜色一律是 `theme::qml_color` 产生的 `#AARRGGBB`；尺寸由 spacing / radii / typography 派生，`theme::units` / `theme::px` 把非有限或负的令牌长度压到 0，文字尺寸另外在 `view::text_px` 里兜底为至少 1px。`theme.rs` 顶部的三个文本基准字号常量是唯一例外（等 `inox-config` 提供字号令牌后应移走）。
- **文案**：所有用户可见字符串必须同时存在于 `i18n/en-US/wallpaper.ftl` 与 `i18n/zh-CN/wallpaper.ftl`，并登记在 `src/i18n.rs` 的 `MESSAGES`。目录与代码**双向**一致由 `headless_tests` 断言（`the_two_catalogs_define_the_same_messages_with_the_same_placeholders`、`the_list_the_code_formats_from_is_exactly_what_the_catalogs_define`）。
- **解码有上限**：不得移除 `decode::MAX_PIXELS` / `MAX_DIMENSION` 或 `scaling::MAX_TILES`。无上限的解码与无上限的平铺都是用户可用一个 70 字节文件触发的拒绝服务。
- **降级不得变成黑屏或崩溃**：任何「用户显式选择却用不了」的资源都必须退到主题背景，并携带可读原因（`model::Unusable`）。已有帧不因新请求失败而被清空。
- **内置壁纸的清单在 L1**：标识、文件名、作者与许可只在 `inox_config::wallpapers`；本 crate 不回答「有哪些壁纸」，只把标识解析为搜索目录下的路径（`inox_common::paths::inox_wallpaper_dirs`，`INOX_WALLPAPER_DIR` 可覆盖）。`wallpaper.image` 的取值规则（键省略＝默认壁纸、`none`＝主题背景、标识＝内置壁纸、其余＝路径）见 [ADR 0030](../../docs/adr/0030-bundled-wallpapers-and-the-default-background.md)，必须与本 crate 的 `model::Selection` 保持一致。
- **语义树是算出来的，不是声称的**：`qml/Main.qml` 里每个带 `Accessible.role` 的节点都要计入 `view_model::WallpaperViewModel::accessible_items`。树在 `reportReady` 里回报自己走出来的数量，Rust 把它和投影比对，不一致就**不是**证据。新增/删除一个可见节点必须同时改这两处。

## 当前已验证 / 被阻塞

- **已验证（E0/E1）**：配置解析顺序（含 `*` 通配）、解码失败各路径、五种模式的几何属性、逐输出原子应用、放置策略、状态契约、JSON 投影（双主题 / 双语言 / 200% 字号）、QML 模块在 `QT_QPA_PLATFORM=offscreen` 下加载并回报与投影一致的语义树、命令行退出码。
- **被阻塞（E2/E3）**：合成器不广告 `wlr-layer-shell-unstable-v1`，因此只能以普通窗口预览，`placement` 不被应用（ADR 0018）。**不得**把这一状态说成「壁纸已作为背景层生效」。
- 配置热重载（`SIGHUP` 与 `org.inox.Wallpaper` 的 D-Bus，`inoxctl reload`）由 `src/reload.rs` 接入：在信号/总线线程上**准备**（读配置 + 解码），准备成功后请求才投给窗口线程，应用失败保留旧帧。
- **未做**：E2 级 GUI 证据（waydriver 在虚拟合成器里截图）、AT-SPI 实际桥接验证、layer 表面本身。

## 主要依赖

- 上游 Inox crate：`inox-common`（`i18n`，Fluent 目录与语言协商；`paths`，数据目录与壁纸搜索目录）、`inox-config`（`[wallpaper]` 模型、内置壁纸清单与主题令牌）。
- QML 宿主：`cxx`、`cxx-qt`、`cxx-qt-lib`，以及 build-dependency `cxx-qt-build`。
- 其他第三方：`image` 0.25（仅 `png` + `jpeg`）、`serde_json`（投影）、`calloop`、`zbus`（热重载的会话总线）、`thiserror`、`tracing`、`tracing-subscriber`。
- 测试：`proptest`（≥1000 样本）、`tempfile`。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `QT_QPA_PLATFORM=offscreen cargo test -p inox-wallpaper --all-features` |
| 本 crate 检查 | `cargo check -p inox-wallpaper --all-targets --all-features` |
| 静态检查 | `cargo clippy -p inox-wallpaper --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt -p inox-wallpaper` |
| 本 crate 构建 | `cargo build -p inox-wallpaper --release --all-features` |
| 本 crate 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-wallpaper --no-deps --all-features` |
| 打开预览窗口 | `cargo run -p inox-wallpaper`（需要一个 Wayland 或 X11 会话） |
| 离屏自检 | `QT_QPA_PLATFORM=offscreen cargo run -p inox-wallpaper -- --self-check` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

`QT_QPA_PLATFORM=offscreen` 加在 `cargo test` **外层**：测试进程自己不改环境（edition 2024 下 `std::env::set_var` 是 `unsafe`），需要离屏的子进程由父测试用 `.env("QT_QPA_PLATFORM", "offscreen")` 启动。

## 测试要求

本 crate 是 **bin-only**：没有 `src/lib.rs`，也没有 `tests/`。cxx-qt 0.10 把 bridge 生成的 `cxx_qt_init_*` 定义嵌在声明 bridge 的目标里，同 package 的第二个 target 链接时会 `undefined reference`（KDAB/cxx-qt#770）。因此所有测试都在二进制内部的 `#[cfg(test)]` 模块里。

- 纯逻辑（`model` / `scaling` / `source` / `decode` / `apply` / `contract` / `placement` / `view_model` / `app` / `theme` / `view` / `runtime` / `reload`）必须有单元测试，且**拒绝路径与错误路径**同样覆盖，不只覆盖成功路径。
- 几何（`scaling`）与解码上限用 `proptest`，每项 ≥1000 个有效样本：`fill` 覆盖整个输出并保持宽高比、`fit` 完全落在输出内、任何被缩放的尺寸不为负/零、同一输入给出同一结果、平铺不重叠且从原点铺满。
- `src/headless_tests.rs`：投影测试（颜色等于令牌转换结果、字号/间距等于令牌推导值、文案来自 Fluent，中英各一条）与目录双向一致性检查。
- **QML 加载测试**：`the_qml_preview_loads_offscreen` 把自己作为子进程 spawn（`--ignored --exact --nocapture headless_tests::qml_preview_child_self_check`），在子进程环境里设 `QT_QPA_PLATFORM=offscreen` + `QT_QUICK_BACKEND=software`，跑 `--self-check`，断言根对象非空、`reportReady` 被接受（标题、语义项数、图像状态三者一致）。它还断言子进程输出里没有 `is not a type` / `Cannot assign` / `TypeError` / `ReferenceError` 这类 QML 错误。
- **命令行**：`run_as_process` 同样 spawn 自身跑 `--help` / 未知参数，断言退出码与可读消息，不打开任何 Qt 窗口。
- 测试代码不受「禁止 unwrap/expect/panic」约束。

## 目录结构

```
crates/inox-wallpaper/
  Cargo.toml          工作区继承：字段一律 *.workspace = true；[lints.rust] 把 unsafe_code 覆盖为 allow
  build.rs            CxxQtBuilder::new_qml_module("com.inox.wallpaper.qml").qml_file("qml/Main.qml")
  qml/Main.qml        声明式界面：一个 Window，绑定 modelJson 的各个路径，无颜色/尺寸/文案字面量
  src/main.rs         入口：参数、配置、日志、Qt 宿主（QGuiApplication + QQmlApplicationEngine）、--self-check
  src/cxxqt_object.rs 唯一的 QML↔Rust 边界：model_json + dispatch/pump/reportReady
  src/view.rs         纯投影：状态 + 令牌 → 一份 JSON 文档
  src/theme.rs        令牌 → QML 颜色/尺寸/字重（没有框架主题对象）
  src/runtime.rs      Task<T>、消息队列、线程本地 install/take、pump
  src/reload.rs       SIGHUP / 会话总线 → 准备好的 reload 请求
  src/app.rs          Elm：State / Message / update（纯）
  src/model.rs src/source.rs src/decode.rs src/scaling.rs src/apply.rs src/placement.rs src/contract.rs
  src/view_model.rs   状态 → 画面模型（纯）
  src/i18n.rs         Fluent 目录装配与 MESSAGES 清单
  src/headless_tests.rs  二进制内的投影 / 目录 / CLI / QML 加载测试
  i18n/en-US/ i18n/zh-CN/  Fluent 目录
  README.md           本 crate 说明与当前状态
  AGENTS.md           本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `wallpaper`。
