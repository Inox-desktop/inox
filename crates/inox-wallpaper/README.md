# inox-wallpaper

壁纸加载与显示。属于 Inox 桌面环境的 **L3** 层，是 `wlr-layer-shell-unstable-v1` 的客户端
（PRD §3.1、[ADR 0013](../../docs/adr/0013-shell-service-and-interaction-contracts.md)）。

## 状态

**E0/E1 已交付**（编译、lint、文档、单元/属性/渲染测试全绿，见下），**E2/E3 被阻塞**。

交付内容：

| 模块 | 职责 |
|------|------|
| `src/model.rs` | `[wallpaper]` → 每个输出的选择：具名输出 → `*` → 共享 `image` → 主题背景 |
| `src/source.rs` | 选择 → 文件系统：存在、是文件、可打开；失败即降级并携带原因 |
| `src/decode.rs` | PNG/JPEG → RGBA8，带像素上限（`MAX_PIXELS`）与逐类错误 |
| `src/scaling.rs` | `fill` / `fit` / `stretch` / `center` / `tile` 的目标矩形，含平铺上限 |
| `src/apply.rs` | 原子应用：先解码、定位，再整体替换；逐输出独立成败 |
| `src/placement.rs` | 背景层 surface 纯数据策略（四锚点、`exclusive_zone = 0`、无键盘交互） |
| `src/contract.rs` | ADR 0013 的状态契约：代次、逐输出结果、服务状态、快照 |
| `src/app.rs` | Elm 状态与消息；`update` 不做 I/O，解码经 `Task::perform` 离开交互循环 |
| `src/view_model.rs` | 纯投影：文案（Fluent）、启用/禁用与可读原因、焦点顺序 |
| `src/view.rs` / `src/theme.rs` | Iced 组件树与令牌映射 |

**未交付（被上游阻塞）**：

- **真实背景层 surface**：`inox-compositor` 尚未广告 `wlr-layer-shell-unstable-v1`，且
  `iced_layershell` 0.19.1 / 0.19.0 都无法针对当前 `iced` 代际编译（ADR 0018）。因此二进制以普通
  `xdg-shell` 窗口运行——那是供人查看的**预览**，启动时记录一条 `warn!` 说明原因。
  `src/placement.rs` 的策略被计算与测试，但**不被应用**；本组件不声称它已生效。
- 配置热重载（`SIGHUP` / `inoxctl reload`，PRD §9.1）：本次未实现，配置只在启动时读取一次。
- E2 级别的 GUI 证据（waydriver / 真实宿主截图、语义树导出）：需要支持 layer-shell 的宿主。

## 配置

`shell.toml` 的 `[wallpaper]`（模型见 `inox-config`）：

```toml
[wallpaper]
mode = "fill"                  # fill | fit | stretch | center | tile
image = "backgrounds/wall.png" # 相对配置目录；也可写绝对路径
per_output = { "HDMI-A-1" = "wide.png", "*" = "default.png" }
```

## 失败即降级，且可诊断

ADR 0014 划的线在这里被逐条实现：**用户显式选择却读不到的资源必须报告**，而**没有选择**只是默认值。

- 文件缺失、是目录、读不了、解码失败、像素超限、几何退化、平铺份数超限 → 输出退到主题背景，
  屏幕**永不**变成黑屏，进程**永不**崩溃；原因以「输出名 + 路径 + 原因」呈现，文案来自 Fluent 目录。
- 已经显示中的壁纸不会因为一次重载失败而被清空：新帧**完整**之后才替换旧帧，失败时保留旧帧。
- 一个输出失败不影响其它输出，结果逐输出分别报告（ADR 0013 的快照/应用契约）。

对应的验收场景：**D05**（loading/empty/error/disabled/retry，禁用原因可读）、**Q02**（浅色/深色/
空态/错误态截图，1280×720）、**Q03**（`en-US` 与 `zh-CN` 目录 id 与占位符一致）。

## 运行与测试

> 仅作参考，不作为本地验收依据；真正判定在 CI。

```sh
cargo run -p inox-wallpaper                 # 打开预览窗口（需要一个 Wayland 或 X11 会话）
cargo test -p inox-wallpaper --all-features # 单元 + 属性 + 目录 + 渲染测试
cargo clippy -p inox-wallpaper --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc -p inox-wallpaper --no-deps --all-features
```

渲染测试把真实组件树按 1280×720 画进 `tiny-skia` 渲染器（无合成器、无 GPU），并对像素断言。
设置 `INOX_SCREENSHOT_DIR=<dir>` 时，每个场景额外写出 `inox-wallpaper-<scenario>.png`，
CI 可收集为 artifact（验收 Q02）。

## 开发约定

- 本组件遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的
  [`AGENTS.md`](./AGENTS.md)。
- 用户可见文案一律走 Project Fluent；颜色/尺寸/间距/圆角一律来自 `inox_config::Theme` 令牌。
- 新增依赖前先确认依赖方向合法（根级 §2）。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 用 `wallpaper`。

## 许可证

BSD 3-Clause，见仓库根的 [`LICENSE`](../../LICENSE)。
