# UI 重做独立复核报告

> **Lead 复核后的更新（2026-10-17）**：本报告 §6.1 的两条 P1 已修复并重跑 —— `inox-panel` 补了 200% 字号渲染场景（`tests/render.rs::text_scaled_to_two_hundred_percent_stays_inside_the_strip`，15 个渲染测试全绿，快照 `target/ui-redesign/inox-panel/after/inox-panel-scale-200.png`）；`inox-greet` 主按钮悬停改为 `on_accent` hairline（`src/theme.rs::submit_style`，原「不响应指针」断言改为「填充不变 + 悬停可见」，16 个渲染测试全绿）。§6.2 的三条已由 Lead 裁决并写入 `ui-guidelines.md`（accent 主按钮的悬停画法、启动器输入框的 `title` 角色例外、身份图标底色的例外）。
> 本报告 1355 条测试结论建立在修复**前**的六个 crate 上；上述两处改动之后只重跑了 panel 与 greet。`inox-wallpaper` 及其依赖的三个 crate 正被本会话之外的并行写入者改动（§8），本报告不判断其编译状态。

- 复核者：`ui-verifier`（未参与任何重做写入，结论只来自本人读代码、读截图、跑测试）。
- 被检查规范：[ui-guidelines.md](ui-guidelines.md)（`7ed0bb67…`，140 行）与根 `AGENTS.md` §3–§7。
- 复核范围：`inox-panel`、`inox-launcher`、`inox-notifications`、`inox-settings`、`inox-lock`、`inox-greet` 六个 crate。
- 复核时间：2026-10-17 20:45–21:09（本机时区 -0700）。
- 基线：分支 `feat/settings-complete`，`HEAD = 6bbe60d`。工作树有 159 项未提交改动。
- 重要限制：本次 UI 重做**未提交**，与更早的功能改动（reload、live config、display trial 等）混在同一工作树里，因此 `git diff` 中的改动**不能**全部归因于 UI 任务；下文只对能逐行核对的断言负责。

## 1. 方法与我自己的证据

- `cargo test -p <crate> --all-targets` × 6（未用 `--workspace`）。
- `cargo fmt -p <crate> -- --check` × 6。
- `cargo clippy -p <crate> --all-targets --all-features -- -D warnings` × 6。
- 截图可复现性：`INOX_SCREENSHOT_DIR=target/ui-redesign/<crate>/after cargo test -p <crate> --test render` × 6，再逐文件比对 md5。
- 目录一致性的独立脚本 `target/verify/ftl-parity.py`（自己解析 `.ftl` 的 id 与 `{$var}`，不依赖 crate 测试）。
- 非测试路径字面量扫描脚本（自己匹配 `Rgba {…}`、`from_rgba8`、`padding(`、`radius(`、`spacing(`、`.size(`、`Length::Fixed(`、`Padding::new(`、`Radius::new(`）。
- `read_image` 逐张看图，并用 `PIL` + `numpy` 做像素测量（框高、色值、行列游程）交叉验证肉眼判断。
- `git diff` 扫描六 crate 全部被删/被改的 `assert` 行。
- `stat -c '%y %n'` 取外部写入者文件的 mtime。

本机结果**不是验收依据**（根 `AGENTS.md` §3.1）。下文「通过」一律指「本地自检通过」，验收在 CI。

## 2. 总览

| crate | 机械核对 | 像素核对 | 本地测试 | 结论 |
|-------|----------|----------|----------|------|
| `inox-panel` | 通过 | 通过（条带例外） | 165 passed / 0 failed | 符合，缺 200% 渲染证据 |
| `inox-launcher` | 通过 | 通过（层次/分组/宽度明显改善） | 217 passed / 0 failed | 基本符合，3 项需 Lead 判断 |
| `inox-notifications` | 通过 | 通过 | 342 passed / 0 failed | 符合；焦点态在无输入路径表面不适用 |
| `inox-settings` | 通过 | 通过（短字段已收窄） | 312 passed / 0 failed | 符合；自检正则偏窄 |
| `inox-lock` | 通过 | 通过 | 208 passed / 0 failed | 符合 |
| `inox-greet` | 通过 | 通过 | 111 passed / 0 failed | 有一条 §5 实质缺口（主按钮 hover） |

六 crate 合计 **1355 passed / 0 failed / 32 个测试二进制**（`target/verify/tests-full.log`）。

## 3. 机械核对（逐 crate）

命令：`python3 target/verify/ftl-parity.py`（id 与占位符两侧比对）、字面量扫描脚本、`grep -rnE 'Color::(from_|BLACK|WHITE|TRANSPARENT)' crates/<crate>/src/`。

| crate | 四字号角色 | 硬编码颜色 | 硬编码尺寸 | i18n id/占位符 | 五状态 pairwise 单测 |
|-------|-----------|-----------|-----------|---------------|---------------------|
| panel | `theme.rs:81,86,91,99`（1.5/1.25/1/0.875） | 无 | 无 | 58/58 一致 | `theme.rs:486`（5 态 × 3 emphasis） |
| launcher | `theme.rs:35-87`（`TextRole`） | 无 | 无 | 31/31 一致 | `theme.rs:676`（5 态）；`theme.rs:739`（动作 4 态） |
| notifications | `theme.rs:37-80` | 无 | 无 | 14/14 一致 | `theme.rs:696`、`theme.rs:720`（各 4 态） |
| settings | `theme.rs:34-125` | 无 | 无 | 189/189 一致 | `theme.rs:787`、`:1046`、`:1083` |
| lock | `theme.rs:74-110` | 无 | 无 | 44/44 一致 | `theme.rs:661` |
| greet | `theme.rs:59-93` | 无 | 无 | 29/29 一致 | `theme.rs:639` |

- 四角色倍数六 crate 全部为 **1.5 / 1.25 / 1.0 / 0.875**，`caption = 0.875 × base ≥ 0.75 × base`（§2 下限）成立，未出现第五种字号。
- 字面量扫描结果 **0 命中**；`Length::Fixed(...)` 的实参全部是 `geometry.*` / `metrics.*` / 令牌派生值（如 `crates/inox-settings/src/view.rs:185`、`crates/inox-launcher/src/view.rs:157`）。
- 唯一 `Color::WHITE` 位于 `crates/inox-lock/src/render.rs:229`，是 `Headless::screenshot` 的合成底色（非界面颜色，且有注释说明测试改为断言帧不透明）。判为**边界例外，不违规**。
- `theme.rs` 里的 `Color::TRANSPARENT` / `width: 0.0` 是「无边框」的结构性哨兵，不是设计色（`crates/inox-lock/src/theme.rs:283`、`crates/inox-greet/src/theme.rs:202`）。

### 3.1 机械核对发现

1. `inox-settings` 自带的反字面量单测 `crates/inox-settings/src/view.rs:1526` 只扫 `padding(1|2)`、`.size(1|2)` 与四种 `Color::` 字面量，**漏掉** `radius(`、`spacing(`、`Length::Fixed(`、`Padding::new(` 与 ≥3 的数字。实际代码没有字面量（我的扫描可证），所以状态通过，但该自检的覆盖面窄于 §10 的字面要求。
2. `inox-launcher` 的 `Metrics::control_padding`（`crates/inox-launcher/src/theme.rs:142-151`）上下用 `spacing.sm`，且**没有** §4「交互目标 ≥ `spacing.xl`」的断言；其余四个窗口型 crate 都有（`inox-notifications/src/view.rs:594`、`inox-settings/src/theme.rs:982`、`inox-lock/src/theme.rs:631`、`inox-greet/src/theme.rs:787`）。实测达标见 §4.2，属防回归断言缺口。
3. `inox-notifications` 的按钮只有四态单测，焦点环（`src/theme.rs:230`、测试 `:663`）当前**不可达**：弹窗栈 `KeyboardInteractivity::None`（`src/placement.rs:184`、`:332`），视图模型没有焦点字段（`src/view_model.rs:34`）。这是结构性「没有输入路径」，不是缺陷，但 §5「每个可交互元素五状态」在该表面不适用——建议在规范里显式写出这个例外，而不是留在代码注释里。

## 4. 像素核对（我自己看到的）

### 4.1 `inox-panel`（1280×32）

- `before/inox-panel-dark-theme.png` vs `after/…`：before 是一条平铺文本，条带底部贴边；after 每个条目有 hairline 圆角边框、垂直内缩（`view.rs:125` 的 `content_height()`），圆角与焦点环完整落在条带内（§4 条带例外成立）。托盘前出现 hairline 分隔，时钟用 caption 色。
- `after/inox-panel-pressed.png`：按下条目整块填 accent、文字 `on_accent`（§5 按下态）。
- `after/inox-panel-focused-under-pointer.png`：焦点用 accent 描边且**在指针悬停时仍然可见**（Q01）；但该图里 **tooltip 文本与标签重叠**（"firefox: Inox — Documentation (2" 压在标签上）。before 同样重叠 → **非本次回归**，是 32px 高离屏缓冲无法容纳 tooltip 的夹具限制；这张图不能作为「不重叠」的证据。
- `before/after inox-panel-long-title.png`：before 最后一条标签与时钟 "19:38" 相撞；after 时钟完整、最后一条在自己的框内被按字形切断（无省略号）。时区/时间在两套图里不同（before 19:38 / after 19:49），不影响布局判断。

### 4.2 `inox-launcher`（1280×720）

- `before/inox-launcher-dark.png`：搜索框满宽（§9 反例）；三条结果行**全部**填 accent，图标列被拉满整行，不可用条目的原因文本溢出到行外，窗口中部是未分组的空白。
- `after/inox-launcher-dark.png`：居中卡片（`surface` + hairline + `md` 圆角 + `lg` 内边距），标题 `Applications`（heading）+ 说明（caption），图标是方形 tile，只有选中行填 accent，不可用条目在卡内换行出原因（caution 色），页脚提示独立成状态条，搜索框收窄并戴 accent 焦点环。
- `after/inox-launcher-unavailable.png`：heading + caption 原因 + `Try again` 次级按钮，错误态含重试（§6）。
- 像素实测（`PIL` + `numpy`，坐标取色）：搜索框外框高 **39 px**、`Try again` 高 **35 px**、选中行高 **55 px**，均 ≥ `spacing.xl`=32；`after/inox-launcher-scale-200.png` 里搜索框外框高 **68 px**，随缩放增长。字与框没有互相重叠；最下面一行落在卡片内的可滚动视口之外（列表未滚到底），属列表视口行为而非文字裁切。
- `before/after inox-launcher-unavailable.png`：before 也是居中文字但标题与正文同字号、提示贴在窗口左下角；after 层次与分组成立。

### 4.3 `inox-notifications`（1280×720）

- `before/after inox-notifications-dark-theme.png`：before 卡片紧凑、标题用高饱和红色承载紧急度、`Open` 是小 chip、`Close` 只是文字、没有时间戳；after 卡片用 `lg` 内边距与 `lg` 间隔、标题改为 heading 字重/字号、`Open` 与 `Close` 都成了 ≥32px 的有框按钮、紧急卡片多出 `Critical just now`（句子 + 时间戳），普通卡片多出 `Just now`。
- `after/inox-notifications-disabled-with-reason.png`：禁用的 `Open` 是 muted 文字 + 无填充，**卡内有原因句**「Actions are turned off in notifications.toml」（§5 禁用须给原因、§6 错误可读）。
- `after/inox-notifications-urgency-critical.png` / `-urgency-low.png`：紧急度同时是颜色 + 句子（§5「颜色是辅助，句子是主体」）。

### 4.4 `inox-settings`（1280×720）

- `before/inox-settings-expanded-en-US-Displays.png`：页面标题与正文几乎同字号，`Connector`/`Width`/`Height`/`Refresh rate`/`Scale` 全部字段拉满窗口（§9 反例），没有卡片。
- `after/inox-settings-expanded-en-US-Displays.png`：页面 title + caption，卡片 `Outputs`（heading）内是窄列表格（数值列 ≤ 6×`xl`），`Add entry` 次级、`Remove entry` 用 danger 描边，页脚状态条 + `Apply`(accent)/`Cancel`/`Reset all to defaults`(danger) 三分成立（§5）。
- `after/inox-settings-focused.png`：`Panel` 导航是 accent 背景（选中），`Keyboard & Mouse` 是 accent 描边（焦点），两者**同时可见**（§5 选中/焦点可共存）。
- `after/inox-settings-enlarged-navigation.png`（200%）：无裁切、无重叠，焦点环与选中背景都还在。
- 遗留：卡片下方仍有一大片留白（Displays 页约 230 px；200% 的 Appearance 页更明显）。§9 把「底部一大片空白」列为反例，本次以卡片建立了分组但没消除留白。

### 4.5 `inox-lock`（1280×720）

- `before/after inox-lock-dark.png`：heading 变大并与字段分开；标签「Password」与占位符不再互相挤压；`Unlock` 由满宽改为 2/3 宽（`CONTROL_PORTION`），提示句移到按钮下方。
- `after/inox-lock-unavailable.png`：`The session lock cannot run`（heading）+ danger 的理由 + caution 的状态句，**没有**重试控件，`Unlock` 为禁用（muted 文字、无填充、无描边）。
- 像素复核禁用的 `Unlock`（x 335–750、y 380–425）：区域内只有卡片底色 `(35,38,46)` 与字形色 `(165,173,186)`，确认**没有**填充与描边（我先前的肉眼印象是错的，这里以测量为准）。

### 4.6 `inox-greet`（1280×720）

- `before/after …ready-inox-dark-en-US-1.png`：after 的 Username/Password 收窄到 2/3 卡宽、`Log in` 戴 **`on_accent` 焦点环**（accent 背景上的白描边，§5 明文要求）、`Clear` 成为 ≥32px 的次级按钮、提示句移到按钮下方、`Session Inox` 独立成卡片（分组 §4）。
- `after/…absent-authenticator.png`：danger 的「Log in cannot run」+ 可读原因句，`Clear` 承载焦点环，无重试控件（§6「不得提供做不到的动作」）。
- `after/…failed.png`：`That login was not accepted` 为 danger 句子，字段仍可输入。
- 遗留 a：`absent-authenticator` 里 `Username`/`Password` **只剩标签、没有可见的输入框外形**（`src/view.rs:157` 只在 `editable && focused` 时画编辑器，`field_block` 的框在禁用态为透明）。原因句在下方，符合 §5「禁用须给原因」，但「标签—控件」的视觉对应丢失，看起来像缺控件。
- 遗留 b：信息句「Logging in starts an Inox session.」在 before/after 都用 caution 橙色，容易被读成警告。

## 5. 回归核对

### 5.1 测试与自检

| crate | 命令 | 结果 |
|-------|------|------|
| panel | `cargo test -p inox-panel --all-targets` | 165 passed / 0 failed |
| launcher | `cargo test -p inox-launcher --all-targets` | 217 passed / 0 failed |
| notifications | `cargo test -p inox-notifications --all-targets` | 342 passed / 0 failed |
| settings | `cargo test -p inox-settings --all-targets` | 312 passed / 0 failed |
| lock | `cargo test -p inox-lock --all-targets` | 208 passed / 0 failed |
| greet | `cargo test -p inox-greet --all-targets` | 111 passed / 0 failed |

- `cargo fmt -p <crate> -- --check`：六个 crate 全部无 diff。
- `cargo clippy -p <crate> --all-targets --all-features -- -D warnings`：六个 crate 全部 `exit 0`（日志 `target/verify/fmt-clippy.log`）。
- 证据链完整性：用当前源码重跑六个 crate 的 render 测试后，对 `target/ui-redesign/` 下 **216 张 PNG 逐文件比对 md5：0 张变化**。其中 `after/`（122 张）中的 107 张由本次重跑覆盖写出（wallpaper 的 15 张未重跑）、`before/`（94 张）仅作对照——即本次复核的 `after/` 截图确实可由当前源码逐字节复现，不是旧产物。
- `target/inox-shots/` 的 25 张 settings PNG 与 `target/ui-redesign/inox-settings/before/` **逐字节相同**（25/25），改动前证据链成立。

### 5.2 被删除/改变的既有断言（逐条判定）

`git diff` 扫描六 crate 全部被删的 `assert`/`#[test]` 行，逐条归类如下。**没有发现被削弱或删除的既有断言**；`retry`→`action`、`text_size`→`body_size`、`small_text_size`→`caption_size`、`headline_text_size`→`title_size` 属重命名，语义保留。

| 位置 | 改动 | 判定 |
|------|------|------|
| `crates/inox-launcher/tests/render.rs:184-205` | 窗口左上角像素由 `surface` 改为 `background`，并新增 `assert_ne!(background, surface)` | **必要**：窗口底色与卡片底色现在分工（§4 表面层次），角点在卡片外；新增断言使强度不降 |
| `crates/inox-launcher/tests/render.rs:305-312` | 无结果态由 `!contains(accent)` 改为 `!contains_block(accent, 3×icon, icon)` | **必要**：聚焦搜索框现在戴 accent 焦点环，「全屏无 accent」已不成立 |
| `crates/inox-launcher/tests/render.rs`（新增 `the_search_field_wears_the_focus_ring_and_the_rest_of_the_screen_does_not`） | 对**未聚焦**画面断言 `!contains(&resting, accent)` | **补偿且更强**：旧断言覆盖的场景由这条以更严格方式接住 |
| `crates/inox-panel/src/view.rs:461-463` | `font_size <= height - padding*2` 改为 `<= content_height()` | **等价改写**：与新条带例外（条目高 = 条带高 − `xs`）一致，bound 由 −2×`sm` 变 −2×`xs` |
| `crates/inox-panel/src/theme.rs`（焦点测试） | 删 `assert_ne!(focused.background, idle.background)`，加 accent 描边 / `focus_ring` 宽度 / text 与 border 双差异 | **更强且合规**：焦点改为描边（§5「焦点用描边」，选中才用背景） |
| `crates/inox-panel/src/theme.rs`（圆角测试） | 删 `assert_eq!(style.border.width, 0.0, "the panel draws no border")` | **过时断言**：条目现在用 hairline 边框做分组（§4），新测试断言 `hairline(&tokens)` |
| `crates/inox-settings/src/theme.rs`（动作测试） | `action(bool)` → `action(Role)`，新增 `Role::Danger` 的三条断言 | **更强**：danger 不得填 accent、不得与次级同色 |
| `crates/inox-settings/tests/render.rs`（空列表） | `ControlView::Collection` 新增 `empty` 字段并断言 `!empty.trim().is_empty()` 且空表不画输入行 | **更强**：§6 空态必须有句子 |
| `crates/inox-settings/src/pages/mod.rs:426` | 新增「卡片分组顺序 == `PageId::fields` 顺序 == 单卡不重复」 | **更强**，并与键盘顺序挂钩 |
| `crates/inox-settings/tests/workflow.rs` | 只增不删（+136 行） | 无削弱 |
| `crates/inox-notifications/src/theme.rs`（close button） | 删 `assert_ne!(idle.text_color, hovered.text_color)`，改为断言 hover 背景 = `surface_variant` | **更合规**：§5 悬停是背景 |
| `crates/inox-notifications/tests/bus.rs` | `zbus::Error::MethodError(name,..)` 改为 `matches!(error, zbus::Error::FDO(...UnknownMethod))` | **接口迁移等价**：仍断言 `UnknownMethod` |
| `crates/inox-launcher/src/view_model.rs`（loading 横幅） | `banner.retry.is_none()` → `banner.action.is_none()`，并新增 `banner.kind` | **重命名 + 增强** |

### 5.3 「主按钮 hover == rest」两条理由的独立核对

- `inox-lock`：**理由成立**。`crates/inox-lock/src/session.rs` 中 grep `wl_seat` / `wl_keyboard` / `wl_pointer` **无命中**；`src/render.rs:194,216` 用 `mouse::Cursor::Unavailable` 且 `render.rs` 不消费任何 `Message`。指针与键盘都到不了锁表面，`theme.rs:738` 的断言与 `crates/inox-lock/AGENTS.md` 明文一致。
- `inox-greet`：**理由不成立**。greet 是真实 iced 应用（`crates/inox-greet/src/main.rs:70` `iced::application(...).run()`），`src/view.rs:362-366` 在 enabled 时挂 `.on_press`，`theme.rs:245` 把 `button::Status::Hovered` 映射进 `ControlState::Hovered`，并且 `theme.rs:621` 有「按下被立即回答」的测试——**指针输入路径存在且可达**。`theme.rs:310-343` 的 `submit_style` 忽略 `Hovered`，`theme.rs:608` 断言 `idle == hovered`，所以主按钮的悬停**不可见**。这是本次复核发现的唯一一条实质 §5 缺口，见 §6。

## 6. 未达标项与需要 Lead 决策的点

### 6.1 未达标项

1. **P1 `inox-panel` 缺 200% 字号的渲染证据**。`crates/inox-panel/tests/render.rs` 完全不含 `typography.scale`，`after/` 只有 scale 1 的 1280×32 图；200% 只有 `src/view.rs:451` 与 `src/theme.rs:397,674` 的数值断言。§2 要求「面板高度推导与 `spacing.lg` 两处都必须对 200% 缩放给出不裁切证据」，§10 清单也要求 before/after。数值断言比截图更严，但**清单要求的是渲染证据**，需补一张（或明确接受数值断言替代）。
2. **P1 `inox-greet` 主按钮悬停不可见**。`crates/inox-greet/src/theme.rs:310-343` + 测试 `:608`。规范没有为「已经填满 accent 的主按钮」定义 hover 画法（§5 表格只给了 `surface_variant` 背景），代码注释给的理由是「令牌没有 accent 的 hover 变体，`surface_variant` 会让主按钮像禁用」。属**规范缺口 + 实现选择**，需要 Lead 决策：补规范例外，或让主按钮 hover 用 `on_accent` 描边/降低填充对比。
3. **P2 `inox-settings` 自带反字面量自检覆盖偏窄**（`src/view.rs:1526`）。实际代码无字面量（我的独立扫描可证），但该自检挡不住 `radius(6)`、`spacing(12)`、`.size(15)`、`Length::Fixed(200)`。
4. **P2 `inox-launcher` 缺 §4 目标高度断言**（`src/theme.rs:142`）。实测 39/35/55 px 均达标（§4.2），但其余四个窗口型 crate 都有断言，这里没有。
5. **P2 页面留白**：`inox-settings` 卡片下方仍有一大片空白（Displays 页约 230 px，200% 的 Appearance 页更明显），§9 明确把「底部一大片空白」列为反例。

### 6.2 需要 Lead 决策（按优先级）

1. `inox-greet` 主按钮 hover：补规范例外，还是改样式（见 6.1 第 2 条）。
2. `inox-launcher` 的 accent 预算与角色语义：同屏 accent 出现在「选中行填充 + 每行图标 tile + 焦点环」（after 截图实测 ≥4 处），与 §5「`accent` 只留给每屏一个主动作」有张力；搜索框文本用 `TextRole::Title`（`src/view.rs:83`），而 §2 把 title 定义为标题、控件当前值归 body。二者观感都成立，但需要规范一句话定性。另外 `selected` 与 `pressed` 样式刻意同一（`src/theme.rs:370-373`、测试 `:710`），「五态两两可辨」的 pairwise 集里没有 selected。
3. `inox-panel` 的 200% 渲染证据与 tooltip 重叠：`focused-under-pointer` 图里 tooltip 与标签重叠（before/after 皆然）。若要求面板在真实 layer-shell 里正确显示 tooltip，需要宿主侧弹出表面，超出现有证据范围。

### 6.3 其它低优先级观察

- `inox-panel` 长标题条目被按字形切断、没有省略号；按 `crates/inox-panel/AGENTS.md`「标签被截断」可接受。
- `inox-greet` 禁用态字段只剩标签（§4.6 遗留 a）；信息句用 caution 色（遗留 b）。
- `inox-lock` / `inox-greet` 的 crate `AGENTS.md` 写「`theme.rs` 里没有字面量颜色或像素尺寸」，实现里有 `Color::TRANSPARENT` 与 `width: 0.0` 的结构性哨兵；建议把措辞改成「没有设计色/设计尺寸的字面量」。

## 7. 交叉一致性

| 量 | panel | launcher | notifications | settings | lock | greet | 判定 |
|----|-------|----------|---------------|----------|------|-------|------|
| base | `height/2 × scale` | `lg × scale` | `lg × scale` | `lg × scale` | `lg × scale` | `lg × scale` | 符合 §2 两处规则 |
| title/heading/body/caption | 1.5/1.25/1.0/0.875 | 同 | 同 | 同 | 同 | 同 | 完全一致 |
| 卡片内边距 | 条带例外 | `lg` | `lg` | `lg` | `lg` | `lg` | 一致 |
| 卡片圆角 | `radii.sm`（条目） | `md` | `md` | `md` | `md` | `md` | 一致 |
| 焦点环 | `xs` | `max(xs, hairline)` | `xs` | `xs` | `xs` | `xs` | 基本一致（launcher 多一个下限） |
| hairline | `max(xs/2, 0)` | `max(xs/2, 1)` | `xs/2` | `max(xs/2, 0)` | `xs/2` | `xs/2` | 默认令牌下都是 1 px |
| 控件上下内边距 | 条带 `xs` 内缩 | `sm` | `md` | `sm` + `control_height ≥ xl` | `md` | `md` | 机制不同、实测结果一致 |
| 窗口/页面内边距 | — | `xl` | `xl` | `xl` | `xl` | `xl` | 一致 |
| 组内 / 组间间距 | `sm` / `lg` | `sm` / `lg` | `sm` / `lg` | `sm` / `lg` | `sm` / `lg` | `sm` / `lg` | 一致 |
| 主 / 次 / 危险 | 重试用 warning | 主/次（无破坏性动作） | Primary/Secondary/Danger | Primary/Secondary/Danger | 主/次 | 主/次 | 只有存在破坏性动作的 crate 才用 danger，符合 §5 |

分叉判断：launcher 的 hairline 下限（`src/theme.rs:44,48,126`）与 focus ring 下限（`src/theme.rs:138`）是**有注释的特例**，不是缺陷；其余差异都落在 §3/§4 允许的机制选择内。

## 8. 外部阻塞（不属于本次 UI 重做）

- `inox-wallpaper` 被排除：另有**非本任务的并行写入者**正在改 `crates/inox-wallpaper/src/{model,source,apply,app,view_model,main}.rs`、`crates/inox-common/src/paths.rs`、`crates/inox-config/src/{lib,wallpapers}.rs`（「内置壁纸」）。
- mtime 证据（`stat -c '%y %n'`，-0700）：

| 文件 | mtime |
|------|-------|
| `crates/inox-wallpaper/src/model.rs` | 2026-10-17 20:32:37 |
| `crates/inox-wallpaper/src/source.rs` | 2026-10-17 20:37:21 |
| `crates/inox-wallpaper/src/app.rs` | 2026-10-17 20:43:04 |
| `crates/inox-wallpaper/src/main.rs` | 2026-10-17 20:43:56 |
| `crates/inox-wallpaper/src/reload.rs` | 2026-10-17 20:44:36 |
| `crates/inox-wallpaper/src/view_model.rs` | 2026-10-17 20:46:10 |
| `crates/inox-wallpaper/tests/render.rs` | 2026-10-17 20:46:32 |
| `crates/inox-wallpaper/src/apply.rs` | 2026-10-17 20:47:31 |
| `crates/inox-common/src/paths.rs` | 2026-10-17 20:21:28 |
| `crates/inox-config/src/lib.rs` | 2026-10-17 20:24:52 |
| `crates/inox-config/src/wallpapers.rs` | 2026-10-17 20:23:50 |
| `crates/inox-wallpaper/src/view.rs` | 2026-10-17 19:52:39 |
| `crates/inox-wallpaper/src/theme.rs` | 2026-10-17 19:53:40 |
| `target/ui-redesign/inox-wallpaper/after/*.png` | 2026-10-17 19:55 |

- 结论：wallpaper 的 UI 重做产物（`src/view.rs` 19:52、`src/theme.rs` 19:53、截图 19:55）**早于**外部写入者对该 crate 7 个源文件的最后修改（20:32–20:47），其中 `view_model.rs` 同时被两边改动。因此 `target/ui-redesign/inox-wallpaper/after/` 的截图**不再对应当前源码**，其静态只读结论证据强度有限，未纳入本报告的符合性判定。
- 本轮遵守约束：**未**在 `inox-wallpaper` / `inox-common` / `inox-config` 上运行任何 cargo 命令，**未**修改这三个 crate 的任何文件。因此「wallpaper 当前编译不通过」是写入者的报告，**我没有独立复现**（该复现会与外部写入者抢 `target` 锁）。
- 六 crate 全部依赖 `inox-config`：本报告的 1355 条通过记录来自 21:00 前后的工作树快照。若外部写入者此后改动 `inox-config`，本报告结论即失效，必须在 CI 上重跑。

## 9. 「本地自检通过」与「待 CI 验收」的分界

**本地自检通过**（本报告可证的范围）：

- 六 crate `cargo test --all-targets` 全绿（1355 passed / 0 failed）。
- 六 crate `cargo fmt --check` 无 diff；`cargo clippy --all-targets --all-features -D warnings` exit 0。
- `target/ui-redesign/<crate>/after/*.png` 可由当前源码逐字节复现；settings 的 before 快照与 `target/inox-shots/` 逐字节一致。
- `en-US` / `zh-CN` 目录 id 与占位符 6/6 一致（我的独立脚本）。
- 非测试路径无字面量颜色/尺寸（我的独立扫描）。

**待 CI 验收**（本地结果不构成依据）：

- CI 的 `fmt`、`clippy`、`cargo-deny`、`doc`、release 构建与按 crate 分片的测试矩阵。
- `cargo test -p inox-integration --test layering`（分层方向）。
- 根 `AGENTS.md` §6 要求的 waydriver + weston headless 真实截图断言：本报告的像素核对用的是 crate 内的离屏 `iced_tiny_skia` 渲染，**不是**虚拟合成器里的真实窗口，因此对 §7（缩放、键盘可达性）只能给「离屏证据 + 单测」级别结论。
- 人类在真实硬件上的 Q01/Q02 验收（键盘完成核心任务、200% 缩放、高对比主题）。

## 10. 后续建议

1. 给 `inox-panel` 补一个 200% 字号的 render 场景与截图（`tests/render.rs` 里加 `tokens.typography.scale = 2.0` 的用例），或把「数值断言替代截图」写进规范。
2. 裁决 `inox-greet` 主按钮 hover：优先在 `ui-guidelines.md` §5 增加「已填 `accent` 的主按钮如何表示悬停」的显式条款，再改代码；不要为一个按钮新增颜色令牌（§11 禁止）。
3. 把 `inox-settings/src/view.rs:1526` 的自检改成对 `view.rs` 非测试片段做统一「数字字面量」正则（`radius(`、`spacing(`、`Length::Fixed(`、`.size(`、`padding(`），并允许 `0` / `1` 的白名单。
4. 给 `inox-launcher` 补 §4 目标高度断言（例如断言 `content_width` 与控件高度乘积、或在几何测试中断言控件高度 ≥ `spacing.xl`），与其余四个 crate 对齐。
5. 在 `ui-guidelines.md` §5 写明「不可聚焦表面（通知栈、环锁表面）的焦点态例外」，把现在只存在于代码注释里的契约升格为规范条款。
6. `inox-settings` 卡片下方的留白：让内容区 `Fill` 或把状态条上移；这是 §9 唯一点名而未完全消除的反例。
7. 涉及 `inox-config` 的改动（含外部写入者）落地后，在 CI 上重跑本报告引用的六条 `cargo test -p <crate> --all-targets`。

## 11. 未核对项（明确声明）

- `inox-wallpaper` 的符合性与编译状态：受约束未跑该 crate 的 cargo，未纳入判定（见 §8）。
- 真实合成器（weston/wayland headless）下的 waydriver 截图与 AT-SPI 语义树：本机未跑，交给 CI 与人类验收。
- `cargo deny` / `cargo doc` / release 构建：本机未跑，交给 CI。
- 本报告写于未提交的工作树上；工作树在复核期间持续被外部写入者改动（`crates/inox-wallpaper/src/apply.rs` 21:00 前仍在变），故 §5.1 的通过记录有明确时间边界。
