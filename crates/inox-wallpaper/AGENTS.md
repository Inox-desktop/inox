# AGENTS.md — inox-wallpaper

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

壁纸资源加载与显示：解析 `[wallpaper]` 配置、解码图像、计算几何、把帧放到每个输出上。所属层级：**L3**（背景层 layer-shell 客户端，ADR 0013；当前状态见下）。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- **不建立到 `inox-compositor` 或任何 L3 crate 的 Cargo 依赖**（ADR 0002 规则 1、3、6；ADR 0013 提议取消
  wallpaper 直连合成器的例外——本 crate 没有使用该例外）。与合成器只通过
  `wlr-layer-shell`、`xdg-shell` 与 `inox-ipc` 协作。新增依赖前先确认方向合法（根级 §2）。
- **`update` 不做 I/O**：解码与文件读取经 `Task::perform` 在运行时线程池执行，结果作为消息回到 `update`
  （ADR 0013：图像解码不得阻塞交互循环）。
- **文案**：所有用户可见字符串必须同时存在于 `i18n/en-US/wallpaper.ftl` 与 `i18n/zh-CN/wallpaper.ftl`，
  并登记在 `src/i18n.rs` 的 `MESSAGES`。`tests/catalog.rs` 断言两个目录 id 与占位符一致；代码里出现
  目录没有的 id 会让界面显示 id 本身。
- **令牌**：颜色、尺寸、间距、圆角、动效只能来自 `inox_config::Theme`，禁止在 `view.rs` / `theme.rs` 写
  字面颜色或像素值。`theme.rs` 顶部的三个文本基准字号常量是唯一的例外，写在一起并在注释里说明
  它们应当在 `inox-config` 提供字号令牌后移走。
- **解码有上限**：不得移除 `decode::MAX_PIXELS` / `MAX_DIMENSION` 或 `scaling::MAX_TILES`。无上限的
  解码与无上限的平铺都是用户可用一个 70 字节文件触发的拒绝服务。
- **降级不得变成黑屏或崩溃**：任何「用户显式选择却用不了」的资源都必须退到主题背景，并携带
  可读原因（`model::Unusable`）。已有帧不因新请求失败而被清空。
- **内置壁纸的清单在 L1**：标识、文件名、作者与许可只在 `inox_config::wallpapers`；本 crate 不回答
  「有哪些壁纸」，只把标识解析为搜索目录下的路径（`inox_common::paths::inox_wallpaper_dirs`，
  `INOX_WALLPAPER_DIR` 可覆盖）。`wallpaper.image` 的取值规则（键省略＝默认壁纸、`none`＝主题背景、
  标识＝内置壁纸、其余＝路径）见 ADR 0030，必须与本 crate 的 `model::Selection` 保持一致。

## 当前已验证 / 被阻塞

- **已验证（E0/E1）**：配置解析顺序（含 `*` 通配）、解码失败各路径、五种模式的几何属性、
  逐输出原子应用、放置策略、状态契约、Iced 界面与像素级渲染测试。
- **被阻塞（E2/E3）**：合成器不广告 `wlr-layer-shell-unstable-v1`，`iced_layershell` 也无法针对
  当前 `iced` 代际编译（ADR 0018），因此二进制以普通窗口运行、只打印一条 `warn!`，
  `placement` 的策略不被应用。**不得**在文档或 PR 中把这一状态说成「壁纸已作为背景层生效」。
- 配置热重载由二进制的 `reload.rs` 接入 `SIGHUP` / 会话 D-Bus（`inoxctl reload`）；资源准备成功后才提交。**未实现**：E2 级 GUI 证据、语义树导出。

## 主要依赖

- 上游 Inox crate：`inox-common`（`i18n`，Fluent 目录与语言协商；`paths`，数据目录与壁纸搜索目录）、
  `inox-config`（`[wallpaper]` 模型、内置壁纸清单与主题令牌）。
- 第三方：`iced` 0.14（`tiny-skia` 渲染、`wayland`、`image`）、`image` 0.25（仅 `png` + `jpeg`）、
  `thiserror`、`tracing`、`tracing-subscriber`。
- 测试：`iced_core` / `iced_runtime` / `iced_tiny_skia`（无头渲染夹具）、`proptest`（≥1000 样本）、`tempfile`。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-wallpaper --all-features` |
| 本 crate 构建 | `cargo build -p inox-wallpaper --release --all-features` |
| 本 crate 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-wallpaper --no-deps --all-features` |
| 静态检查 | `cargo clippy -p inox-wallpaper --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt -p inox-wallpaper` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |
| 写出场景截图 | `INOX_SCREENSHOT_DIR=<dir> cargo test -p inox-wallpaper --all-features --test render` |

## 测试要求

- 纯逻辑（`model` / `scaling` / `source` / `decode` / `apply` / `contract` / `placement` / `view_model` / `app`）
  必须有单元测试，且**拒绝路径与错误路径**同样覆盖，不只覆盖成功路径。
- 几何（`scaling`）与解码上限用 `proptest`，每项 ≥1000 个有效样本：`fill` 覆盖整个输出并保持宽高比、
  `fit` 完全落在输出内、任何被缩放的尺寸不为负/零、同一输入给出同一结果、平铺不重叠且从原点铺满。
- `tests/catalog.rs`：`en-US` 与 `zh-CN` 的 id 与占位符一致（`inox_common::i18n::compare_catalogs`），
  且 `MESSAGES` 与目录内容双向一致。
- `tests/render.rs`：至少四个场景（浅色、深色、空态、错误态）在 1280×720 下绘制并对像素断言；
  断言不得依赖字体度量（断言控件的**背景**令牌，而非字形），否则无字体的机器上会假失败。
- 测试代码不受「禁止 unwrap/expect/panic」约束。

## 目录结构

```
crates/inox-wallpaper/
  Cargo.toml          工作区继承：字段一律 *.workspace = true
  src/                lib.rs、model/source/decode/scaling/apply/placement/contract、
                      app/view_model/view/theme/i18n、main.rs
  i18n/en-US/         Fluent 目录（默认语言）
  i18n/zh-CN/         Fluent 目录（第二语言）
  tests/              catalog.rs、render.rs、support/（无头渲染夹具）
  README.md           本 crate 说明与当前状态
  AGENTS.md           本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）
在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `wallpaper`。
