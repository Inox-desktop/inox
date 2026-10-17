# inox-settings

Inox 的系统设置应用。属于 Inox 桌面环境的 **L3** 层。

## 职责

它是 **唯一写入 Inox 配置的组件**（[ADR 0014](../../docs/adr/0014-configuration-apply-and-migration.md)）。它编辑 `config.toml`、`shell.toml` 与
`notifications.toml` 中用户可改的字段，并把「已保存」与「已生效」当作两件不同的事来呈现。

它**不**是其他组件运行的必要中心：配置的权威 schema、默认值与校验规则都在 L1 的 `inox-config`，本组件只是写入与协调方。

## 状态

已实现（证据等级 **E0/E1** 加离屏渲染，见 [ADR 0018](../../docs/adr/0018-l3-component-architecture.md)）：

- **草稿（`src/draft.rs`）**：用户正在编辑的值叠在文件中的值之上，记录**哪些字段被改过、由谁改的**。
  未被触碰的字段永远不会被写回；改回原值即不再是改动；「取消」不写任何东西；外部重载不会丢掉用户手上的修改。
- **校验（`src/validate.rs`）**：镜像 `inox-config` 的规则（`panel.height` 8–256、三个通知超时 0–600000、
  `locale.language` 必须是合法 BCP-47 且不是 `C`/`POSIX` 哨兵）。报错是
  `FieldError { path, message }`——路径就是 `inox-config` 解析错误里用的点号路径，消息是 Fluent id 加参数。
- **保存（`src/persist.rs`、`src/document.rs`）**：在目标目录内写临时文件、设权限、整份写入、`sync_all`、
  原子 `rename`、再同步目录；从不截断正在使用的文件。写入携带读取时的内容摘要，文件被外部改过时报
  `Conflicted` 而不是覆盖。文档补丁**只重写被改动的键所在的行**，其余行逐字节保留（注释、顺序、
  本组件不建模的键都不受影响），并且**认出 TOML 允许的每一种键写法**——裸键、引号键、点号键、
  内联表成员、引号表头：用户手写的 `panel.height = 12` 或 `panel = { height = 12 }` 会就地改写，
  而不会被写成同一个键的第二遍定义（那样的文件读不回来，会话起不来）。
- **应用（`src/apply.rs`）**：`Draft → Saving → Pending → Applied / SavedOnly / Rejected` 状态机，
  带请求代次；陈旧回执不改动状态；**只有消费方回读确认后才显示「已生效」**；每个组件的结果单独呈现
  （ADR 0014 明确拒绝承诺跨组件全局原子性）。
- **界面（`src/app.rs`、`src/view.rs`、`src/pages/`）**：外观（浅色/深色/跟随会话）、面板、通知、语言四页；
  逐字段错误、禁用原因、Apply / Cancel / Reset 以及 pending/applied/rejected 状态；键盘可达且有可见焦点，
  可达的只有窗口**画为可用**的控件（焦点就画在控件上，落在禁用控件上等于落在空处）；写盘在途时
  没有动作能撤销它，因此 `Cancel` 此时不可用并说明原因。

未实现，且**不声称已实现**：

- **到各组件的传输**。本里程碑没有任何通道能让本组件要求合成器、Shell 或通知守护进程应用新配置
  （ADR 0018）。因此界面把改动如实报告为「已保存、未确认生效」，而不是显示一个无法证实的成功。
- **快捷键页与显示配置页**。验收 D04 提到主题 / 快捷键 / 显示配置；但 `compositor.toml` 目前没有
  快捷键表（`inox_config::InputConfig` 只有键盘布局、重复速率与指针开关），输出列表则是数组表，
  与本文档补丁处理的标量字段不是同一类编辑器。两者都如实登记，而不是半成品。
- **E2/E3 证据**：需要能运行 Inox 会话的宿主。

## 与 layer-shell 的关系

**本组件是普通 `xdg-shell` 窗口，不是 layer surface**，因此不需要放置策略，也不受「合成器尚未广告
`zwlr_layer_shell_v1`」这一阻塞影响（见 [ADR 0018](../../docs/adr/0018-l3-component-architecture.md) 与
`inox-panel` 等四个 layer-shell 客户端）。启动时它会用一条 `tracing::warn!` 说明**缺失的是到各组件的传输**，
而不是表面协议。

## 开发约定

- 本项目遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，不在本地进行编译验证。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)。

## 测试

| 目的 | 命令 |
|------|------|
| 单元与集成测试 | `cargo test -p inox-settings --all-features` |
| 生成界面快照 | `INOX_SCREENSHOT_DIR=/tmp/inox-shots cargo test -p inox-settings --test render` |

`tests/render.rs` 用二进制同款的 tiny-skia 渲染器在 1280×720 下绘制真实控件树并断言像素（验收 Q02）；
`tests/workflow.rs` 在真实临时目录上跑完整的 D04 用户任务，并用 1000 例属性测试断言
「校验通过的草稿写盘后回读，恰好等于窗口显示的值」。设置 `INOX_SCREENSHOT_DIR` 时快照写成
`inox-settings-<场景>.png`，CI 可收集为 artifact。

## 许可证

BSD 3-Clause，见仓库根的 [`LICENSE`](../../LICENSE)。
