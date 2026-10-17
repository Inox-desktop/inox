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

MSRV 由根 `Cargo.toml` 的 `[workspace.package] rust-version` 统一给出，当前为 **1.86**；成员一律 `rust-version.workspace = true`，不再各自声明。

1.86 来自依赖本身的硬要求，而不是偏好：`inox-compositor` 经由 `smithay` 0.7 依赖的 `calloop` 0.14 要求 rustc ≥ 1.86。旧方案中「`inox-compositor` 是 1.86、其余 L1 是 1.85」的分裂状态已随工作区统一而消失；根级 `AGENTS.md` §8 要求提升 MSRV 时说明破坏性影响。

## 本站点

本站点位于 `docs/book/`，**没有第三方依赖**：它不含 Rust 代码、没有 `Cargo.toml`，只使用 mdBook 构建，因此不参与依赖图、层级表与 `cargo-deny` 检查，也不引入任何 mdBook 插件。
