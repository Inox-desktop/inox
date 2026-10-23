# AGENTS.md — inox-settings

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-Desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

系统设置应用：Inox 中**唯一写入配置的组件**。所属层级：**L3**。

它编辑 `config.toml`、`compositor.toml`、`shell.toml` 与 `notifications.toml` 中用户可改的字段，按 ADR 0014 区分「已保存」与「已生效」，按 ADR 0013 用快照/代次/命令结果建模与各组件的契约。它不持有 schema、默认值或校验规则的权威——那些都在 L1 的 `inox-config`。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- **`app::update` 不做 I/O。** 一切副作用都作为 `app::OutgoingCommand` 离开更新循环（写文件、请求组件应用、重读配置），由 `main.rs` 执行并把结果作为 `Message` 送回来。这条是状态机可测的原因，也是它与二进制不会各有一份实现的原因；新增副作用时必须走同一条路。
- **唯一动文件的地方是 `persist::save`。** 它必须保持 ADR 0014 要求的失败原子性（目标目录内临时文件、整份写入、`sync_all`、原子 `rename`、目录同步），并且必须保留「外部编辑检测」：写入携带读取时的 `Guard`，摘要不符时返回 `Conflicted` 而不是覆盖。任何新增写入路径都必须同样经过它。
- **只写用户改过的键。** `document::patch` 只重写被编辑键所在的行，其余行逐字节保留；「取消」不排队任何命令。改动 `draft`/`document` 时不得引入整文件重写，也不得缩小 `patch` 认识键的范围（见下方对外契约）。
- **界面不得声称未确认的成功。** 只有 `apply::ApplyState::Applied`（每个消费方的回读都带当前代次）才能显示「已生效」；无法请求的消费方必须是 `SavedOnly`。
- **显示改动是唯一会追问的改动（ADR 0028）**：只有携带 `Field::Outputs` 的请求以 `Configuration2.Trial` 发出（`live::apply` 的 `trial` 参数），且只有合成器回执之后才开始倒计时（`app::State::arm_display_trial`，由 `Message::DisplayTrialArmed` 触发，不是写盘触发）。四条不可放宽的规则：倒计时是 `DISPLAY_TRIAL_SECONDS`（30）秒、一秒一条 `Message::TrialTick`，归零即请求回退；回答只有「确认」与「恢复原设置」两种，试用期间页面控件与其它动作一律禁用并写明 `settings-display-trial-active`，`Esc` 不回答试用；**回退必须恢复文件**——`outputs` 被改回改动前的值，经 `document::patch` + `validate::document` 只重写这一个键，并**作为独立的 `OutgoingCommand::Restore`** 写入（不是一次新的 `Apply`），因为它回答的是试用而不是发出新请求，且不能因为别的消费方还在应答就被阻塞；合成器不可达时仍要恢复文件并如实报告（`settings-display-trial-notice-unreachable` / `-failed`）。`pending_trial` 是「本次请求是试用」的意图，重试（`Application::retry_apply`）沿用同一意图。headless 或合成器不可达时不弹框、只显示 `SavedOnly`。
- **界面不得提供做不到的动作。** 禁用控件必须写明原因（`disabled_reason`），并且不是键盘可达项；写盘在途时 `Cancel` 属于此类。
- **用户可见文案全部走 Fluent**（en-US 与 zh-CN 同步，`tests/catalog.rs` 用 `inox_common::i18n::compare_catalogs` 与 `i18n::message_ids()` 断言齐备）；颜色、尺寸、圆角只能来自 `inox_config::Theme` 令牌。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）：在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次，本 crate 用 `{ workspace = true }` 引用。**新增依赖前先确认依赖方向合法**（根级 §2），否则 `cargo test -p inox-integration --test layering` 会失败。
- **对外契约（不可轻改）**：`Field` 的 `path()` 必须始终是 `inox-config` 解析错误里用的点号路径；`validate` 的规则必须镜像 `inox-config` 而不是自行发明（`tests/workflow.rs` 用真实解析器逐条核对边界）；`apply` 的陈旧回执必须被丢弃。
- **壁纸选项来自 L1 清单，不是本 crate 的发明**：`Field::WallpaperImage` 的选择表（`WALLPAPER_CHOICES`）必须与 `inox_config::wallpapers::BUNDLED` 及 `BACKGROUND_ID` 一致，由单元测试强制；「默认」选项在写盘时**删除 `wallpaper.image` 键**（`toml_value` 返回 `None`），而不是写空字符串；配置里手写的自定义路径不在选项表内，投影时按原值追加并逐字写回（ADR 0030）。
- **`document::patch` 必须认出 TOML 允许的每一种键写法**：裸键、引号键、点号键、内联表成员、引号表头（含 `[[表]]` 形式的表头，按表读以免再造一个同名表）。漏认一种，同一个键就会被写第二遍——那份配置 `inox-config` 读不回来，而窗口还会说它保存成功。删掉任何一种识别路径都必须同时删掉对应的 `document.rs` 单元测试与 `tests/workflow.rs` 的回读用例。
- **键盘只能落在窗口画为可用的控件上**：`view_model::focus_order` 只收 `enabled` 的控件（禁用控件不画焦点，落上去等于键盘落在空处），`app::State::activate` 对已离开该序列的目标不执行任何事。写盘在途时不存在能撤销它的动作：`Cancel` 不可用（写明原因），`app::State::cancel` 也拒绝在途请求——取消不能阻止一次已经在写的保存。
- 本 crate 是**普通 `xdg-shell` 窗口**，不是 layer-shell 客户端，因此不需要放置策略。配置应用传输由二进制 `live.rs` 通过会话 D-Bus 执行，只有当前代次与语义快照都匹配才确认；窗口服务与 layer-shell 的限制仍按 ADR 0018 报告。

## 主要依赖

- 同工作区：`inox-common`（Fluent 目录加载）、`inox-config`（配置模型与主题令牌）。**没有**同层依赖，也**没有**到 `inox-compositor` 的依赖（ADR 0013、0018）。
- 第三方：`iced`（tiny-skia 渲染器）、`thiserror`、`anyhow`（二进制）、`tracing`、`tracing-subscriber`。
- dev：`iced_core`、`iced_runtime`、`iced_tiny_skia`（离屏渲染测试）、`image`（快照）、`proptest`、`tempfile`。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-settings --all-features` |
| 本 crate 构建 | `cargo build -p inox-settings --release --all-features` |
| 本 crate 文档 | `cargo doc -p inox-settings --no-deps --all-features` |
| 静态检查 | `cargo clippy -p inox-settings --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt -p inox-settings` |
| 界面快照 | `INOX_SCREENSHOT_DIR=/tmp/inox-shots cargo test -p inox-settings --test render` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`）：草稿（只写触碰过的字段、取消不写、恢复默认）、校验（含拒绝路径与 Fluent id）、文档补丁（逐字节保留）、原子保存失败矩阵、应用状态机（含陈旧回执）、投影（禁用原因、焦点顺序、逐字段错误）。
- `tests/workflow.rs`：在真实临时目录上跑 D04 的用户任务（应用 / 取消 / 冲突 / 宕机 / 重启 / 重载），以及 **≥1000 例**的属性测试「校验通过的草稿写盘后回读恰好等于窗口显示的值；校验不通过的草稿不写任何文件」；显示试用的三条路径（确认保留、倒计时归零恢复文件、没有可应答的合成器时只保存）必须走真实文件。
- `tests/catalog.rs`：en-US 与 zh-CN 的 id 与占位符一致（Q03）。
- `tests/render.rs`：至少三个 Q02 场景（浅色、深色、错误态），加禁用态、焦点可见性与两次真实点击交互；`INOX_SCREENSHOT_DIR` 设置时写出快照。显示试用弹窗必须有像素场景（覆盖层、倒计时句子、两个回答、焦点在「确认」上），并用一次真实点击和一次纯键盘回答各验证一次。
- 断言失败时必须能指出**哪个字段/哪个文件/哪一代次**，不允许用「有错误即可」代替定位。

## 目录结构

```
crates/inox-settings/
  Cargo.toml    工作区继承：字段一律 *.workspace = true
  src/
    app.rs         State / Message / update（Elm，无 I/O）+ OutgoingCommand
    apply.rs       已保存 vs 已生效的状态机、代次、每组件结果
    document.rs    四个配置文件作为文档、只改被编辑键的 TOML 补丁
    draft.rs       字段注册表、值、草稿叠加与改动记录
    persist.rs     原子写入、内容摘要、外部编辑检测
    validate.rs    镜像 inox-config 的校验与 FieldError
    collections.rs  显示器、输入法 argv、终端 argv、分屏壁纸的结构化编辑与转换
    focus.rs        控件真实布局边界、输入焦点与自动滚动
    pages/         十个页面：description + view builder
    view_model.rs  状态 → 界面所画内容的投影
    view.rs        iced 控件与布局几何
    theme.rs       令牌 → iced 主题与样式（ADR 0018 允许的每组件一份）
    i18n.rs        Fluent 目录装配
    live.rs        受影响消费方选择、保存后冲突检测与 D-Bus 回读
    bin/inoxctl.rs 手工配置重载入口
    main.rs        日志、配置、目录，以及执行 OutgoingCommand 的运行时
  i18n/en-US/settings.ftl, i18n/zh-CN/settings.ftl
  tests/          render.rs, workflow.rs, catalog.rs, support/mod.rs
  README.md     本 crate 说明
  AGENTS.md     本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `settings`。
