# 组件清单

Inox 采用**单一仓库（Cargo workspace）**：全部组件都是本仓库 `crates/` 下的工作区成员，彼此通过 path 依赖连接，版本由 `[workspace.package]` 统一（见[依赖管理](../development/dependencies.md)）。决策见[架构决策记录](./decisions.md)。组件清单以 PRD §3.1 为准。

**层级成员的唯一事实来源**是根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`，并由 `cargo test -p inox-integration --test layering` 断言（验收项 B04）——本页下表与它不一致时以它为准。

## 状态说明

本页的「当前状态」按以下口径标注，均可在 `crates/<name>/` 中核对：

| 标注 | 含义 |
|------|------|
| **已实现** | 该 crate 中已有可用实现，且有对应测试与 CI |
| **骨架** | 只有 crate 结构、CI 与文档，`src/` 中尚无业务逻辑 |
| **计划中** | 尚未创建，仅在 PRD 中规划 |

「已实现」说的是**代码与测试**已落地，不是真机已验证：登录链（`inox-auth`、`inox-host`、`inox-greet`）
的真机证据是 E4，**尚未取得**，逐条见[验收规范](https://github.com/Inox-desktop/inox/blob/main/docs/quality/acceptance.md)。

## 全部组件

| crate | 层级 | 路径 | 职责 | 发布物 | 当前状态 |
|-------|------|------|------|--------|----------|
| `inox-common` | L1 | `crates/inox-common` | 共享类型、错误处理、日志、工具函数 | crate | 已实现（`logging`、`paths`） |
| `inox-config` | L1 | `crates/inox-config` | 配置模型、TOML 解析、默认配置 | crate | 已实现（模型、解析、错误定位；决策状态仍为 `proposed`） |
| `inox-protocols` | L2 | `crates/inox-protocols` | 标准协议绑定与自定义 Wayland 协议扩展 | crate | 已实现（标准 `wl_*` 再导出 + 私有 `inox-ipc` 绑定 + `inox-glass-v1` v2（模糊 + 圆角折射/色散）的绑定与两个版本的协议往返；效果由合成器在 headless 路径实现，DRM 路径报 0） |
| `inox-compositor` | L2 | `crates/inox-compositor` | 基于 Smithay 的 Wayland 合成器（广告 `wl_output`/`wl_compositor`、M3 的 `wp_viewporter`/`wl_data_device_manager`/主选区、私有 `inox_ipc` 与 `inox-glass-v1` v2） | 二进制 `inox-compositor` | 已实现（M1、M2、M3 的协议面与数据互通；layer-shell v4 宿主已接入；headless 路径的玻璃模糊与圆角折射 pass 有像素证据；单输出真实后端已实现，E4 尚未执行；DRM 路径不实现玻璃并诚实报 0） |
| `inox-xwayland` | L2 | `crates/inox-xwayland` | XWayland 集成（监督 xwayland-satellite 进程） | 二进制/服务 | 已实现（进程监督、真实 X11 连接往返的就绪判定、生命周期） |
| `inox-shell` | L3 | `crates/inox-shell` | Shell 主框架与插件系统 | 二进制 `inox-shell` | 部分实现（跨入口导航、服务契约、内部入口注册表；不绘制界面） |
| `inox-panel` | L3 | `crates/inox-panel` | 面板（任务栏、菜单栏、Dock、系统托盘、时钟） | 二进制 `inox-panel` | 部分实现（任务/托盘/时钟模型；界面为 QML，经**离屏渲染**写入自己创建的 `zwlr_layer_shell_v1` 表面；菜单栏与 Dock 的窗格由 `inox-glass` 的 `GlassPane` 绘制，并绑定 `inox-glass-v1` v2，headless 路径上有模糊与折射；窗口服务与 SNI 传输尚未实现） |
| `inox-launcher` | L3 | `crates/inox-launcher` | 应用启动器 | 二进制 `inox-launcher` | 部分实现（XDG 条目解析、Exec 展开、带代次的搜索；界面为 QML **普通窗口**，`placement.rs` 的层表面策略只建模、未应用） |
| `inox-notifications` | L3 | `crates/inox-notifications` | 通知守护进程 | 二进制 `inox-notifications` | 部分实现（协议契约、有界队列与策略、文本脱敏；`org.freedesktop.Notifications` 服务由 `zbus` 提供；界面为 QML **普通窗口**，通知卡片是 `inox-glass` 的 `GlassPane`（ADR 0044），但普通窗口拿不到模糊，真实会话里表现为纯半透明） |
| `inox-settings` | L3 | `crates/inox-settings` | 系统设置应用 | 二进制 `inox-settings`（另有 `inoxctl`） | 部分实现（草稿/校验/原子保存/回读状态机；界面为 QML **普通窗口**，无需 layer-shell） |
| `inox-lock` | L3 | `crates/inox-lock` | 锁屏（`ext-session-lock-v1`） | 二进制 `inox-lock` | 部分实现（锁状态机、凭据清理、挂起时序；协议腿、**锁表面界面**、`auth-v1` 客户端与解锁路径、锁表面键盘输入均已落地——QML 离屏渲染成 configure 尺寸的帧并提交，认证成功后才发送唯一的 `unlock_and_destroy`；运行期触发已有 `inox-lock --request-lock` → `session-control-v1`；指针/触摸、IME 与按键重复、挂起传输、空闲链路与快捷键仍未接线，见 README 的已知限制） |
| `inox-wallpaper` | L3 | `crates/inox-wallpaper` | 壁纸设置与渲染 | 二进制 `inox-wallpaper` | 部分实现（来源解析、缩放几何、原子应用与预览窗口；界面为 QML **普通窗口**，背景层策略只建模、未应用） |
| `inox-qml-prototype` | L3 | `crates/inox-qml-prototype` | QML + Qt Quick + cxx-qt 的最小原型 | 二进制 | 已实现（最小 bridge、QML 模块与 headless 加载测试；占位原型，不是产品组件） |
| `inox-glass` | L3 | `crates/inox-glass` | 液态玻璃材质决策（clamp / 降级 / v1-v2 协商）与共享玻璃 QML 组件库 | crate（无二进制、无 Qt 依赖） | 已实现（`inox-glass-v1` v2 的材质 API、`GlassTarget` 适配器、`qml/` 五个无状态组件与 `CatalogWindow` 目录页；纯 Rust 逻辑有单元/proptest/漂移测试；两个消费者都编它的 QML 资产（面板运行期还用它的材质 API），面板画窗格、通知画卡片；组件像素证据由 `inox-panel` 的离屏测试提供；同层例外 `inox-panel -> inox-glass`（ADR 0040）与 `inox-notifications -> inox-glass`（ADR 0044）） |
| `inox-session` | L4 | `crates/inox-session` | 会话管理器，拉起合成器与 Shell | 二进制 `inox-session` | 部分实现：Wayland 往返就绪、有界进程监督/注销；完整服务 ready 待实现 |
| `inox-idle` | L4 | `crates/inox-idle` | 空闲管理 | 二进制 `inox-idle` | 部分实现：标准空闲通知客户端与锁前挂起策略；可信锁/挂起通道待审查 |
| `inox-auth` | L2 | `crates/inox-auth` | 认证边界（PAM + `auth-v1` + VT 分配/交回） | 二进制 `inox-auth` | 已实现：系统 PAM（安全封装，无新增 `unsafe`）、Unix 套接字服务与 `SO_PEERCRED` 对端校验、滚动限额；登录链的会话负责人（`src/leader.rs`：一次登录一个进程，`pam_open_session` 只在它里面调用）与 logind VT 分配/交回（`src/logind.rs`）已落地，一次只允许一个用户会话；口令与打开会话分在**两个栈**上（`--service inox-auth` 校验口令、`--session-service inox-auth-session` 打开会话，见 `packaging/`）；契约文本在 `inox-protocols/protocol/auth-v1.md`；真机 PAM/VT 行为是 E4，尚未取得 |
| `inox-greet` | L4 | `crates/inox-greet` | 显示管理器（可选） | 二进制 `inox-greet` | 已接线：默认路径要求认证边界套接字（否则开窗前拒绝），口令字段以掩码呈现、凭据只经 `Secret` 移交（明文不进 QML），认证成功后由边界启动会话；界面为 QML **普通窗口**；身份来自系统账户库（`getent passwd`），界面不提供手工输入账户名；会话启动后**驻留并在 3 秒后回到登录画面**（`state::SESSION_STARTED_DWELL`），因此注销回来时屏幕上已经是登录界面；`--preview` 仍是「没有边界时的界面」；真机验收是 E4，尚未取得 |
| `inox-host` | L4 | `crates/inox-host` | greeter 宿主：在裸 VT 上按顺序拉起登录界面的合成器与登录界面，监督并在退出时清理 | 二进制 `inox-host` | 已实现：`supervisor`（边界不可达拒绝、合成器就绪等待 10 s、登录界面 60 s 内最多 3 次有界重启、SIGTERM 逆序清理）与 `cli`/`process`/`runtime` 适配，`tests/supervisor.rs` 用真进程（假合成器与假界面）驱动；它不认证、不碰凭据、不启动用户会话。真机 VT 交接与登录是 E4，尚未取得 |
| `inox-integration` | 测试 | `crates/inox-integration` | 端到端集成测试与分层方向检查（B04） | 测试报告 | 部分实现（分层检查已落地；M3 的 XWayland/剪贴板/拖放 E3 用例以进程方式驱动工作区二进制，完整会话 E2E 待 M6） |

工作区之外还有三处非 crate 位置：

- **本站点**位于 `docs/book/`（mdBook），无 `Cargo.toml`，不参与依赖图与 `cargo deny`；
- 组织仓库 `.github` 承载组织级工作流模板与 Issue/PR 模板；
- **安装面**位于 `packaging/`（PAM 栈、systemd 单元、安装/试运行/回退脚本），
  同样没有 `Cargo.toml`，不参与分层检查——它装的是二进制与单元，不是代码。它是登录链唯一的部署入口，
  使用方式见[登录与开机链](../usage/login.md)。

规划中、**尚未创建**的组件：`inox-portal`（`xdg-desktop-portal` 后端）、`inox-screenshot`（截图/录屏）、`inox-osd`（屏幕提示）、`inox-cli`（`inoxctl` 命令行工具）。新增 crate 属破坏性架构变更，须先写 ADR。

## 各组件的 UI 技术栈与表面模式

七个 GUI 组件的界面都已从 Iced 迁到 **QML + Qt Quick**，经 **cxx-qt** 绑定到 Rust。分工是固定的：领域模型、状态机、Fluent 文案与设计令牌留在 Rust，Rust 把「状态 + 令牌 + 文案」投影成一份 JSON 交给 QML 渲染，QML 把用户动作作为窄 JSON 指令（`dispatch`）交回——不为每个字段单独开 `qproperty`。Iced 已从依赖图移除。

| 组件 | UI | 表面模式 | QML 宿主 |
|------|----|----------|----------|
| `inox-panel` | QML | 真实 `zwlr_layer_shell_v1` 表面（`src/layer.rs` 自己创建） | **离屏**：Qt Quick 软件场景图 + `grabWindow()`，帧写进自己的 `wl_shm` 缓冲 |
| `inox-lock` | QML | 真实 `ext_session_lock_v1` 表面（`src/session.rs` 自己创建） | **离屏**：每个锁表面一个 `QQmlApplicationEngine` + `QQuickWindow`，各自按 configure 尺寸出帧 |
| `inox-wallpaper` | QML | 普通 Qt 窗口（预览） | `QGuiApplication` + `QQmlApplicationEngine`，QML 根是 `Window` |
| `inox-launcher` | QML | 普通 Qt 窗口 | 同上 |
| `inox-notifications` | QML | 普通 Qt 窗口（通知栈） | 同上 |
| `inox-settings` | QML | 普通 Qt 窗口 | 同上 |
| `inox-greet` | QML | 普通 Qt 窗口（登录界面） | 同上 |

- **面板与锁屏不得退化为普通窗口**：它们的 `calloop` 主循环、协议表面、放置、输入与安全边界都不动，QML 只是渲染器；缺 layer/会话锁协议时以可读错误与非零退出，而不是把摆放策略当作已应用。
- **其余五个是普通窗口**：Qt 事件循环驱动它们，QML 同时负责渲染与交互；`placement.rs` 里的层表面策略仍只是数据，没有宿主会应用它。
- **绑定是 bin-only**：含 `#[cxx_qt::bridge]` 的 crate 没有 `lib.rs`，原有 `tests/*.rs` 迁进 bin 内的 `#[cfg(test)]` 模块；含 bridge 的 crate 在自己的 `[lints.rust]` 里把 `unsafe_code` 放开为 `allow`（cxx-qt 绑定的例外），Inox 手写的 Rust 仍然不写 `unsafe`。

### 已知缺口

- **离屏的面板与锁屏没有 AT-SPI 语义桥**：Qt 的 `offscreen` 平台没有 QPA 无障碍桥，因此这两处的 role/name/state 导出仍是 Q01 的缺口；五个窗口组件走标准 QPA 路径，但真实屏幕阅读器验收也未取得。Iced 时代同样缺这条桥，这不是回归。
- **真实宿主证据未取得**：waydriver 在真实合成器宿主上启动 L3 表面、截图并断言的 job **尚未接线**，所以「组件在真实 layer 表面上的样子」没有 E2 证据。
- **投影整体重发**：每次状态变化都重新序列化整份 JSON，QML 重新 `JSON.parse`；先保证边界干净与正确性，性能取舍等实测数据再决定是否做增量。

## 需要注意的几处

### `inox-common` 是叶子节点

`inox-common` 不得依赖任何其他 Inox crate，它是 L1 的依赖终点，因此只收纳**不依赖 Inox 其他部分**的共享能力。当前工作区版本为 `0.1.1`，提供 `logging`（统一的 `tracing` 初始化入口）与 `paths`（XDG 基础目录解析）模块。

这条规则由 `layering.rs` 机械断言：即使把 `inox-common` 加进同层例外表也无法放行，叶子规则先于例外检查。

### `inox-compositor` 的二进制名

PRD §3.1 曾把发布物记作 `inox-comp`；根级 `AGENTS.md` §5 说明这只是简写，实际二进制与你运行的命令都是 `inox-compositor`。

当前 `inox-compositor` 实现的是 **M1 + M2 + M3 的协议面与数据互通**：headless 输出创建、键盘与指针事件处理、以 `xdg-shell` toplevel 为窗口的 master/stack 平铺，以及 `wp_viewporter`、`wl_data_device_manager`（v3）与 `zwp_primary_selection_device_manager_v1` 的广告、焦点跟随的剪贴板与 Wayland↔Wayland 拖放（决策已记录，状态仍为 `proposed`）。为测试而加的输入注入套接字由非默认 feature `test-input` 与 `--test-input` 开关共同启用，默认构建不包含、未启用时该选项会被拒绝。`src/backend/drm.rs` 是 M2.5 真实后端的脚手架（决策状态 `proposed`）：后端特性已在 `Cargo.toml` 中启用并被 CI 编译检查，但真实 DRM/KMS 与 libinput 路径尚未在运行中打通。M3 未覆盖的部分：跨 X11/Wayland 边界的拖放被上游阻塞（satellite 无 XDND），`wlr-data-control` 与锁屏剪贴板断言推迟到 M5。详见[构建与运行](../usage/building.md)。

### L2 内部有一条已登记的同层边

`inox-compositor` 依赖 `inox-protocols`（核心 `wl_*` 绑定与私有 `inox-ipc`），而两者同属 L2。这既是分层规则 4（协议只能从 `inox-protocols` 出口）的要求，又与规则 1（禁止同层依赖）冲突——四层模型表达不了层内顺序。该边登记在 `[workspace.metadata.inox.rules] same_layer_exceptions` 中，删掉条目 CI 会立刻变红。

同样的情况在 L1 已存在（`inox-config → inox-common`）。规则 1 该如何表达层内顺序仍是待裁决项；现在它阻塞的是已经落地的代码，不再是假设。

### 合成器与 Shell 之间没有 Rust 依赖

L2 与 L3 之间的边界不是「暂时没有依赖」，而是分层规则 3 明确规定的设计：两者只能通过 Wayland 协议、`inox-ipc` 与 D-Bus 通信。这意味着 Shell 组件可以在虚拟合成器（`weston --backend=headless`）中单独测试，Shell 崩溃也不会带走合成器。该规则同样由 `layering.rs` 断言，双向都不允许。

### L3 访问合成器只有一个登记的单点

分层规则 6 允许 `inox-wallpaper` 直接依赖 `inox-compositor`，其余 L3 组件不得如此。这一单点登记在根 `Cargo.toml` 的 `[workspace.metadata.inox.rules] l3_dependencies_on_compositor` 中。**该例外目前是空的**：L3 的实现按交互契约的边界完成，壁纸通过 layer-shell 协议而不是合成器内部类型获得渲染面，因此没有任何 L3 crate 依赖合成器。删除登记条目属于**收紧**规则，同样需要 ADR 批准（PRD §2.1），所以条目保留，等待交互契约的裁决。

### L3 组件之间的 Rust 依赖有两个登记例外，都指向同一个 crate

七个 L3 GUI 组件是**七个独立进程**：面板是 layer-shell 客户端，锁屏是会话锁客户端，启动器、通知、壁纸与设置
是普通窗口应用（迁移后的界面均为 QML，见上一节），`inox-shell` 不持有界面。它们之间以及与
`inox-compositor` 之间只通过 Wayland 协议、`inox-ipc` 与 D-Bus 协作。共享能力放在 L1 的既有职责里：用户可见文案的加载在 `inox-common::i18n`，配置模型在 `inox-config`，
主题令牌在 `inox-config::theme`。每个组件仍各有「令牌 → QML 颜色与尺寸」的映射（`theme.rs`
里的 `qml_color` 之类的纯函数，有单测）与一小段 QML/宿主编织代码——这是明确接受的权衡，不是疏忽。

同层边有两条，都指向 `inox-glass`：

- **`inox-panel -> inox-glass`**（L3 → L3），由 ADR 0040 授权：面板的菜单栏与 Dock 的窗格由 `GlassPane` 绘制，
  面板在运行期用它的材质 API 做降级决策（`src/frost.rs`），在构建期用它的 QML 资产；
- **`inox-notifications -> inox-glass`**（L3 → L3），由 ADR 0044 授权：通知卡片同样由 `GlassPane` 绘制，
  但这条边**只在构建期**（`build.rs` 读 QML 资产），运行期不新增 Rust 依赖。

两条都登记在根 `Cargo.toml` 的 `same_layer_exceptions`。`inox-glass` 不持有界面进程、不做 I/O：它把
`tokens.glass` 与 `inox-glass-v1` 的 capabilities 合成同一个材质决策（clamp、v1/v2 协商、降级理由），
并携带五个无状态 QML 玻璃组件。没有这一层，每个组件都会复制同一段 clamp/降级算术，
而复制出来的版本会缓慢漂移。除此之外不允许再加入 L3 → L3 边：另一个共享 UI crate 仍须先写 ADR。

### 登录链：三个组件，三个权威

开机到桌面这条链上没有第四个权威：

- `inox-auth`（L2）**拥有认证与用户会话**：它验口令；一次登录一个**会话负责人**（`--session-leader`）
  在自己的进程里 `pam_open_session`，再由它降权 `exec` 成目标账号的 `inox-session`——`pam_systemd`
  只给调用它的那个进程注册 logind 会话，所以打开会话这件事不能留在共享的守护进程里；
- `inox-host`（L4）**拥有登录阶段的进程编排**：起合成器、起登录界面、有界重启、退出时逆序清理。
  它不认证、不碰凭据、不启动用户会话，也不依赖 `inox-auth` 的 `server` feature——它只知道套接字能不能连；
- `inox-greet`（L4）**拥有登录界面**，只经 `auth-v1` 与边界说话，不关窗也不重启自己（驻留复位）；
- **VT 与设备归 logind + libseat**：没有任何 Inox 组件执行 VT ioctl 或转发 DRM fd。

部署侧因此只有**三个单元、三个 PAM 栈**：`inox-auth.service`（root，常驻）与
`inox-greeter@<VT>.service`（模板，实例名就是 VT 号，`User=greeter`、`PAMName=inox-greeter`、
`ExecStart=/usr/bin/inox-host`）；栈是 `inox-auth`（口令在这里校验）、`inox-auth-session`（会话负责人
在这里打开会话，`auth` 行必须许可，因为它没有口令）与 `inox-greeter`（登录界面自己的 logind 会话）。
把前两个合成一个文件是**不能**做的事：许可行落在校验口令的栈上就是所有人免密登录。
第三个单元是 `inox-login-fallback.service`：它由登录单元的 `OnFailure=` 触发，会话前的重试预算耗尽、登录界面被判 failed 之后启动 greetd，让一次失败的启动仍然能登录；它不重新启用 greetd，也不改开机链。模板单元不能带 `Alias=display-manager.service`，开机链的别名由 `packaging/install.sh --enable`
对 `inox-greeter@1.service` 建立。真机证据（安装、试运行、切换、注销、回退、VT3 救援）**尚未取得**，
见[登录与开机链](../usage/login.md)。

### layer 表面与真实宿主证据

PRD §2.3 把 `wlr-layer-shell-unstable-v1` 列为**必须**，面板要用它；锁屏用的是 `ext-session-lock-v1`。这两条协议腿现在都在位：

- `inox-compositor` 已接入并广告 `zwlr_layer_shell_v1` v4（`tests/layer_shell.rs` 用真实客户端覆盖四层、独占区、焦点与隔离）；
- `inox-panel` 自己创建 `zwlr_layer_shell_v1` 表面，`inox-lock` 自己创建 `ext_session_lock_v1` 表面；两者都用离屏 QML 出帧，缺协议即以可读错误和非零码退出。

**仍然缺的是真实宿主证据**（不是协议腿）：CI 里没有「启动真实 L3 Qt 表面 → 截图 → 断言」的 waydriver 宿主 job，`weston` 也不支持 layer-shell。所以面板与锁屏的像素与交互只在离屏路径与真实协议 peer（socket pair / 子进程）中得到断言，它们在真实合成器上的样子**没有 E2 证据**。

历史备注：迁移前这里记录的两个缺口，一是合成器尚未广告 layer-shell、二是 `iced_layershell` 0.19.1/0.19.0 **无法编译**（其 `iced_exdevtools` 依赖对 `winit-core` 0.31.0-beta.3 的 `non_exhaustive` 枚举做穷尽匹配，`error[E0004]`）。第一条已由合成器补齐，第二条随 Iced 退出依赖图而失效——它是走离屏 QML 这条路的现实动因之一，不是当前架构。

启动器、通知、壁纸与设置仍以**普通窗口**运行，`placement.rs` 里的锚点、独占区、边距与尺寸是**已经测试但未被应用**的数据；这不再是「被上游阻塞」，而是为普通窗口组件选定的形态。

## 相关页面

- 每层的依赖规则见[总体架构](./overview.md)；
- 各组件的 CI 门禁见[持续集成](../development/ci.md)；
- 兼容性记录见[依赖管理](../development/dependencies.md)（工作区唯一的 `Cargo.lock`）。
