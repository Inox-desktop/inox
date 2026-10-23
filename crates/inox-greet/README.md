# inox-greet

L4 可选登录组件：登录生命周期模型 + **QML/Qt Quick 预览**（ADR 0036 D03，自 Iced 迁移的首个完整样本）。

`inox-greet --preview` 展示一个普通窗口：标题、说明、用户名输入、禁用的登录按钮与清空按钮。所有颜色、间距、圆角与字号来自 `inox_config::Theme` 设计令牌，所有用户可见文案来自 Fluent 目录（`en-US` / `zh-CN`）——QML 内没有硬编码颜色、尺寸或字符串（根级 AGENTS.md §4、§7.3）。令牌与文案在安全 Rust 侧投影（`src/view.rs`），经 cxx-qt bridge（`src/cxxqt_object.rs`）以 `QColor` / `int` / `QString` 属性暴露给 QML。

预览**不会收集密码、不会启动会话**：`Greeter` 状态机以「认证边界不可用」构造，登录按钮始终禁用，QML 只做渲染与事件上报（ADR 0015、ADR 0036 §失败、恢复与安全）。

默认 `inox-greet` **拒绝真实登录并返回失败**（非零退出 + 可读错误）：ADR 0015 要求的认证服务、helper 权限/许可证、可信通道及凭据清理尚待安全审查，不能用模型测试替代系统认证。模型提供当前代次关联、30 秒超时、滚动 60 秒最多 3 次尝试、取消清理及启动回执；没有接入 PAM、greetd 或其他认证实现。

## 结构（bin-only）

本 crate **只有二进制目标**。cxx-qt 0.10 把 bridge 生成的 C++ 初始化器链接进「声明该 bridge 的目标」，若把 bridge 放在库里由二进制引用，二进制对象图不引用这些定义，`--gc-sections` 会丢弃它们并导致 `undefined reference to 'cxx_qt_init_*'`（[KDAB/cxx-qt #770](https://github.com/KDAB/cxx-qt/issues/770)）。同理，**集成测试（`tests/`）也无法链接**该 C++ 归档，所以全部测试都放在 bin 内的 `#[cfg(test)]` 模块中。

```
crates/inox-greet/
  build.rs              # CxxQtBuilder + QmlModule("com.inox.greet.qml") + .files(["src/cxxqt_object.rs"])
  qml/main.qml          # 只做渲染；import com.inox.greet.qml
  src/main.rs           # bin 入口：参数、配置、Qt 启动、--self-check
  src/view.rs           # 令牌 + Fluent → QML 可读投影（Qt-free，可单测）
  src/app.rs            # QML 动作 → Greeter 事件（安全 Rust）
  src/cxxqt_object.rs   # #[cxx_qt::bridge]：GreetPreview 属性与 invokable
  src/state.rs          # 登录状态机（语义未改）
  src/i18n.rs           # 双语言目录
  src/headless_tests.rs # bin 内测试（投影、文案、进程级拒绝、offscreen QML 加载）
```

## 测试与自检

```sh
# 全部测试（含 offscreen QML 加载；测试自己以子进程方式设置 QT_QPA_PLATFORM）
cargo test -p inox-greet

# 手动跑无头自检：加载 QML 模块、泵事件循环、断言 root object 已创建
QT_QPA_PLATFORM=offscreen cargo run -p inox-greet -- --preview --self-check
```

`--self-check` 是**诊断开关**：它加载预览、等待 Qt 真正实例化 root object，然后退出（成功 0 / 失败非零）。它不改变「无 `--preview` 即拒绝登录」的默认语义，也不能用来登录。

注意 `QQmlApplicationEngine::load` 是异步的——只调用 `load` 不泵事件循环，测试会「假通过」。自检通过 `objectCreated` 信号判定 root object 是否创建，并要求 QML 树回报它确实读到了 Rust 侧投影（`reportReady`）。

QML 资源 URL 是 `qrc:/qt/qml/<模块 URI 点转斜杠>/<文件相对路径>`：模块 `com.inox.greet.qml` + `qml/main.qml` ⇒ `qrc:/qt/qml/com/inox/greet/qml/qml/main.qml`（注意重复的 `qml` 段）。

详见 [ADR 0021](../../docs/adr/0021-l4-session-services.md)、[ADR 0035](../../docs/adr/0035-cxx-qt-binding.md)、[ADR 0036](../../docs/adr/0036-component-migration-plan.md)。真实 greeter 宿主、waydriver 截图与 AT-SPI 桥仍待实现；QML 已为各控件设置 `Accessible.role` / `Accessible.name` 并保持 Tab/Enter/Space/Escape 导航。本地仅 fmt/check/clippy，验收以 CI 为准。

许可证 LGPL-3.0，见根 LICENSE。
