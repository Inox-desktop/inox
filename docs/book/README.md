# Inox 文档站点

Inox 的用户与开发者文档站点（mdBook），源文件位于本目录的 `src/`。

它是 Inox 在**产品代码之外**的统一文档入口：汇总架构说明、组件职责、配置参考与开发约定。站点的权威内容始终是仓库根的 [PRD](https://github.com/Inox-desktop/inox/blob/main/PRD.md)、[docs/adr](https://github.com/Inox-desktop/inox/tree/main/docs/adr) 与根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md)；本站点不复制其正文，只做索引与解释。

## 状态

| 章节 | 内容 |
|------|------|
| 简介 | Inox 是什么、本站点范围、如何阅读、权威来源 |
| 架构 | 分层与依赖方向、组件清单与当前状态、协议矩阵、ADR 流程 |
| 配置 | 配置目录布局、`config.toml` / `compositor.toml` 参考、主题与设计令牌 |
| 开发 | 协作流程、持续集成、测试策略、依赖管理 |
| 使用 | 构建与运行 headless 合成器（并说明哪些能力尚不存在） |
| 附录 | 里程碑状态、术语表 |

关于配置的章节来自 [ADR 0009](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0009-configuration-model-and-reload.md)（状态 `proposed`），页面开头均标注「提案，尚未发布」；描述组件现状时会明确区分「已实现」与「骨架」。

## 构建

```bash
mdbook build docs/book   # 输出到 docs/book/book/
mdbook serve docs/book   # 本地预览
```

CI 的 `book` job 执行同一条 `mdbook build`，构建失败即为红。本目录不含 Rust 代码、没有 `Cargo.toml`，因此不参与工作区依赖图、层级表与 `cargo-deny`。

## 与仓库其余部分的关系

本站点原先是一个独立的 `inox-docs` 仓库，[ADR 0016](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0016-single-repository-cargo-workspace.md) 把全部组件合并为单一 Cargo workspace 时，它随迁到 `docs/book/`，并因此不再是工作区成员（它本来也不参与任何 Rust 依赖关系）。

章节之间用相对路径互链；引用仓库内其他文件时使用绝对 URL（`https://github.com/Inox-desktop/inox/...`）。
