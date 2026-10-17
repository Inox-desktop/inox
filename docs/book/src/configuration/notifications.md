# 通知配置（`notifications.toml`）

> **状态：字段模型已实现，守护进程为部分实现。** 模型位于 `inox-config` 的 `src/notifications.rs`，
> 有默认值与范围校验的单元测试。`inox-notifications` 按 [ADR 0018](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0018-l3-component-architecture.md)
> 交付到 **E0/E1**：协议契约、队列与策略、脱敏与界面模型已实现并测试；**D-Bus 服务本身属 M5**
> （PRD §13 的 D06），尚未实现，因此本页不是「通知已经可用」的声明。

```toml
[display]
position = "top-right"     # top-right | top-left | bottom-right | bottom-left
max_visible = 3            # 1..=16
width = 340                # 160..=1024
margin = 12                # 0..=256
icon_size = 32             # 8..=128

[timeouts]
low_ms = 5000              # 0..=600000，0 表示不自动过期
normal_ms = 8000
critical_ms = 0            # 关键通知默认不自动消失

[queue]
max_pending = 32           # 1..=1024
policy = "drop-oldest"     # drop-oldest | coalesce-by-app

[policy]
do_not_disturb = false     # 仅为启动时的初值
show_actions = true
hide_body_when_locked = true
```

## `[display]`

| 键 | 类型 | 默认值 | 范围 | 说明 |
|----|------|--------|------|------|
| `position` | 字符串 | `"top-right"` | 四个角 | 通知堆叠的位置；与 `margin` 一起构成弹窗的放置策略 |
| `max_visible` | 整数 | `3` | 1..=16 | 同时可见的通知数；超出部分留在队列里并明确告知 |
| `width` | 整数 | `340` | 160..=1024 | 单条通知的宽度（像素） |
| `margin` | 整数 | `12` | 0..=256 | 与屏幕边缘的间距 |
| `icon_size` | 整数 | `32` | 8..=128 | 图标尺寸 |

## `[timeouts]`

按紧急程度给出的自动过期时间（毫秒）。`0` 表示**不自动过期**：关键通知的默认值 `critical_ms = 0`
意味着它只有被显式关闭才会消失。

| 键 | 默认值 | 对应紧急程度 |
|----|--------|--------------|
| `low_ms` | `5000` | `urgency = 0` |
| `normal_ms` | `8000` | `urgency = 1` |
| `critical_ms` | `0` | `urgency = 2` |

## `[queue]`

| 键 | 类型 | 默认值 | 范围 | 说明 |
|----|------|--------|------|------|
| `max_pending` | 整数 | `32` | 1..=1024 | 待显示队列上限。队列**有界**：溢出时按策略处理并计数，不无限增长 |
| `policy` | 字符串 | `"drop-oldest"` | `drop-oldest` / `coalesce-by-app` | 溢出策略：丢弃最旧的，或按应用合并 |

两种策略都必须**可观察**：守护进程统计丢弃、合并与被替换的条数，验收 D06 要求这些计数可被读出，
而不是让溢出静默发生。替换（`replaces_id`）在原位更新并重置计时，不占用新的队列位。

## `[policy]`

| 键 | 类型 | 默认值 | 说明 |
|----|------|--------|------|
| `do_not_disturb` | 布尔 | `false` | **仅启动初值**：勿扰状态由守护进程持有，面板在运行期切换；配置不描述运行期状态 |
| `show_actions` | 布尔 | `true` | 是否显示通知动作按钮 |
| `hide_body_when_locked` | 布尔 | `true` | 会话锁定时隐藏正文、应用名与图标，改显示通用摘要（ADR 0015：锁定状态不得显示敏感正文） |

勿扰只抑制**呈现**，不抑制投递：队列照常计数，动作仍然有效，且勿扰不会抢走焦点（验收 D06）。

## 文本与动作的边界

通知正文只识别 `b`、`i`、`u`、`a href` 与 `img` 这一子集，其余标记与裸 `<`、`&` 一律转义为文本；
`a href` 只接受 `http`、`https` 与 `mailto`，`javascript:`、`data:`、`file:` 一律按纯文本处理。
清理后的结果是**结构化的 span 列表**而不是 HTML 字符串，视图再按令牌着色。动作标识在一条通知的
生命周期内**至多触发一次**，触发即关闭该通知，重复触发被拒绝并记录。

## 相关页面

- 顶层 `config.toml` 与组件文件路径见[配置目录布局](./layout.md)；
- 面板、启动器与壁纸见[面板、启动器与壁纸配置](./shell.md)。
