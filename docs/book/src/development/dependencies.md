# 依赖管理

Inox 是**单一仓库（Cargo workspace）**：全部组件是 `crates/` 下的工作区成员，组件之间用 path 依赖连接，版本只在一处维护。规则见根级 `AGENTS.md` §8 与 PRD §3.2，决策见 [ADR 0016](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0016-single-repository-cargo-workspace.md)。

## 工作区内的依赖

Inox crate 之间一律使用 **path 依赖**，并在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次：

```toml
# 根 Cargo.toml
[workspace.dependencies]
inox-common = { path = "crates/inox-common", version = "0.1" }

# crates/inox-config/Cargo.toml
[dependencies]
inox-common.workspace = true
```

两条约束有实际原因：

- **必须带显式 `version`**：`deny.toml` 中 `[bans] wildcards = "deny"` 会拒绝通配符依赖，而没有 `version` 的 path 依赖会带一个隐式的 `*` 版本要求，从而被判为通配符依赖。
- **只在一处声明**：成员不得各自绑定版本号。版本升级是改根清单一行的事，因此不存在「同一个 crate 在不同组件里被解析成两个版本」的可能。

**不再对 Inox crate 使用 git 依赖或 `tag` 锁定。** 旧的多仓库方案以此传递跨仓库改动，代价是每次上游改动都要「合 PR → 等 CI → 打 tag → 逐个下游改 tag → 各自再等 CI」；工作区内的 path 依赖让跨层改动在一个 PR、一次 CI 内完成。相应地，`deny.toml` 的 `[sources] allow-git` 条目也随之删除——工作区内没有任何 git 来源，`unknown-git = "deny"` 现在是纯限制。

## 依赖方向

组件之间的依赖方向由 [ADR 0002](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0002-layering-and-dependency-direction.md) 规定（只能向下、同层禁止、合成器与 Shell 解耦、`inox-common` 为叶子），并由 CI 机械强制而非仅靠约定：

```text
cargo test -p inox-integration --test layering
```

该检查读取 `cargo metadata` 的真实依赖图，与根 `Cargo.toml` 的层级表比对，失败时给出具体的边与被违反的规则条目。**新增依赖前先确认方向合法**，否则 CI 会直接变红。

## 工作区外部的依赖

工作区外部的依赖仍必须锁定 `tag` 或 `rev`，禁止未锁定的 `branch = "main"`：上游 `main` 的任意一次提交都会改变构建结果，CI 会因此随机失败，且失败与本次改动无关。目前唯一以这种方式引入的外部依赖是 `inox-xwayland` 在 CI 中 `cargo install` 的 `xwayland-satellite`（锁定 `tag v0.8.3`）。

## 组件兼容性记录

兼容性记录就是工作区唯一的 [`Cargo.lock`](https://github.com/Inox-desktop/inox/blob/main/Cargo.lock)，也是验收项 L01 的不可变清单来源。

旧方案有两处人工环节：`compat.toml` 依赖矩阵，以及「发布 tag」与「登记矩阵」两个必须成对完成、否则集成测试对不上版本的动作。两者都已随 ADR 0016 删除——`Cargo.lock` 由构建系统自己产出，不可能与其声称的组件版本不一致。迁移前 `compat.toml` 已经与实际依赖的 tag 不符（登记 `v0.1.0`，而 `inox-config` 依赖 `v0.2.0`），这一点正是删掉它的直接原因。

组件仓库此前打过的 `v0.1.0` / `v0.1.1` 标签保留在待归档的旧仓库中，不重写历史。工作区统一版本为 **`0.1.1`**，与 `inox-common` 当前的发布线一致（0.x 阶段兼容变更走 patch，见 ADR 0010 版本号语义与发布流程，状态 `proposed`）；`v0.2.0` 已被重切为 `v0.1.1` 且不再使用。

## 新增第三方依赖

- 新增依赖需在 PR 描述中说明用途与许可证。
- 优先选择生态中被广泛使用的 crate。
- 只在**首个真实需要它的时刻**引入，不为「以后可能要用」提前声明抽象或依赖（ADR 0004 的排期原则）。
- 版本同样只在 `[workspace.dependencies]` 声明；成员只追加自己需要的 feature。
- 不随意升级 major 版本；升级需跑通 CI 并说明破坏性影响。

### 许可证白名单

仅允许 BSD / MIT / Apache-2.0 / ISC / Zlib 等宽松许可证；**禁止 GPL / AGPL / SSPL 进入核心依赖**。由单一 `cargo deny check` 强制，同时检查 RUSTSEC 漏洞与重复版本。

**检查范围是完整依赖图**，而不是旧方案中 18 份 `deny.toml` 各自看到的子图；因此迁移后可能出现此前被分割掩盖的真实告警，这类告警应逐条处理，不得靠放宽白名单消除。

## MSRV

MSRV 由根 `Cargo.toml` 的 `[workspace.package] rust-version` 统一给出，当前为 **1.89**；成员一律 `rust-version.workspace = true`，不再各自声明。CI 跟随 `stable`，声明的下限只约束使用者。

当前下限来自 `zbus` 6.0 的 Rust 1.89 要求（[ADR 0027](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0027-zbus-6-migration.md)）。此次从 1.88 提升到 1.89，不再支持 1.88 工具链；Iced/image 引入时曾把工作区从 1.86 提升到 1.88。成员仍统一继承根声明，CI 除 stable 全量检查外增加 Rust 1.89 静态检查。

zbus 6 将停止维护的 zvariant、zvariant_derive、zbus_names 合并进自身，不再使用旧 blocking API。通知传输线程与配置重载工作线程使用 `zbus::block_on` 驱动异步 API；数据类型改为 `Variant` / `OwnedVariant`，标准 D-Bus 错误按 `Error::FDO` 分类。总线接口与用户配置格式不变，许可证仍为 MIT，完整依赖检查仍由 CI 执行。

## L3 的第三方依赖

L3 组件（面板、启动器、通知、设置、锁屏、壁纸）在 L1/L2 之外引入下列依赖，全部在 `[workspace.dependencies]` 声明一次：

| crate | 许可证 | 用途 | 作用范围 |
|-------|--------|------|----------|
| `iced` 0.14 | MIT | GUI 工具包（ADR 0003 已固定），`default-features = false`，只启用 `tiny-skia`、`wayland`、`thread-pool`、`crisp`、`image` | 六个有界面的组件 |
| `fluent-bundle` / `fluent-syntax` 0.16 / 0.12 | Apache-2.0 OR MIT | 用户可见文案的加载与解析（PRD §11.2） | `inox-common::i18n` |
| `unic-langid` 0.9 | MIT OR Apache-2.0 | BCP-47 语言标签 | 同上 |
| `chrono` 0.4 | MIT OR Apache-2.0 | 面板时钟的民用时间 | `inox-panel` |
| `image` 0.25 | MIT OR Apache-2.0 | 壁纸的 PNG/JPEG 解码（只启用这两种格式） | `inox-wallpaper`、渲染测试 |
| `zbus` 6.0 | MIT | 通知总线与配置重载；显式启用 default-rt/proxy/service/p2p/tracing | 通知、设置与配置消费方 |
| `zeroize` 1 | MIT OR Apache-2.0 | 凭据缓冲在释放时清零（ADR 0015） | `inox-lock` |
| `iced_core` / `iced_runtime` / `iced_tiny_skia` 0.14 | MIT | **仅 dev-dependency**：离屏渲染测试装置 | 各 L3 组件的 `tests/` |

两个取舍值得记录：

- **不启用 `wgpu`。** `tiny-skia` 是纯软件渲染，无 GPU 的 CI 与 llvmpipe 环境都能呈现，而 `wgpu` 会引入一个没有任何 Inox 配置会选中的第二渲染器。软件渲染是 CI 与回退路径，不是性能验收基准（PRD §8）。
- **没有 `iced_layershell`。** 面板、启动器、通知与壁纸按 PRD §2.3 需要 layer-shell 表面，但截至本次实现，`iced_layershell` 0.19.1 与 0.19.0 **都无法编译**：其 `iced_exdevtools` 依赖对 `winit-core` 0.31.0-beta.3 的 `non_exhaustive` 枚举 `NativeKeyCode` 做了穷尽匹配。因此本次交付把 layer-shell 的**放置策略**建模为纯数据并测试，界面以普通窗口运行，表面协议腿留待上游修复或另择宿主方案（[ADR 0018](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0018-l3-component-architecture.md)）。

## 本站点

本站点位于 `docs/book/`，**没有第三方依赖**：它不含 Rust 代码、没有 `Cargo.toml`，只使用 mdBook 构建，因此不参与依赖图、层级表与 `cargo-deny` 检查，也不引入任何 mdBook 插件。
