# AGENTS.md — inox-launcher

本 crate 遵循根仓库的**全部硬性规则**（[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)），本文件只补充本 crate 特有内容，不放宽任何硬性规则。

## 职责

应用启动器：XDG 应用索引、搜索与排序、按条目规范启动并验证结果。所属层级：**L3**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

表面模式：**普通窗口**。`main` 引导 `QGuiApplication` + `QQmlApplicationEngine`，加载内嵌的 `qml/Main.qml`（`Window` 根）；QML 只渲染 Rust 的 JSON 投影并通过一个窄动作通道回报。

## crate 特有约束

- **`unsafe` 例外仅限 cxx-qt 生成的胶水**：`[lints.rust] unsafe_code = "allow"` 覆盖工作区的 `forbid`，因为 `#[cxx_qt::bridge]` 宏必然生成 `unsafe extern "C++"`。Inox 手写的 Rust 仍不写 `unsafe`；不要为了绕开某条约束去写字面 `unsafe`。
- **不引入私有 ABI**：本 crate 只用 cxx-qt / Qt 的公开 API（`QGuiApplication`、`QQmlApplicationEngine`、`QUrl`、`QString`）。需要 Qt 私有头或私有 Qt Quick 接口时先改 ADR 记录范围，再动代码。
- **bin-only**：没有 `src/lib.rs`，所有模块挂在 `main.rs` 的 `mod` 上。cxx-qt 0.10 把 bridge 的 `cxx_qt_init_*` 定义嵌进声明 bridge 的 target，独立库 target / `tests/` 集成测试会以 `undefined reference` 链接失败（KDAB/cxx-qt#770，契约 §2.1）。因此断言只能写在 `#[cfg(test)] mod catalog_tests` / `headless_tests` 里。
- **QML↔Rust 只有一条数据通道和一条动作通道**：`model_json`（整份投影，每次变化整体重发）与 `dispatch(json)`；不要为每个字段加 `#[qproperty]`，也不要在 QML 里做业务决策（契约 §3）。
- **QML 里没有颜色、像素或用户可见文案**：`qml/Main.qml` 只绑定 `vm.*`（契约 §4）。新增视觉值先加到 `view.rs` 的投影，再在 QML 里绑定；文案永远来自 Fluent。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。`State::new` 会读文件系统，但失败都以 `ServiceState::Unavailable` + 可读原因呈现，不 panic。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- **`Exec` 只能展开成 argv。** 本 crate 不得出现返回命令行字符串的 API，也不得把搜索词传入任何进程启动路径：`exec::ExecContext` 没有查询字段，`exec::argv` 返回 `Vec<String>`。这是验收 D02「不把搜索词拼成 shell 命令」的结构性保证，破坏它需要重写而非放宽。
- **`update` 里不做 I/O。** 读索引与等待被启动进程的证据都在 `crate::runtime::Task` 里（扫描走 `Task::perform`，探针走独立线程），结果经 `Queue` 回到 Qt 线程的 `pump()`。`State::new` 的首次扫描发生在窗口存在之前，不占用交互循环。
- **启动辅助是同一个二进制，不是第二个 bin**：`main` 见到 `--exec-application` 就装好 SIGHUP 遮罩并 `exec` 目标程序，所以被启动的应用不以本 crate 的线程/信号状态运行。`cargo build -p inox-launcher` 只产出一个 target（bin-only 也正因如此不必处理 cxx-qt 的多 target 链接问题）。
- **编辑器内的焦点是一次串行请求，不是一个 widget 句柄**：QML 是焦点的主人，Rust 只递增 `focus_serial`，桥把它发布成 `focusSerial`，QML 在变化时 `forceActiveFocus()`。不要为了「直接 focus」把 QML 控件塞进状态。
- **用户可见文案只来自 Fluent**，`en-US` 与 `zh-CN` 同步；`catalog_tests` 双向检查「代码提到的 id 都在目录里」与「目录里的 id 都被用到」，所以新增/删除文案必须两边一起改。
- **颜色、尺寸、间距、圆角只能来自 `inox_config::Theme`**。`view.rs` 与 `theme.rs` 里没有字面量颜色或像素尺寸；`placement.rs` 里的字面量只有无量纲比例与个数（表面尺寸是输出尺寸的份额，份额之内每一段的高度由令牌推出），`theme::text_size` 缩放的是 Qt Quick 在 96 dpi 下的名义正文尺寸。
- **它仍是普通窗口，不是 layer 表面。** 合成器尚未广告 `zwlr_layer_shell_v1`，`placement::Placement` 只是算好并写进日志的数据；启动时**恰好一条** `warn!` 说明这一点。不得声称摆放已生效，也不得为了「像 panel」把它变成 layer 表面——那是 panel/lock 的路径。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）。本 crate 只依赖 L1：`inox-common`（i18n 与 XDG 路径）、`inox-config`（`shell.toml` 的 `[launcher]` 与主题令牌）。**不得**新增 L2 或 L3 依赖。
- 令牌映射、Fluent 装配与 layer 引导在每个 L3 组件里各有一份，这是 L3 组件架构明确接受的成本；**不要**为此新建共享 crate 或同层依赖，那需要先有 ADR。

## 主要依赖

- 上游 Inox crate：`inox-common`（`i18n::Localizer`/`Locale`、`paths::data_home`）、`inox-config`（`LauncherConfig`、`Theme` 令牌）。
- 第三方（UI）：`cxx`、`cxx-qt`、`cxx-qt-lib`（bridge 与 `QString`/`QGuiApplication`/`QQmlApplicationEngine`），构建期 `cxx-qt-build` 注册 `com.inox.launcher.qml` 模块。
- 第三方（其余）：`serde` / `serde_json`（投影与动作通道）、`calloop`（启动时安装 SIGHUP 遮罩）、`zbus`（`org.inox.Launcher` 的 `/org/inox/Configuration` `Reload` 方法）、`thiserror`、`tracing`、`tracing-subscriber`、`anyhow`（仅 `main`）。
- 测试：`proptest`、`tempfile`；没有渲染/i 截图 harness（见「已知缺口」）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。`CARGO_HOME` 指向仓库内缓存，`--offline` 避免联网；本机还可用 `QT_QPA_PLATFORM=offscreen` 跑离屏 QML 加载测试（测试自身会给子进程设这个变量）。

| 目的 | 命令 |
|------|------|
| 格式化 | `cargo fmt -p inox-launcher` |
| 本 crate 测试 | `CARGO_HOME=<repo>/.cargo-home QT_QPA_PLATFORM=offscreen cargo test -p inox-launcher --all-features --offline` |
| 静态检查 | `CARGO_HOME=<repo>/.cargo-home cargo clippy -p inox-launcher --all-targets --all-features --offline -- -D warnings` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-launcher --no-deps --all-features` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`），**含拒绝与错误路径**，不只覆盖成功路径。
- 属性测试每项 ≥1000 样本（`ProptestConfig::with_cases(1000)`）：`Exec` 展开、搜索排序与代次、解析器健壮性。
- `catalog_tests`：`compare_catalogs` 的 id 与占位符一致性，加上代码 ↔ 目录的双向检查。
- `headless_tests`：投影断言（颜色 = 令牌转换、字号/间距 = 令牌推导、标题/提示来自 Fluent 且中英不同）、QML 动作经 `dispatch` 进入真实 `update`（打字、Enter 只启动一次、Esc、←/→ 步进一格、↑/↓ 步进 `columns` 格、指针选择、按住不重复）、以及**离屏 QML 加载**——测试进程以 `QT_QPA_PLATFORM=offscreen` 自 spawn 子进程，等 `reportReady` 确认根对象与语义树（QML 里 `Accessible.role` 的控件数量）后才算通过。这些取代了 Iced 的 `tests/render.rs` 像素断言。
- 测试代码不受「禁止 unwrap/expect/panic」约束。

## 已知缺口（诚实记录）

- **没有像素级证据**：Iced 的 `UserInterface`/tiny-skia 渲染装置已删除，也不再有 `INOX_SCREENSHOT_DIR` 的 PNG 工件；Q02 的截图证据需要 waydriver + 真实 Qt 平台，工作区尚未提供宿主。
- **语义树只被「存在性」断言**：QML 用 `Accessible.role/name` 暴露控件，`reportReady` 检查固定 chrome 的数量下限；离屏平台没有 AT-SPI 桥，screen reader 路径未被验证，Q01 不声称已验收。
- **就绪以进程存活为证据**（`activation.rs` 模块文档写明）：合成器没有窗口列表接口，多数应用也没有 D-Bus 名字。
- **重载桥没有自动化测试**：`reload.rs`（SIGHUP 遮罩 + `org.inox.Launcher` 的 `Reload`）本身只有 SIGHUP 安装会失败时返回错误这一条结构性保证；`main::reload_state` 的「过期快照 / 目录不可加载则保留旧配置」路径也没有测试驱动（需要会话总线或信号），改动这段代码要人工验证。
- **启动辅助的 `--exec-application` 模式**只被真实启动路径覆盖，没有单独的进程级测试。
- **打开器没有常驻宿主**：`open` 的每一步（类型判定、`mimeapps.list`、定位、argv、启动）都有单元测试，
  `cargo run -p inox-launcher -- --open <目标>` 就是它的真实二进制入口；但**没有**进程级 E2E，也**没有**
  `org.freedesktop.portal.OpenURI`（`inox-portal` 未创建）、没有 magic 内容嗅探（类型只由名字决定）、
  不验证就绪（`xdg-open` 的契约是启动后立刻返回）。通知里的链接仍未接线：`Outgoing::FollowLink` 仍被丢弃。

## 目录结构

```
crates/inox-launcher/
  Cargo.toml        工作区继承：字段一律 *.workspace = true
  build.rs          cxx-qt：注册 com.inox.launcher.qml 模块并编译 bridge
  qml/Main.qml      声明式 UI（Window 根）；只绑定 vm.*，不含字面量颜色/尺寸/文案
  src/main.rs       入口 + Qt 宿主 + QML 加载自检；模块声明（bin-only）
  src/cxxqt_object.rs  #[cxx_qt::bridge]：model_json / windowWidth / focusSerial / quitRequested + dispatch/pump/reportReady
  src/view.rs       状态 + 令牌 + Fluent → 一份 JSON 投影
  src/theme.rs      令牌 → QML 颜色（#AARRGGBB）与尺寸的纯函数
  src/runtime.rs    Queue / Task / oneshot（替代 iced::Task 与 Subscription）
  src/reload.rs     SIGHUP + org.inox.Launcher 重载桥（状态仍在 Qt 线程改）
  src/app.rs        State / Message / update（纯状态机）
  src/view_model.rs 状态 → 画面语义（纯投影：瓦片、分类 chips、横幅）
  src/icons.rs      Icon= → 图标文件（XDG 图标主题查找、file:// URL）
  src/categories.rs freedesktop Categories= → 固定分类 chips
  src/desktop_entry.rs, exec.rs, index.rs, search.rs, contract.rs,
  activation.rs, placement.rs, i18n.rs
  src/open/{mod.rs,mime.rs,mimeapps.rs}  打开器：类型判定、mimeapps.list、定位与启动（`--open` / `inox-open`）
  src/catalog_tests.rs, src/headless_tests.rs   #[cfg(test)] 证据
  i18n/{en-US,zh-CN}/launcher.ftl
  README.md         本 crate 说明
  AGENTS.md         本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `launcher`。
