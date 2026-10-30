# 总体架构

Inox 采用四层架构，**依赖只能向下**（`L4 → L3 → L2 → L1`），禁止反向依赖、跨层横向依赖与循环依赖。该约束由分层规则固定，条款见 PRD §2.1。

```text
┌─────────────────────────────────────────────┐
│ Layer 4: 会话与系统集成                     │
│  inox-session, inox-greet, inox-idle,       │
│  inox-host                                  │
├─────────────────────────────────────────────┤
│ Layer 3: Shell 与用户界面                   │
│  inox-shell, inox-panel, inox-launcher,     │
│  …, inox-glass                              │
│  inox-notifications, inox-settings,         │
│  inox-lock, inox-wallpaper                  │
├─────────────────────────────────────────────┤
│ Layer 2: 合成器与窗口管理                   │
│  inox-compositor, inox-xwayland,            │
│  inox-protocols, inox-auth                  │
├─────────────────────────────────────────────┤
│ Layer 1: 基础设施与共享库                   │
│  inox-common, inox-config                   │
└─────────────────────────────────────────────┘
```

## 各层职责

| 层级 | 职责 | 成员 |
|------|------|------|
| **L1** 基础设施与共享库 | 通用类型、错误处理、日志、配置解析 | `inox-common`、`inox-config` |
| **L2** 合成器、窗口管理、协议与认证边界 | DRM/KMS、输入、渲染、窗口管理、协议绑定、XWayland 集成、系统认证 | `inox-compositor`、`inox-protocols`、`inox-xwayland`、`inox-auth` |
| **L3** Shell 与用户界面 | 基于 QML + Qt Quick 的桌面外壳与 UI 组件（领域逻辑仍在 Rust） | `inox-shell`、`inox-panel`、`inox-launcher`、`inox-notifications`、`inox-settings`、`inox-lock`、`inox-wallpaper`、`inox-qml-prototype`、`inox-glass`（共享液态玻璃材质与 QML 组件；两条同层边 `inox-panel -> inox-glass` 与 `inox-notifications -> inox-glass` 见 ADR 0040/0044） |
| **L4** 会话与系统集成 | 会话管理、空闲管理、显示管理、登录阶段编排 | `inox-session`、`inox-idle`、`inox-greet`、`inox-host` |
| 元 / 组织 | 文档与组织级模板，不参与依赖图 | `inox`、`.github` |
| 测试 | 端到端集成测试 | `inox-integration` |

## 桌面 Shell 的 UI 栈

七个 GUI 组件（面板、启动器、通知、设置、锁屏、壁纸、登录界面）的界面**不是 Rust 代码**：QML + Qt Quick 负责声明式界面，Rust 保留领域模型、Fluent 文案与设计令牌，两者经选定的 **cxx-qt** 绑定连接。边界只有两条：Rust 把「当前状态 + 令牌 + 文案」投影成一份 JSON（`model_json`）交给 QML，QML 把用户动作写成一条窄 JSON 指令（`dispatch`）交回 Rust；不为每个字段单独开一个 `qproperty`。

表面有三种模式：

- **普通窗口**：壁纸、启动器、通知、设置与登录界面用 `QGuiApplication` + `QQmlApplicationEngine` 打开一个 Qt 窗口，QML 根是 `Window`；
- **原生 shell 表面**（ADR 0046，面板的终点）：面板进程用 Qt 的 wayland 平台建一个**真实窗口**，再由 crate 内的私有 ABI 给它 `inox_shell_surface_v1` 角色；合成器负责放置、插帧与合成，Qt 自己按 `wl_surface.frame` 出帧，没有 `grabWindow()`、没有 `wl_shm` 上传。协议层没有"动画完成"事件，所以减少动态或合成器不支持时整段过渡都会被丢掉，面板必须在"所有过渡都是瞬时"下也好看。
- **离屏渲染**（面板在没有私有头的构建上、以及锁屏现在仍然走的路）：组件自己创建 `zwlr_layer_shell_v1` / `ext_session_lock_v1` 表面，仍由 `calloop` 驱动主循环；Qt Quick 在 `offscreen` 平台把 QML 渲染成 RGBA 帧，帧被复制进组件自己持有的 `wl_shm` 缓冲。QML 在这里只是渲染器，输入、焦点、放置与安全边界全部在 Rust。

原生与离屏这两条腿由**构建期探针**二选一（`build.rs` 看 Qt 版本与私有头），不是运行时开关：ADR 0046 §9 明确禁止"拿不到 `inox_shell_v1` 就退回 layer-shell"，因为那会让两套放置代码在半数机器上分叉。

合成器始终是 **Smithay**，绝不用 Qt Wayland Compositor（GPL-only）；Iced 已从依赖图移除。逐组件的表面模式与已知缺口见[组件清单](./components.md)。

## 依赖规则

1. 依赖方向只能向下：`L4 → L3 → L2 → L1`。允许跨层向下（例如 L3 直接依赖 L1），禁止反向与跨层横向依赖。
2. `inox-common` 是**叶子节点**，不得依赖任何其他 Inox crate，也不得依赖 `inox-config`。
3. `inox-compositor` 与 `inox-shell`（及其 L3 子组件）之间**不建立 Rust 依赖**，只通过 Wayland 协议、`inox-ipc` 与 D-Bus 通信，二者必须能各自独立启动与独立测试。
4. `inox-protocols` 是协议的唯一出口：新增协议必须先在 `inox-protocols` 中定义或引用，并说明安全与生命周期约束（根级 `AGENTS.md` §4）。
5. L3 对 L2 的依赖有明确例外：`inox-wallpaper` 可依赖 `inox-compositor` 的渲染能力；其余 L3 组件不得直接依赖 `inox-compositor`。
6. `inox-integration` 是测试聚合 crate，不受上述方向限制，但只允许在不可变 commit 或发布 tag 上做验证。
7. 循环依赖、反向依赖与横向依赖由 CI 检查拦截（`cargo tree` 审计与 `cargo deny check` 的 `[bans]` 段）。

## 为什么这样分层

分层规则把依赖方向作为单条判据固定下来：任何一个 crate「可以引入什么依赖」只需看分层即可判断，无需逐个协商。这条判据原先靠仓库边界承载，实际并没有被任何检查执行；改为单一工作区后，它由 `cargo metadata` 驱动的 `layering.rs` 逐条机械断言。收益是每个 crate 仍可独立编译、独立测试（`cargo test -p <crate>`），同时跨 crate 的改动可以在一个 PR 内原子完成，不必走「上游打 tag → 下游逐个升级」；代价是跨组件调用仍需经过协议与序列化——这是设计约束，与代码组织形态无关。

依赖图之外的硬性技术约束（Smithay 抽象、calloop 事件循环、Shell 的 QML + Qt Quick 与 cxx-qt 绑定）见技术栈的决策：其中 Shell 的 GUI 部分已由 QML + Qt Quick 与 cxx-qt 取代——技术栈原文固定的 Iced 已不再是 Shell 的 UI 栈，这条例外已记录。

## 运行时数据流

```text
 应用 (Wayland/X11 客户端)
        │  wayland socket
        ▼
 ┌───────────────────┐   inox-ipc (自定义 Wayland 协议)
 │  inox-compositor  │◄───────────────────────────┐
 └───────────────────┘                             │
        ▲  layer-shell / xdg-shell              ┌──┴────────────┐
        │                                       │  inox-shell   │
 ┌──────┴────────┐   D-Bus (notifications,      │  及 L3 组件   │
 │  inox-session │   settings, portals)         └───────────────┘
 └───────────────┘
```

- **Wayland 协议**：客户端通过 `wayland socket` 连接合成器；Shell 组件使用 `wlr-layer-shell`、`xdg-shell` 等标准协议，以及 Inox 自定义的 `inox-ipc`（定义于 `inox-protocols`）。
- **D-Bus**：通知、设置、`xdg-desktop-portal` 等跨进程服务走会话总线。
- **配置热重载**：组件监听 `SIGHUP` 或 `inoxctl reload`，重新读取 TOML 并原子应用；重载失败保留旧配置并记录警告。

## 登录链：开机到桌面

桌面之外还有一段链，它必须先跑起来，上面那些组件才存在：

```text
systemd
 ├─ inox-auth.service                 root，常驻，Restart=always
 │    └─ /run/inox/auth.sock          root:greeter 0660；只接受 greeter 的 uid
 │       ├─ Authenticate              校验口令（/etc/pam.d/inox-auth）
 │       └─ StartSession ──► 会话负责人（一次登录一个进程）
 │                             ├─ pam_open_session（XDG_VTNR 由 root 父进程给出）
 │                             │  /etc/pam.d/inox-auth-session（auth 必须许可：负责人没有口令）
 │                             ├─ logind.ActivateSession（新会话的 VT）
 │                             └─ exec inox-session（目标账号，已降权）
 └─ inox-greeter@<VT>.service         User=greeter, PAMName=inox-greeter
      └─ inox-host                    TTYPath=/dev/tty<VT>, XDG_VTNR=<VT>
           ├─ inox-compositor --socket wayland-inox-greet
           └─ inox-greet              INOX_AUTH_SOCKET=/run/inox/auth.sock
```

- **VT 与设备归 logind + libseat**：Inox 组件不执行 VT ioctl、不转发 DRM fd。greeter 的设备访问来自它自己的
  logind 会话，用户阶段的来自会话负责人建立的那个会话；VT 切换由 `inox-auth` 调 `ActivateSession` 完成。
- **VT 归属**：greeter 在 `inox-greeter@<VT>`（实例名即 VT 号，开机链是 1）；用户会话由 `inox-auth` 取
  VT2..=6 中第一个空闲且不等于 greeter 的 VT；同时只允许一个用户会话。
- **注销回路**：用户会话结束时 `inox-auth` 重新激活它启动前记录的那个会话（通常就是 greeter），
  登录界面一直驻留、3 秒后回到登录画面，因此不需要谁重新拉起它。
- **救援通道**：`getty@tty3.service` 常驻，任何登录链故障都能 `Ctrl+Alt+F3` 进去修。
- **部署**：`packaging/`（三个 PAM 栈、三个单元、安装/试运行/预检/回退/卸载脚本）。默认安装**不动开机链**，
  试运行用 VT4，`--enable` 才把 `display-manager.service` 交给 `inox-greeter@1.service`。
  步骤与排障见[登录与开机链](../usage/login.md)。

真机证据**尚未取得**：安装、试运行、切换、注销、回退与 VT3 救援都还没有人在这台机器上跑过并留证
（验收 S06–S11）。

## 当前实现边界

架构图描述的是目标形态。就本工作区当前状态而言：L2 的 `inox-compositor` 在 headless 路径上实现了输出创建、输入事件与窗口管理（四种窗口模式、≥2 工作区，并广告私有 `inox-window-manager-v1` 供任务栏观察与驱动；证据 E1+E2，见[里程碑与路线图](../roadmap.md)）；`inox-common` 提供 `logging` 与 `paths` 模块；`inox-protocols`、`inox-config`、`inox-xwayland`、`inox-auth`、`inox-greet` 均有实现。L3 的七个组件已从 Iced 迁到 QML + Qt Quick：面板能编原生宿主时拿 `inox_shell_surface_v1` 角色（Qt 的真实窗口，ADR 0046 P3），否则仍持有真实的 `zwlr_layer_shell_v1` 表面（QML 离屏渲染进 `wl_shm` 帧）；锁屏仍持有真实的会话锁表面并把 QML 离屏渲染进去；其余五个是普通 Qt 窗口。L3 新增共享的 `inox-glass`（ADR 0040）：面板经它向合成器请求模糊与折射，并和通知栈一起用它的玻璃 QML 组件绘制自己的外壳（面板的窗格与通知的卡片，后者见 ADR 0044）；通知栈是普通窗口，拿不到 layer 角色，因此它的卡片在真实会话里是**纯半透明**而不是磨砂。**仍未取得**的是真实宿主上的截图证据——waydriver 宿主 job 尚未接线；离屏的面板与锁屏也没有 AT-SPI 语义桥。L4 的登录链（`inox-host` 的编排、`inox-auth` 的会话负责人与 VT 交回、`packaging/` 的三个 PAM 栈与三个单元）已按登录链落地，但**真机登录尚未被验证**（E4 证据未取得）；`inox-session` 与合成器之间的数据流仍不完整。
