# AGENTS.md — Inox 文档站点（docs/book）

本目录遵循根仓库的**全部硬性规则**（[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)），本文件只补充本站点特有内容，不放宽任何硬性规则。

## 职责

用户与开发者文档站点（mdBook）。位于 `docs/book/`，**不是**工作区成员。

## 目录特有约束

- 本目录**不含 Rust 代码、无 `Cargo.toml`**：因此 `unsafe` / `unwrap()` / `tracing` 等 Rust 相关规则在此不适用，本目录也不参与依赖图、`[workspace.metadata.inox.layers]` 层级表与 `cargo-deny` 检查。新增 crate 的层级登记**不涉及**本目录。
- 文档语言使用中文（面向用户的文档与 PRD 用中文）；命令、标识符与代码片段保持英文原样。
- 保持改动聚焦：只维护文档内容与 mdBook 配置，不引入构建脚本或额外工具链。
- 新增或修改文档后，必须保证 `mdbook build docs/book` 可通过（CI 以构建失败为红）。
- 引用仓库内其他文件时使用绝对 URL；章节之间用相对路径互链。
- **描述组件现状必须可在 `crates/<name>/` 中核对**：尚未实现的组件要明确标注为「骨架」「计划中」或「尚未实现」，不把规划中的能力写成已有能力。

## 主要依赖

无第三方依赖；仅使用 mdBook 构建文档。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

| 目的 | 命令 |
|------|------|
| 构建文档 | `mdbook build docs/book` |
| 本地预览 | `mdbook serve docs/book` |

## 测试要求

- 本站点无单元测试要求，文档正确性由 CI 的 `book` job（`mdbook build`）保证。
- 章节之间的链接必须有效，`src/SUMMARY.md` 必须与实际章节文件一致。
- Markdown 另由仓库根的 `.github/workflows/docs.yml` 用 `markdownlint-cli2` 检查。

## 目录结构

```
docs/book/
  book.toml       mdBook 配置
  README.md       站点说明与构建方式
  AGENTS.md       本文件
  src/
    SUMMARY.md          章节目录（必须与实际章节文件保持一致）
    introduction.md     简介与权威来源索引
    architecture/       分层、组件清单、协议矩阵、ADR 流程
    configuration/      配置目录布局、合成器配置、主题与设计令牌
    development/        协作流程、CI、测试策略、依赖管理
    usage/              构建与运行
    roadmap.md          里程碑与当前状态
    glossary.md         术语表
```

## 提交信息格式

Conventional Commits，scope 使用 `docs`。
