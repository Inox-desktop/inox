# 持续集成

**CI（GitHub Actions）是唯一的构建与测试验证途径。** 本地 `cargo build` / `cargo test` 的结果不作为验收依据（根级 `AGENTS.md` §3.1）。这条规则不是流程偏好，而是项目前提：AI 生成的代码由 CI 判定，人类只审查架构与关键逻辑。

## 工作流分工

`.github/workflows/ci.yml` 是唯一在 PR 与 `main` 上判定改动的 Rust 工作流，取代了单仓工作区之前每个组件仓库各自一份的 17 份副本（迁移时其中 3 份已经分叉，意味着「同一份模板复制 17 次」并不保证一致性）。

另外两个工作流不参与合并判定：`.github/workflows/docs.yml` 只跑 markdownlint，`.github/workflows/release.yml` 只在 `v*` tag 或手工触发时产出并发布金丝雀制品（见[发布制品 job 与 Release 工作流](#发布制品-job-与-release-工作流)）。

| job | 命令 | 作用 |
|-----|------|------|
| `fmt` | `cargo fmt --all -- --check` | 默认 rustfmt 配置，不允许自定义覆盖 |
| `clippy` | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 零警告；不链接最终二进制，但依赖的构建脚本仍需要系统库（`libudev.pc` 等），因此与 `build`/`test` 一样安装 |
| `deny` | `cargo deny check` | 许可证白名单、RUSTSEC 漏洞、重复版本，针对**完整依赖图** |
| `doc` | `cargo doc --workspace --no-deps --all-features` | 配合 `RUSTDOCFLAGS=-D warnings`，缺失文档注释会失败 |
| `build` | `cargo build --workspace --release --all-features` | 证明发布构建能编译（安装链接所需系统库）。**它不上传任何产物**：可下载的二进制由下面的 `package` job 产出 |
| `plan` | 由层级表推导测试矩阵 | 覆盖不全或矩阵为空即失败 |
| `test` | `cargo test -p <crate> --all-features`，**每 crate 一个分片**；作业级 `QT_QPA_PLATFORM=offscreen` | 失败可归因、可局部复现；QML 分片另装 Qt 6 与字体 |
| `xwayland-e2e` | `INOX_XWAYLAND_E2E=1 cargo test -p inox-xwayland --test headless_x11_mapping` | X11 映射端到端（宿主为第三方 headless 合成器，仅 E2 等级） |
| `m3-e2e` | 先 `cargo build --workspace --features inox-compositor/test-input`，再 `INOX_M3_E2E=1 cargo test -p inox-integration --all-features` | M3 的 E3 证据：在 **Inox 自己的合成器**上驱动 X11/剪贴板/拖放用例（第三方宿主不算，见[验收规范 §1](https://github.com/Inox-desktop/inox/blob/main/docs/quality/acceptance.md)） |
| `book` | 先 `scripts/check-docs.py`，再 `mdbook build docs/book` | 本站点的结构、链接与构建（见[本站点的门禁](#本站点的门禁)） |
| `package` | `packaging/package.sh --output-dir target/dist` | 产出金丝雀 tarball 并作为 `inox-canary-tarball` artifact 上传（见[发布制品 job 与 Release 工作流](#发布制品-job-与-release-工作流)） |

上表只列与「怎么判定一次改动」直接相关的 job。另有四个专项 job 不在表内：`msrv`（用 Rust 1.89 跑 `cargo check --locked --workspace --all-targets --all-features`，守住工作区 MSRV）、`session-e2e`（L4 无头会话的启动、会话锁与限时注销）、`input-method-e2e`（Fcitx5 真实拼音预编辑与中文上屏）与 `license-audit`（用 `ldd` 断言只链接已声明的 LGPL Qt 模块，不得出现 GPL-only 模块）。

触发条件：`push` 到 `main`、`pull_request`。**任一 job 失败即阻止合并**。

## 测试分片由层级表推导

`plan` job 读取根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`，产出 `test` job 的矩阵。这样做的原因是**避免静默遗漏**：如果矩阵是手工维护的，新增一个 crate 却忘了加进矩阵，它的测试就永远不会运行，而 CI 仍然全绿。现在矩阵与层级表同源，而层级表又被 `layering.rs` 断言必须覆盖全部工作区成员，因此「新增 crate 未被测试」这条路径不存在。

分片的第二个好处是分片命令与开发者本地命令**逐字一致**：CI 跑的就是 `cargo test -p <crate>`，没有 CI 专有的封装脚本。

## 需要系统库的分片

分片按 manifest 声明决定装什么，而不是靠手工名单：声明了 `smithay` 的分片装图形依赖，声明了 `cxx-qt` 的分片装 Qt 6 开发包与字体，声明了 `pam` 的分片装 `libclang`（`pam-sys` 的构建脚本用 `bindgen`）。其余分片保持廉价。

- **合成器分片（`graphics`）**：
  - **系统包**：`libxkbcommon-dev`（Smithay 的 `wayland_frontend` 在链接期需要 `-lxkbcommon`）、`xkb-data`（合成器用它提供的 rules 文件编译确定性的 `evdev`/`us` keymap）、`libegl1` 与 `libgl1-mesa-dri`（EGL 由 Smithay 在运行时 `dlopen`，llvmpipe 软件光栅化来自 Mesa 的 DRI 驱动），以及 M2.5 真实后端的 `libseat-dev` / `libdrm-dev` / `libgbm-dev` / `libinput-dev` / `libudev-dev`（链接需要，因 runner 无 DRM master，只做编译检查）。
  - **软件渲染**：工作流级环境变量 `LIBGL_ALWAYS_SOFTWARE=1`。runner 没有 GPU，headless 集成测试通过 llvmpipe 渲染。
  - **headless 集成测试**：验证输出创建、渲染（以离屏像素读回断言）、输入事件与干净退出；测试进程需要随时可用的私有 `XDG_RUNTIME_DIR`。
- **QML 分片（`ui` / `qt`）**：`qt6-base-dev`、`qt6-declarative-dev`、`qml6-module-qtquick*` 一组运行时模块、`qt6-wayland-dev` 与 `lld`，外加 `fonts-dejavu-core`、`fonts-noto-cjk`。原因有两个：`cxx-qt` 的**构建脚本**要能解析到 Qt（只做类型检查的 job 也一样），而 QML 加载测试要在运行时 import 真实模块、Qt 还要用系统字体排版。作业级环境变量 `QT_QPA_PLATFORM=offscreen` 让这些测试在无显示环境下建 `QQuickWindow` 并取帧。包清单只在 `.github/actions/system-deps` 里维护一份，细节见[依赖管理](./dependencies.md)的「系统依赖：Qt 6」一节。

各 crate 的 `AGENTS.md` 记录该 crate 特有的 CI 依赖与命令，以它们为准。

## M3 端到端（E3）分片

`m3-e2e` 是唯一在 **Inox 合成器自身**上跑的端到端 job：它先按 `--features inox-compositor/test-input` 构建工作区二进制，再运行 `inox-integration` 的 M3 用例。因此它需要三份额外的东西 —— 合成器的软件渲染依赖（与 `test` 分片相同，故复用 `system-deps` action 并开启 `egl`）、`xwayland`/`x11-apps`/`x11-utils`（`xeyes` 与 `xwininfo`）、以及构建 `xwayland-satellite` 所需的 `clang`/`libxcb*`。

`test-input` 是**非默认** feature，只供该 job 使用：它给合成器加一个测试专用输入注入套接字（默认构建不含、默认运行关闭、不是 Wayland 全局），M3 的用例靠它切焦点、向 X11 窗口输入并进行指针拖放。用例以子进程方式驱动二进制，因此 `inox-integration` **不新增任何 Cargo 依赖边**，分片矩阵与 `graphics` 判定都不受影响。未设置 `INOX_M3_E2E` 时这些用例打印跳过并返回，所以常规的 `test (inox-integration)` 分片保持廉价、也不会因缺少 X11 工具而变红。

## 分层方向检查

`test (inox-integration)` 分片中的 `layering.rs` 是可执行的分层规则（验收项 B04）：读取真实依赖图，断言分层规则的依赖方向、`inox-common` 的叶子性、合成器与 Shell 的无依赖边界、L3 访问合成器的单点限制等。它同时带有一组对合成图的**单元测试**，确保规则本身不会在无人察觉时失效。

## 本站点的门禁

本站点不含 Rust 代码、没有 `Cargo.toml`，因此不参与 fmt/clippy/test/deny 流水线（也就是不参与工作区依赖图与 `cargo deny`）。它的门禁在 `book` job 里，由两步组成：先 `scripts/check-docs.py`，再 `mdbook build docs/book`。

`mdbook build` **单独**是一个弱门禁：`SUMMARY.md` 里列了一个不存在的章节时，它照样以 0 退出，只生成一个只有标题的占位页。这条路径会以 404 的形式到达已发布的站点。`scripts/check-docs.py` 补的就是这一块：

- `src/SUMMARY.md` 里引用的章节必须存在，且 `src/` 下每个 Markdown 页面都要能从 `SUMMARY.md` 到达（存在但未登记的页面在站点上是看不见的）；
- `docs/` 下的相对链接与图片目标必须能解析（网络链接与页内 `#锚点` 跳过，因此这个检查可以离线跑）；
- 之后由 `mdbook build docs/book` 确认 Markdown 语法能被 mdBook 解析。

风格不在这两步里：仓库根目录的 `.github/workflows/docs.yml` 用 `markdownlint-cli2` 检查**全部** Markdown 文件（含各 crate 的 `AGENTS.md` 与 `docs/`），规则配置见 `.markdownlint.json`。三件事分别由三个工具负责，不要让其中一个的绿灯替另一个背书。

## 发布制品 job 与 Release 工作流

`package (canary tarball)` job 在**每个 PR 与每次 push 到 `main`** 上产出一个可下载的金丝雀 tarball，并作为 artifact `inox-canary-tarball` 保留 30 天。它存在的理由是验收规范把 E4 的证据固定为「CI 产出的二进制」：只在一个发布日才被执行的打包脚本等于没被验证过，因此这条路必须由每一次 CI 运行承担。

它依次做四件事：

1. 对所有 `packaging/*.sh` 与 `scripts/*.sh` 跑 `bash -n`，对 `scripts/*.py` 跑 `python3 -m py_compile`，并跑 `package.sh --help`、`install.sh --help`、`uninstall.sh --help`。安装面与辅助脚本是仓库里没有任何别的 job 编译或测试的部分，语法错误必须在 CI 上暴露，而不是等到某人的机器处于半安装状态时。注意这**只是语法与入口检查**：`install.sh` 与 `preflight.sh` 需要 root，CI 不执行它们；
2. `packaging/package.sh --output-dir target/dist` —— 用**默认特性**构建（不是 `--all-features`）、暂存 14 个二进制与 `data/wallpapers`、`packaging/`，写 `BUILD-INFO` 与 `SHA256SUMS`；
3. 校验 `.sha256`、把 `BUILD-INFO` 从压缩包里读出来，并断言压缩包只有一个顶层目录、含全部 14 个二进制、壁纸与 `packaging/install.sh`；
4. 上传 artifact。

第 2 步里有一条**只属于产物**的断言：`package.sh` 会真的运行暂存出来的 `bin/inox-compositor --test-input <path>`，要求该选项被拒绝、且错误信息点名 `test-input`。被接受或进程挂住就让打包失败。`--all-features` 会把合成器的 `test-input` feature（一个接受注入键盘与指针事件的套接字）编译进二进制，这条断言让那种构建无法静默出厂；单元测试覆盖不了它，因为被测性质属于制品而不属于源码树。

`.github/workflows/release.yml` 是**另一个**工作流，负责把同一个 tarball 挂到 GitHub Release 上：

- 触发条件是 `v*` tag 的 push，以及手工 `workflow_dispatch`（输入 `version`）；
- 发布版本必须以根 `Cargo.toml` 的工作区版本开头（`0.1.1-canary.2` 可以，清单还写着 `0.1.1` 时的 `0.2.0-canary.1` 会被拒绝）：否则 `BUILD-INFO`、文件名与 release 标题会一起描述一个没人能复现的构建；
- 版本里含 `-canary`、`-alpha`、`-beta`、`-rc`、`-dev` 时标为 **prerelease**；
- 创建 Release 前断言 `BUILD-INFO` 的 `commit` 就是正在发布的那个提交；
- 手工 dispatch **只上传 workflow artifact，不创建 Release**。

制品怎么拿、怎么装、怎么切一个金丝雀、以及它**不能声明什么**，见[发布制品与金丝雀渠道](./release.md)。
