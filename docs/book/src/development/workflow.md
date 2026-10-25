# 协作流程

Inox 采用 **AI-first** 工作流：AI 编写绝大部分代码、测试与文档，人类负责架构决策、PR 审查与真实硬件验收。组织级说明见 [CONTRIBUTING.md](https://github.com/Inox-desktop/.github/blob/main/CONTRIBUTING.md)。

## 硬性规则

根仓库的[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查) 是对所有 crate 生效的硬性规则，crate 级 `AGENTS.md` 只能收紧、不得放宽。其中最影响日常改动节奏的几条：

- **不在本地编译/测试来当验收依据** —— 验收一律以 CI 为准（见[持续集成](./ci.md)）。
- 默认禁止 `unsafe`；只有 `inox-compositor` 与 `inox-xwayland` 的最小 FFI 胶水代码可例外，并必须附 `// SAFETY:` 理由。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- 库用 `thiserror`、可执行程序用 `anyhow`；禁止吞掉错误。
- 依赖许可证白名单：宽松许可 + LGPL；项目本身 LGPL-3.0；禁止 AGPL / SSPL 与 GPL-only 的 Qt 模块进入核心依赖。
- 用户可见文案走 Project Fluent，禁止硬编码；组件只消费设计令牌，禁止硬编码颜色与尺寸。
- 面向用户的文档与 PRD 用中文；代码注释与标识符用英文。

## 一次改动的完整流程

1. **读约束**：根级 `AGENTS.md` + 目标 crate 的 `AGENTS.md` + 相关的 PRD 章节与 ADR。
2. **判断是否需要 ADR**：新增/删除组件 crate、改变分层或依赖方向、引入新协议策略、更换核心依赖（Smithay / QML + Qt Quick 与 cxx-qt / calloop）、配置或 IPC 格式的破坏性变更，都**必须先写 ADR**（见[架构决策记录](../architecture/decisions.md)）。
3. **在功能分支上改动**：只动必要的文件，不顺手重构无关代码。
4. **同步补齐测试与文档**：新增或修改行为必须同时新增/更新测试；用户可见文案必须同步 `en-US`。
5. **本地只做轻量自检**（可选）：`cargo fmt`、`cargo check`、`cargo clippy`。这些结果**不作为验收依据**；环境没有工具链时直接跳过。
6. **推送并创建 PR**：描述填写 PR 模板的全部小节。
7. **读 CI 日志并修复**：反复推送直到全绿；无法解决时在 PR 留言并请求人类介入。
8. **等待人类审查与合并**：AI／代理**不自行合并到 `main`**，不使用 `--no-verify`，不修改分支保护。

## 分支与提交

- `main` 是保护分支，**禁止直接推送**。功能分支命名：`feat/xxx`、`fix/xxx`、`chore/xxx`、`docs/xxx`。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)：

  ```text
  <type>(<scope>): <subject>
  ```

  `type` ∈ `feat` `fix` `docs` `refactor` `perf` `test` `build` `ci` `chore` `revert`；`scope` 使用组件短名（如 `compositor`、`config`、`docs`）。
- 破坏性变更在 subject 后加 `!`，并在 `BREAKING CHANGE:` footer 说明。
- 一个 PR 聚焦一件事。

示例：

```text
feat(compositor): 支持 headless 后端创建输出
fix(config): 热重载失败时保留旧配置
docs(docs): 补齐配置参考页
```

## PR 要求

PR 描述按[组织级模板](https://github.com/Inox-desktop/.github/blob/main/PULL_REQUEST_TEMPLATE.md)填写，包含六节：

| 小节 | 内容 |
|------|------|
| 动机 | 为什么需要这个改动；关联的 Issue / PRD 章节 / 用户场景 |
| 改动 | 具体改了什么 |
| 受影响的 crate | 本次改动涉及哪些 crate、是否需要新增依赖（新增依赖要先确认依赖方向合法） |
| 测试方式 | 新增/修改了哪些测试；本地做过哪些轻量自检（并注明验收以 CI 为准） |
| 性能影响 | 对渲染帧率、启动时间、内存占用的影响；没有就写「无」 |
| 关联 ADR | 涉及架构 / 协议 / 依赖方向变更时链接对应 ADR，否则写「不适用」 |

合并门槛：**CI 全绿 + 至少 1 名人类审查通过 + 分支保护检查通过**。由 AI 修复 CI 失败时，需要在 PR 中追加说明改动了什么。

## 人类的不可委派职责

PRD §5.5 规定以下工作只能由人类完成：

- 架构合理性与边界划分的最终判断；
- 真实硬件上的功能验收（GPU 兼容、输入延迟、多显示器、XWayland 应用）；
- 许可证 / 商标 / 合规决策；
- 合并批准。

CI 无法替代真实硬件测试：PRD §6.4 要求人类定期在真实设备上运行最新构建，验收结果以 Issue 形式记录。

## 文档改动的提交约定

本站点不含 Rust 代码，提交时 scope 固定使用 `docs`。
