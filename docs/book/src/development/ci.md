# 持续集成

**CI（GitHub Actions）是唯一的构建与测试验证途径。** 本地 `cargo build` / `cargo test` 的结果不作为验收依据（根级 `AGENTS.md` §3.1）。这条规则不是流程偏好，而是项目前提：AI 生成的代码由 CI 判定，人类只审查架构与关键逻辑。

## 单一工作流

`.github/workflows/ci.yml` 是唯一的 Rust 工作流，取代了 ADR 0016 之前每个组件仓库各自一份的 17 份副本（迁移时其中 3 份已经分叉，意味着「同一份模板复制 17 次」并不保证一致性）。

| job | 命令 | 作用 |
|-----|------|------|
| `fmt` | `cargo fmt --all -- --check` | 默认 rustfmt 配置，不允许自定义覆盖 |
| `clippy` | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 零警告；因不链接而不安装系统库 |
| `deny` | `cargo deny check` | 许可证白名单、RUSTSEC 漏洞、重复版本，针对**完整依赖图** |
| `doc` | `cargo doc --workspace --no-deps --all-features` | 配合 `RUSTDOCFLAGS=-D warnings`，缺失文档注释会失败 |
| `build` | `cargo build --workspace --release --all-features` | 发布构建（安装链接所需系统库） |
| `plan` | 由层级表推导测试矩阵 | 覆盖不全或矩阵为空即失败 |
| `test` | `cargo test -p <crate> --all-features`，**每 crate 一个分片** | 失败可归因、可局部复现 |
| `xwayland-e2e` | `INOX_XWAYLAND_E2E=1 cargo test -p inox-xwayland --test headless_x11_mapping` | X11 映射端到端（宿主为第三方 headless 合成器，仅 E2 等级） |
| `book` | `mdbook build docs/book` | 本站点构建 |

触发条件：`push` 到 `main`、`pull_request`。**任一 job 失败即阻止合并**（分支保护的检查名需按新的 job 集合维护 —— 见 ADR 0016 实施记录中「尚未完成、需要人类动作」）。

## 测试分片由层级表推导

`plan` job 读取根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`，产出 `test` job 的矩阵。这样做的原因是**避免静默遗漏**：如果矩阵是手工维护的，新增一个 crate 却忘了加进矩阵，它的测试就永远不会运行，而 CI 仍然全绿。现在矩阵与层级表同源，而层级表又被 `layering.rs` 断言必须覆盖全部工作区成员，因此「新增 crate 未被测试」这条路径不存在。

分片的第二个好处是分片命令与开发者本地命令**逐字一致**：CI 跑的就是 `cargo test -p <crate>`，没有 CI 专有的封装脚本。

## 需要系统库的分片

只有 manifest 中声明了 `smithay` 的 crate 才安装系统库（当前是 `inox-compositor`），其余分片保持廉价。合成器分片需要：

- **系统包**：`libxkbcommon-dev`（Smithay 的 `wayland_frontend` 在链接期需要 `-lxkbcommon`）、`xkb-data`（合成器用它提供的 rules 文件编译确定性的 `evdev`/`us` keymap）、`libegl1` 与 `libgl1-mesa-dri`（EGL 由 Smithay 在运行时 `dlopen`，llvmpipe 软件光栅化来自 Mesa 的 DRI 驱动），以及 M2.5 真实后端的 `libseat-dev` / `libdrm-dev` / `libgbm-dev` / `libinput-dev` / `libudev-dev`（链接需要，因 runner 无 DRM master，只做编译检查）。
- **软件渲染**：工作流级环境变量 `LIBGL_ALWAYS_SOFTWARE=1`。runner 没有 GPU，headless 集成测试通过 llvmpipe 渲染。
- **headless 集成测试**：验证输出创建、渲染（以离屏像素读回断言）、输入事件与干净退出；测试进程需要随时可用的私有 `XDG_RUNTIME_DIR`。

各 crate 的 `AGENTS.md` 记录该 crate 特有的 CI 依赖与命令，以它们为准。

## 分层方向检查

`test (inox-integration)` 分片中的 `layering.rs` 是可执行的分层规则（验收项 B04）：读取真实依赖图，断言 ADR 0002 的依赖方向、`inox-common` 的叶子性、合成器与 Shell 的无依赖边界、L3 访问合成器的单点限制等。它同时带有一组对合成图的**单元测试**，确保规则本身不会在无人察觉时失效。

## 本站点的门禁

本站点不含 Rust 代码、没有 `Cargo.toml`，因此不参与 fmt/clippy/test/deny 流水线（也就是不参与工作区依赖图与 `cargo deny`）。它的门禁是 `book` job 中的 `mdbook build docs/book`：

- `src/SUMMARY.md` 里引用的章节文件必须存在；
- 章节之间的相对链接必须有效；
- Markdown 语法必须能被 mdBook 解析。

此外仓库根目录的 `.github/workflows/docs.yml` 用 `markdownlint-cli2` 检查**全部** Markdown 文件（含各 crate 的 `AGENTS.md` 与 `docs/`），规则配置见 `.markdownlint.json`。
