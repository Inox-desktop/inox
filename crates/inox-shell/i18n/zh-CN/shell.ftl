# Inox Shell 协调器（`inox-shell`）面向用户的文案。
#
# ID 遵循 L3 里程碑的 `<component>-<area>-<name>` 约定，并在 `src/i18n.rs` 的
# `id::ALL` 中登记；`tests/catalog.rs` 用它校验本文件与 en-US 目录一致。
# Fluent 的消息标识符不能含点号，因此分区用更多连字符表示。

### 命令行

shell-usage =
    用法：inox-shell [--help]

    inox-shell 是 Inox Shell 协调器。它解析会话的各个入口（面板、启动器、通知与设置），
    并报告各自读取的配置文件。这些入口是各自绘制界面的独立进程；本协调器自身不绘制任何内容。
shell-argument-unknown = `{ $argument }` 不是 inox-shell 认识的参数。

### 启动

shell-starting = Inox Shell { $version } 正在启动，语言 { $locale }。
shell-warn-no-layer-shell = Inox 合成器尚未广告 wlr-layer-shell，因此本会话的入口暂时无法成为桌面表面，其放置策略也不会生效（ADR 0018）。inox-shell 自身不绘制任何内容，也不创建表面。
shell-error-config = 无法加载 Inox 配置：{ $reason }
shell-error-catalog = 无法加载 Inox Shell 的文案目录：{ $reason }

### 入口注册表

shell-registry-summary = { $count ->
       *[other] 已解析 { $count } 个 Shell 入口：{ $entries }。
    }
shell-registry-entry = { $required ->
        [yes] { $binary } 负责 { $entry } 入口，读取 { $config }，是必需组件
       *[no] { $binary } 负责 { $entry } 入口，读取 { $config }，是可选组件
    }
shell-registry-unknown-entry = `{ $name }` 不是 Inox Shell 认识的入口；已知入口为 { $known }。
shell-registry-duplicate-entry = `{ $entry }` 入口被注册了多次。
shell-registry-invalid-config-path = `{ $entry }` 入口读取 `{ $path }`，该路径不是配置目录内的文件：{ $detail }
shell-registry-not-registered = 注册表中没有 `{ $entry }` 入口。

### 入口之间的导航

shell-nav-opened = 已打开 `{ $entry }` 入口；键盘交给 `{ $focus }`。
shell-nav-opened-display = 已打开 `{ $entry }` 入口，未夺取键盘。
shell-nav-already-open = `{ $entry }` 入口已经打开，没有变化。
shell-nav-closed = 已关闭一层；焦点返回窗口 { $window }。
shell-nav-closed-no-window = 已关闭一层；窗口焦点不变。
shell-nav-nothing-to-close = 没有打开的入口，因此没有可关闭的层。

### 返回目标失效时的回退

shell-nav-returned-to-fallback = 已关闭一层，未聚焦任何窗口：{ $reason }
shell-nav-fallback-window-destroyed = 该层打开时所处的窗口 { $window } 已不存在，改由合成器自身的焦点策略决定
shell-nav-fallback-session-changed = 该层打开于会话代次 { $saved }，而会话当前处于代次 { $current }，因此不复用它记录的窗口

### 服务状态

shell-service-starting = 正在启动
shell-service-ready = 就绪
shell-service-unavailable = 不可用；{ $actionable }
shell-service-failed = 失败；{ $reason }

### 命令结果

shell-command-sent = 已发送；服务尚未应答
shell-command-accepted = 服务已接受；改动尚未生效
shell-command-applied = 已生效；服务报告了实际效果
shell-command-rejected = 被服务拒绝；{ $reason }
shell-command-timed-out = 超过时限仍未收到应答
