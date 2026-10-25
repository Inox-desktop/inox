# inox-settings

Inox 的系统设置应用，属于 **L3**，也是唯一写入 Inox 配置的组件。
普通 `xdg-shell` 窗口：**QML + Qt Quick**（经 cxx-qt 绑定）渲染，状态、草稿、持久化与文案都在 Rust，颜色与尺寸只来自 `inox_config::Theme` 令牌（ADR 0033、0035）。

## 设置范围

| 页面 | 可编辑项目 |
|------|------------|
| 外观 | 浅色、深色、跟随会话 |
| 面板 | 位置、高度、自动隐藏、预留空间、边距、时钟及格式、任务按钮上限、图标大小 |
| 通知 | 位置、同时显示数量、宽度、边距、图标、各紧急度超时、勿扰、操作按钮、锁屏隐私、队列上限及满队列策略 |
| 语言 | 跟随环境、英文、中文；保留外部配置中的其他合法语言标签 |
| 键盘与鼠标 | XKB 布局与变体、重复延迟/速率、轻触点击、自然滚动 |
| 窗口布局 | 主区域比例、主窗口数量、窗口间距 |
| 启动器 | 结果数量、图标大小、隐藏应用、查询长度、终端程序与逐项参数 |
| 壁纸 | 内置壁纸选择、主题背景、填充/适应/拉伸/居中/平铺、按显示接口覆盖图片 |
| 显示器 | 输出条目增删、接口、分辨率、刷新率、分数缩放、负坐标及自动排列 |
| 输入法 | 引擎程序与逐项参数；空列表关闭下次会话的自动启动 |

壁纸支持 PNG/JPEG，图片由壁纸组件读取。这里直接挑选 Inox 内置的壁纸（清单来自 L1 的
`inox_config::wallpapers`，可选「默认」与「主题背景」），也可以保留外部配置里手写的自定义路径并逐字写回；
设置应用不安装或下载图片（[ADR 0030](../../docs/adr/0030-bundled-wallpapers-and-the-default-background.md)）。
输入法示例：第一项 `/usr/bin/fcitx5`，之后每项一个参数；不经 shell 解释。
物理合成器会读取保存的 `input.input_method`，显式 `--input-method` 优先；
确定性 headless 后端仍只使用显式启动参数。词库、语言方案和切换快捷键由引擎自己的配置工具管理。

## 界面结构（QML + cxx-qt）

```
main.rs            QGuiApplication + QQmlApplicationEngine，加载 qrc:/qt/qml/com/inox/settings/qml/qml/Main.qml
cxxqt_object.rs    唯一的 QML↔Rust 边界：SettingsApp { modelJson, ready, pump(), dispatch(json), reportReady(...), quit() }
ui_host.rs         Qt-free 的 Host 契约与线程局部安装；bridge 只依赖它
runtime.rs         UiRuntime：状态机、后台工作线程、pump（排空消息 + 试用倒计时）
view.rs            投影：状态 + 令牌 → 一份 JSON；QML 发回的动作是一组闭合的 UiAction
theme.rs           令牌 → #AARRGGBB 颜色与全部几何值（纯函数）
qml/               11 个文件：Main、SettingsSurface、FieldRow、TextFieldControl、NumberControl、
                   ChoiceRow、FlagControl、CollectionControl、ControlButton、StatusStrip、TrialDialog
```

- **一份 JSON 模型，整份重发。** 每次状态变化，Rust 把「当前状态 + 令牌 + Fluent 文案」重新投影成 JSON 塞进 `modelJson`，QML 里 `JSON.parse` 后绑定。不为每个字段加属性，QML 里也没有第二份界面定义。
- **QML 不做决定。** 页面、卡片、控件形态、值、禁用原因、焦点位置、状态行句子、试用弹窗全部由 Rust 决定；QML 里没有颜色字面量、没有像素字面量、没有用户可见字符串。
- **单向动作通道。** 点击与按键序列化成 `{"type": ...}` 交给 `dispatch`，由 `parse_action` 解析成 `UiAction` 再变成 `Message`；不认识的动作用 `warn!` 记录后丢弃，不会让窗口崩。
- **pump。** QML 的 16 ms 定时器调用 `pump()`，它排空后台线程送回来的消息（写盘结果、D-Bus 回执）、推进显示试用倒计时，并在投影变化时重发 `modelJson`。
- **键盘。** QML 只上报 Tab / Enter / Esc / 上下方向键；焦点顺序、禁用项是否可达、要滚动哪个容器都由 Rust 投影。`Keys` 挂在铺满窗口的 `Item` 上（挂在 `Window` 上会被 Qt 静默丢弃）。
- **窗口是普通窗口**，不是 layer-shell 客户端；不使用 Qt Wayland Compositor（GPL-only），也不引入 webview。
- 同一 package 还有第二个 bin `inoxctl`，它用 `#[path]` 直接编译 `cxxqt_object.rs` 以满足 cxx-qt 的链接要求（`cargo build --bins` 是这条的检查）；细节见 `AGENTS.md`。

## 保存与生效

编辑采用草稿；应用前检查字段范围、时钟格式、结构化列表以及完整文件能否被真实配置解析器读取。
数值可直接输入，未完成或溢出的输入会留在界面并阻止保存。取消不写文件，恢复默认使用 schema 默认值。

保存经过 `persist::save`：目标目录内临时文件、完整写入、同步、原子替换、目录同步；
以读取时的内容摘要检测外部编辑，冲突时保留草稿并提供重读。
只写改过的键；集合使用 TOML 文档编辑器替换该集合，保留其他配置。
多文件保存可能部分失败；成功文件更新基线，失败字段保留为改动，其他在途保存完成前禁止再次编辑。

保存完成后，通过会话 D-Bus 请求受影响的消费方热重载，并核对代次和完整语义快照。
面板改动只联系面板，壁纸改动只联系壁纸；主题与语言联系所有消费方。
消费方实际更新状态后才显示「已生效」；缺失或超时显示「已保存、未确认生效」，拒绝显示具体原因。
设置窗口的主题和语言也随当前代次的有效回执更新。组件的窗口、查询、任务和通知不会整体重建。
组件拒绝或暂时不可用时，可再次点击「应用」重试已保存的偏好，不重复写盘；全部消费方结算前保持在途状态。

支持运行中更新主题/语言、键盘布局与重复参数、设备支持的触摸板选项、平铺布局、
面板内容与高度、启动器索引/搜索/终端偏好、通知队列/策略以及壁纸预览。
壁纸先完成读取和解码，失败保留旧帧。
**显示页是唯一带确认的热应用**：模式、缩放与位置由合成器以「试用」应用，窗口弹出确认框
并倒计时 30 秒；点「确认修改」保留，点「恢复原设置」或倒计时结束则合成器恢复试用前的
输出偏好、窗口把 `compositor.toml` 里的 `outputs` 改回原值（只改这一个键）。
期限由合成器持有，窗口消失也会回退；合成器不可达时不弹框，只显示「已保存、未生效」。
headless 后端的输出来自 `--output`，该路径仍拒绝运行时改动。
输入法引擎替换和进程环境尚不支持热应用，会明确拒绝并保留已保存偏好。
面板边缘/独占区/自动隐藏/边距与通知桌面角落需要 layer-shell 宿主，当前窗口模式会拒绝这些实时改动。
壁纸回执只证明当前预览窗口，不能代替真实桌面背景层验收；显示试用也一样，
真实显示器上的模式切换与回退属于 E4 真机验收，不由离屏渲染推断。

手改配置后运行 `inoxctl reload`，或向具体消费方发送 `SIGHUP`。
`inoxctl` 会报告每个运行中组件的结果，任何拒绝或未确认均非零退出。
合成器现有的 Wayland `reload_config` 也接入相同的准备与应用入口。
接口、生命周期及限制见 [ADR 0026](../../docs/adr/0026-live-configuration-reload.md)；
显示试用与回退见 [ADR 0028](../../docs/adr/0028-display-trial-confirm-and-rollback.md)
与 [`configuration-v2.md`](../inox-protocols/protocol/configuration-v2.md)。
自定义合成器快捷键仍缺少 schema 与消费者，本次不创建无效配置。
网络、蓝牙、音频、账户管理不属于当前 Inox 配置模型。

## 验证与开发

所有控件文案有 en-US / zh-CN 目录；长页面滚动，键盘焦点按真实布局自动显示。
测试全部跑在二进制内（crate 是 bin-only，`tests/` 会破坏 cxx-qt 的链接，见 `AGENTS.md`）：

| 证据 | 位置 |
|------|------|
| 真实文件上的保存/回读、取消、冲突、写盘失败、重启、重载、显示试用三条路径、≥1000 例属性测试 | `src/headless_tests/workflow.rs` |
| 每个可编辑字段的边界与 TOML 往返 | `src/headless_tests/complete_settings.rs` |
| en-US / zh-CN 的 id 与占位符一致（Q03） | `src/headless_tests/catalog.rs` |
| **投影断言**（颜色都是令牌、禁用原因、焦点序列、试用弹窗、SavedOnly ≠ Applied、字号加倍） | `src/headless_tests/projection.rs` |
| **离屏加载**：真实 `qrc:` QML 模块载入、QML 树回报的就绪与投影一致、没有 QML 错误 | `src/headless_tests/qml.rs` |

```bash
cargo check  -p inox-settings --all-targets --all-features --offline
cargo clippy -p inox-settings --all-targets --all-features --offline -- -D warnings
QT_QPA_PLATFORM=offscreen cargo test -p inox-settings --all-features --offline
cargo build  -p inox-settings --bins --offline   # cxx-qt 的链接陷阱，必须单独过
cargo fmt    -p inox-settings
```

验收以 GitHub Actions 为准，不在本地把构建或测试结果当作验收。离屏加载是 E1 证据：它证明模块能载入、树读到了投影，
**不等于**真实会话里的像素、鼠标交互或屏幕阅读器验收——这些仍是缺口，记录在 `AGENTS.md` 的「已知缺口」里。

架构依据：[ADR 0014](../../docs/adr/0014-configuration-apply-and-migration.md)、
[ADR 0018](../../docs/adr/0018-l3-component-architecture.md)、
[ADR 0024](../../docs/adr/0024-settings-coverage-and-input-method-preferences.md)、
[ADR 0033](../../docs/adr/0033-qml-qt-quick-shell-evaluation.md)、
[ADR 0035](../../docs/adr/0035-cxx-qt-binding.md)。
开发遵循根级与本 crate 的 `AGENTS.md`，提交 scope 为 `settings`。

LGPL-3.0，见仓库根的 `LICENSE`。
