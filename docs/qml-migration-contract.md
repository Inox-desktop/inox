# QML 迁移执行契约（feat/qml-all-components）

本文件是 Iced → QML + Qt Quick 全量迁移期间，各 crate 工作的**唯一实施契约**。根级 `AGENTS.md`、crate 级 `AGENTS.md` 与 QML 评估/0035/0036/0038 仍然有效；本文件只把「怎么做」写清楚，方便多个代理并行且不互相踩。

## 0. 分支与边界

- 分支：`feat/qml-all-components`（从 `feat/login-chain` 开出）。
- 每个代理**只**改自己负责的 `crates/inox-<component>/`；根 `Cargo.toml`、`Cargo.lock`、`deny.toml`、`.github/`、`docs/`、根 `AGENTS.md`、`PRD.md` 由 Lead 统一改。
- 迁移期间 Iced 与 QML 在不同 crate 共存；每个 crate 自己的 `Cargo.toml` 立刻去掉 `iced` / `iced_core` / `iced_runtime` / `iced_tiny_skia`，根级清理在收尾 PR。
- 每个 crate 必须能在本地通过（仅自检，不是验收）：

  ```sh
  CARGO_HOME=/home/rownix/Projects/Inox/.cargo-home cargo check -p inox-<component> --all-features --offline
  CARGO_HOME=/home/rownix/Projects/Inox/.cargo-home cargo clippy -p inox-<component> --all-targets --all-features --offline -- -D warnings
  CARGO_HOME=/home/rownix/Projects/Inox/.cargo-home QT_QPA_PLATFORM=offscreen cargo test -p inox-<component> --all-features --offline
  ```

  `CARGO_HOME` 指向仓库内缓存（宿主 `~/.cargo` 在沙箱里只读，不加会下载失败）；`--offline` 避免网络。第一次编译 Qt 绑定要几分钟，属正常。

## 1. 两种表面模式

| crate | 表面 | QML 宿主 |
|-------|------|----------|
| `inox-wallpaper`、`inox-launcher`、`inox-notifications`、`inox-settings`、`inox-greet` | 普通窗口（Iced 下本来也是） | `QGuiApplication` + `QQmlApplicationEngine`，QML 根是 `Window` |
| `inox-panel` | 真实 `zwlr_layer_surface_v1`（`src/layer.rs`） | **离屏** Qt Quick：QML 渲染成 RGBA 帧，写进现有 `wl_shm` |
| `inox-lock` | 真实 `ext_session_lock_v1`（`src/session.rs`） | 同上 |

panel/lock 的输入、放置、安全边界、`calloop` 循环**不动**；QML 只负责画。详见离屏 QML。

> **表面模式的后续方向（ADR 0046，进行中）。** panel 与 lock 的离屏宿主是**迁移期的过渡态**，
> 不是终点：离屏帧没有 `wl_surface.frame` 时间线、没有第二个表面、没有输入、没有 AT-SPI，
> 因此跨表面的过渡（面板长成菜单、菜单从按钮里钻出）与子菜单在架构上做不到。ADR 0046 的结论是
> 两者都改成 **Qt 的原生 wayland 表面**，并新增私有协议 `inox-shell-v1` 承载合成器侧的动效、
> popup 关闭权与工作区事件；锁屏保留 `ext-session-lock-v1`，只换渲染器。
> 在 P3/P4 落地之前，上表仍是实现的真实情况；本文件的其余条款（尤其 §6 安全边界）继续按现状执行。

## 2. crate 结构（统一形态）

```text
crates/inox-<component>/
  Cargo.toml            # 去 iced，加 cxx / cxx-qt / cxx-qt-lib / cxx-qt-build
  build.rs              # CxxQtBuilder::new_qml_module(...)
  qml/Main.qml          # 声明式 UI（panel/lock 是离屏窗口根）
  src/main.rs           # 入口：Qt 宿主（或离屏宿主）+ calloop/事件泵
  src/cxxqt_object.rs   # #[cxx_qt::bridge]，唯一 QML↔Rust 边界
  src/view.rs           # Qt-free、可 serde 的 UI 投影（状态 → JSON 值）
  src/theme.rs          # 令牌 → QML 颜色/尺寸（不再有 iced::Theme/Style）
  src/runtime.rs        # 本 crate 的 Task/Subscription 替代（见 §5）
  src/app.rs / state / view_model / ...  # 领域逻辑，尽量少改
  src/headless_tests.rs # bin 内 cfg(test)：投影 + QML 加载 + 逻辑
```

### 2.1 必须 bin-only

cxx-qt 0.10 把 bridge 生成的 `cxx_qt_init_*` 定义嵌入「声明 bridge 的目标」，静态库里没有目标引用时会被 `--gc-sections` 丢掉，导致 `undefined reference`（KDAB/cxx-qt#770）。因此：

- 删除 `src/lib.rs`，所有模块挂到 `main.rs`（`mod app; mod ...;`）。
- 把 `tests/*.rs` 的内容迁到 bin 内的 `#[cfg(test)]` 模块；需要驱动二进制的测试在测试里 `std::env::current_exe()` 自 spawn（参考 `feat/greet-qml` 的 `src/headless_tests.rs`）。
- 同一 package 里若还有第二个 bin（`inox-settings` 的 `inoxctl`），也必须处理链接：让它 `mod` 引入 bridge 并至少引用一次（或把 bridge 的 `cxx_qt_init` 符号显式引用），否则该 target 链接失败。优先方案：把 `inoxctl` 的入口保持为独立 `[[bin]]`，并在其中 `#[allow(unused_imports)] use crate::cxxqt_object as _;` 只能解决同 crate 引用——实际以 `cargo build -p inox-settings --bins` 通过为准。
- Cargo.toml 的 `[lints.rust]` 必须 `unsafe_code = "allow"` + `missing_docs = "warn"`；Inox 手写 Rust 仍然不写 `unsafe`。

### 2.2 build.rs（窗口组件）

```rust
fn main() {
    cxx_qt_build::CxxQtBuilder::new_qml_module(
        cxx_qt_build::QmlModule::new("com.inox.<component>.qml").qml_file("qml/Main.qml"),
    )
    .files(["src/cxxqt_object.rs"])
    .qt_module("Quick")
    .build();
}
```

QML 资源 URL：`qrc:/qt/qml/com/inox/<component>/qml/qml/Main.qml`（模块 URI 的点换目录 + 文件自身的相对路径，参考 `inox-qml-prototype` 与 `feat/greet-qml`）。

panel/lock 另外 `.cpp_file("src/offscreen.cpp").include_dir("src")`，并保留本 crate 的 QML 模块。

## 3. 桥（cxxqt_object.rs）的数据形状

**不要为每个字段写一个 `#[qproperty]`。** 大组件的视图模型有几十上百个字段，逐字段桥会把 QML↔Rust 边界变成第二份 UI 定义。统一用「一份 JSON 模型 + 窄动作通道」：

```rust
#[qproperty(QString, model_json)]   // 只读投影，QML 里 JSON.parse
#[qproperty(bool, quit_requested)]  // 需要时再加少量控制位
```

- Rust 侧：`view.rs` 把「当前状态 + 令牌 + Fluent 文案」投影成 `serde_json::Value`（或 `#[derive(Serialize)]` 结构），序列化后塞进 `model_json`。**每次状态变化都整体重发**；QML 里 `property var vm: JSON.parse(app.modelJson)`。这是本迁移接受的取舍：先保证正确与边界干净，性能问题在 P03 有数据后再做增量。
- QML → Rust：一个通用 invokable，JSON 字符串：

  ```rust
  #[qinvokable]
  #[cxx_name = "dispatch"]
  fn dispatch(self: Pin<&mut Self>, action: &QString);
  ```

  动作形如 `{"type":"activate","id":7}` / `{"type":"set-query","value":"foo"}`。Rust 端用 `serde_json` 解析成 crate 内的 `UiAction` 枚举，再转成既有 `Message`；解析失败只 `warn!`，不 panic。
- 定时器：QML 侧 `Timer { interval: 16; running: true; repeat: true; onTriggered: app.pump() }`。`pump()` 排空后台线程送来的 `Message` 队列、跑 `update`、必要时刷新 `model_json`，并返回/暴露 `needs_repaint`（离屏宿主用）。panel/lock 不用 QML Timer，由 `calloop` 的 timer 调 `pump()`。
- 文本输入 / 按键：QML `Keys.onPressed` 调 `app.key(json)`；**密码永远不进入 QML 属性**（见 §6）。文本框的显示值由 Rust 投影（锁屏只给掩码长度/掩码串）。

## 4. 颜色、尺寸、文案

- QML 里**禁止**出现颜色字面量、像素字面量、用户可见英文字符串。所有值来自 `vm`（Rust 投影）。
- `Rgba` → QML 颜色用 Qt 的 `#AARRGGBB` 字符串（`QColor::name(QColor::HexArgb)` 的格式）；在 `theme.rs` 写一个 `fn qml_color(Rgba) -> String` 并单测。
- 尺寸/圆角/间距从 `inox_config::Theme` 的 spacing/typography/radius 与用户字体缩放推导；推导规则保留原 `theme.rs` 的注释与常量，改写成纯函数。
- Fluent：所有用户可见文案仍由 `Localizer` 在 Rust 解析，进 JSON；QML 不拼文案，只拼接 Rust 给的完整句子/带占位符的格式化结果。
- 字体：字体家族/字重/字号从令牌与 `typography` 投影，QML 只绑定。

## 5. 去掉 iced 运行时

- `app.rs` 的 `Message` / `update` / `State` 纯逻辑与单测**保留**；删除 `iced::application`、`view()`、`iced::Element` 相关代码。
- `iced::Task`：本 crate 写 `src/runtime.rs` 提供最小 `Task<T>`（`none` / `done` / `perform` 的同步或线程版 / `batch` / `map`），把 `Task::perform(async {...})` 改成返回 `Message` 的闭包 + 线程/队列。不要为了保留 async 引入新的异步运行时；`zbus` 的 `block_on` 用法按原样保留。
- `iced::Subscription`：改成显式来源——`calloop` timer / 后台线程 + `std::sync::mpsc` / QML `Timer`；由宿主在启动时注册，消息进 §3 的队列。不要保留「订阅声明 + 框架解释」的模型。
- `iced::keyboard`：QML 发按键（字符串键名 + 修饰键位），Rust 映射成既有 `Message`；或者面板/lock 继续用现有协议输入。不要把 iced 的键盘类型留在 app.rs。
- 删除 `src/render.rs` / `tests/render.rs` / `tests/support` 中依赖 iced 的渲染装置；对应的断言改为「投影可测值」或「QML 模块能加载」。

## 6. 安全边界（lock / greet 必须遵守）

- 凭据只存在于 Rust 的 `Secret`（`zeroize`）中；QML 只拿到掩码（长度/掩码串），**不得**把明文密码放进任何 `#[qproperty]`、`model_json`、QML 变量、`TextField`。
- 焦点/键盘：QML 只上报按键与 Tab/Esc 语义，焦点顺序仍在 Rust 的 view model；QML 用 `Accessible.role/name` 暴露语义树。
- 锁表面：`inox-lock` 仍必须由 `src/session.rs` 创建 `ext_session_lock_v1` 表面；QML 只渲染。面板同理保留 `src/layer.rs`。

## 7. 测试要求（每个 crate）

必须保留/新增：

1. **纯逻辑单测**：原有 app/view_model/state 测试全部保留并通过。
2. **投影测试**：`view.rs` 的 JSON 值断言——颜色等于令牌转换结果、字号/间距等于令牌推导值、文案来自 Fluent（中英各一条）。
3. **QML 加载测试**：在 `QT_QPA_PLATFORM=offscreen` 下用 `QQmlApplicationEngine` 加载本 crate 的 QML 模块，断言根对象非空 + QML 回报 `ready`（参考 greet 的 `reportReady`/`QML_READY`）。测试进程环境用「父测试 spawn 自身子进程」的方式设置，不能在本进程 `set_var`（edition 2024 下是 unsafe，禁止）。
4. **协议测试**：panel/lock 的 `tests/protocol.rs` 必须继续以真实 socket pair / 子进程驱动并断言 layer/lock 表面行为；迁进 bin 内的 `#[cfg(test)]` 后仍要跑。
5. 不 mock 核心逻辑；只打桩系统边界。

## 8. 文档

每个 crate 更新 `AGENTS.md` 与 `README.md`：职责、cxx-qt/`unsafe` 例外、QML 结构、测试命令、表面模式（窗口/离屏）、已知缺口。**不要**改根文档（Lead 统一）。

## 9. 完成定义（DoD）

- `cargo check/clippy/test` 三条本地命令（§0）通过；`cargo fmt` 干净。
- `rg -n "iced" crates/inox-<component>` 只剩历史注释或零命中。
- QML 里 `rg -n '#[0-9a-fA-F]{3,8}|[0-9]+px' qml/` 无颜色/尺寸字面量（除 `0`/`1` 这类结构性数值）。
- `git status` 只包含本 crate 的文件（外加 Lead 同意共享的文件）。
- 在回报里写清：改了什么、哪些测试从渲染改成投影、已知缺口、本地命令结果。

## 10. 参考实现

- `git show feat/greet-qml:crates/inox-greet/src/{main.rs,cxxqt_object.rs,view.rs,headless_tests.rs}` 与 `.../qml/main.qml`：窗口 + cxx-qt + JSON/属性投影 + headless 自检的完整样本。
- `crates/inox-qml-prototype`：最小 bridge/QML 模块/headless 测试。
- panel/lock 的离屏 C++ 宿主参考 `.probe-qtbridge/`（探针，不提交）：`src/offscreen.h` / `src/offscreen.cpp` 的形态可复制到 crate 内，改成 QML 模块 URL 与尺寸参数。

## 11. 收尾状态（2026-10-24）

迁移已进入收尾：七个组件全部改为 QML + Qt Quick（cxx-qt 绑定），根 `Cargo.toml` 不再声明
`iced` / `iced_layershell` / `iced_core` / `iced_runtime` / `iced_tiny_skia`，`Cargo.lock` 中不再有
`iced*` 包。收尾的判据、私有 ABI 与私有协议的授权边界、以及每条验收 ID 记录在
。

- §1–§10 仍是本迁移的实施契约；收尾不改变任何一条实现约束。
- 新增授权：允许在**局部**使用 Qt 私有 ABI 与 Inox 私有合成器协议，前提是产物保持原生
  Qt Quick、私有 ABI 的范围在 crate 级 `AGENTS.md` 记录、GPL-only Qt 模块仍然禁止。
- 已知缺口（不得在文档里声称已解决）：panel/lock 的离屏表面没有 AT-SPI 语义桥；真实合成器宿主上的
  waydriver 截图断言仍未接线；QML 侧每次状态变化整体重发 JSON 投影。
- **表面模式不是收尾的一部分（2026-10-29 追加）**：ADR 0046 把 panel/lock 的终点从「离屏 QML 帧 +
  Rust 持有的表面」改为「Qt 原生 wayland 表面 + 私有协议 `inox-shell-v1`」，并说明了为什么离屏路径
  的上限是架构性的（没有 frame 时间线、没有第二个表面、没有输入、没有 AT-SPI）。P0 只落了协议
  （`crates/inox-protocols/protocol/inox-shell-v1.xml`、`src/shell.rs`、`tests/shell_roundtrip.rs`），
  合成器与两个组件都还没变；在 P3/P4 之前，§1 的表格仍是实现的真实情况，「QML 不接输入」也仍然
  生效。§1 的离屏条目会在 P5 删除，届时本节的这条也一并删除。
