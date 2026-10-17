# inox-shell

Inox 桌面环境的 **Shell 主框架**：入口之间的导航规则，以及各 L3 入口共同遵循的服务状态契约。属于 **L3**。

本 crate **不绘制任何内容**。面板、启动器、通知与设置都是各自绘制界面的独立进程；本 crate 提供的是它们共同遵循的规则，以及一个把这些规则变成可测事实的协调器二进制。

## 现在有什么

| 模块 | 内容 | 验收 |
|------|------|------|
| `src/navigation.rs` | 入口栈：每层记录打开时所处的窗口（服务端窗口 ID + 会话代次），`Esc` 每次只关一层并给出焦点应回到哪里；普通展示不夺焦点，键盘导航进入已打开的入口时夺取唯一焦点；窗口已销毁或会话已重建时不向失效对象投递 | D01 |
| `src/contract.rs` | 跨进程状态契约：带代次的快照与客户端视图、`Sent`/`Accepted`/`Applied`/`Rejected`/`TimedOut` 的区分、请求代次（旧结果不得覆盖新请求）、服务状态与有界重试 | D05 |
| `src/registry.rs` | 内部入口注册表：四个入口的二进制名、读取的配置文件、是否必需，以及会话持有的状态；未知名称、重复登记与逃出配置目录的路径在构造期拒绝 | C01 |
| `src/diagnostic.rs` | 可读原因：只携带 Fluent 消息 ID 与占位符，不硬编码句子 | Q03 |
| `src/i18n.rs` | 随 crate 编译进来的 `en-US` / `zh-CN` 目录，以及 `id::ALL` 登记的全部消息 ID | Q03 |
| `src/main.rs` | 协调器二进制：加载配置、按会话语言装配文案、解析注册表并报告，退出 0 | — |

公开 API 见 `cargo doc -p inox-shell`；状态机与契约的规则写在各模块文档，验收场景在 `tests/`。

## 证据等级（ADR 0018）

- **E0/E1 已交付**：`fmt`、`clippy -D warnings`、`cargo doc -D warnings`、`cargo test -p inox-shell` 全绿；纯逻辑有单元测试与 `proptest`（每项 ≥1000 样本：导航状态机、重试有界性、请求代次、快照单调性）；`en-US`/`zh-CN` 目录经 `inox_common::i18n::compare_catalogs` 断言 ID 与占位符一致。
- **E2/E3 未交付**：本 crate 不创建 Wayland 表面，因此没有截图测试与 GUI 驱动测试——这一点是刻意的，不是遗漏。需要真实界面的验收（Q01 语义树、Q02 截图）属于各入口组件，不属于这里。
- **被阻塞**：Inox 合成器尚未广告 `wlr-layer-shell`，因此四个需要 layer 表面的入口（面板、启动器、通知、壁纸）暂时无法成为桌面表面。放置策略（锚点、独占区、边距、尺寸、键盘交互、按输出分配）属于这些持有表面的组件，不属于本 crate（ADR 0018：`inox-shell` 自身不持有 layer 表面）。协调器在启动时用一条 `warn!` 说明这一点并点名 ADR 0018，而不是假装已生效。

## 与「插件系统」的关系

PRD §3.1 把「插件系统」列为本 crate 的职责。ADR 0013 明确把通用第三方插件平台**推迟**到自己的 ADR：加载他人代码涉及权限、生命周期与兼容性，不能顺手决定。当前实现的是**内部入口注册表**（`src/registry.rs`），没有动态加载、没有路径搜索、也不接受第三方清单。

## 开发约定

- 本项目遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，本地结果不作为验收依据。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)。

## 许可证

BSD 3-Clause，见仓库根的 [`LICENSE`](../../LICENSE)。
