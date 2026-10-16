# ADR 0008: Wayland 协议策略与 inox-ipc（协议绑定集中于 inox-protocols）

- 状态：proposed
- 日期：2026-10-16
- 决策者：架构组

## 背景

根级 `AGENTS.md` §4 规定「新增 Wayland 协议前，必须先在 `inox-protocols` 定义/引用，并说明安全与生命周期约束」，并要求「需要改变……IPC/配置格式的破坏性变更时，必须先写 ADR」；PRD §5.4 也把「引入新协议策略」列为必须先写 ADR 的情形。随着 M1 把 `inox-protocols` 从骨架推进为被 `inox-compositor` 真实消费的 crate（PRD §3.1、§2.2），项目需要一次性固定：**标准协议绑定从何而来、Inox 私有协议如何定义、其版本与兼容策略是什么**。若不作此决策，各组件会各自 `cargo add wayland-protocols`、各自记录版本、各自生成或手写协议代码，导致版本漂移、多份事实来源与不受控的协议面。

现状与约束：

- PRD §3.1 把 `inox-protocols` 定位为 L2「标准协议绑定与自定义 Wayland 协议扩展」crate，主要依赖 `wayland-rs`；`inox-compositor` 依赖它。
- PRD §2.3 的协议支持矩阵是项目对协议面的唯一权威清单，并在表下注明「新增协议必须先在 `inox-protocols` 定义或引用，并说明安全与生命周期约束；破坏性变更需 ADR」。
- PRD §2.2 与根级 `AGENTS.md` §7.2 规定：合成器与 Shell 之间优先使用标准 Wayland 协议或 D-Bus，Inox 私有通信走定义于 `inox-protocols` 的 `inox-ipc`；私有协议为 unstable，破坏性变更需版本化并在协议矩阵更新。
- 根级 `AGENTS.md` §8 要求跨仓库依赖使用 Git 依赖并锁定 `tag` 或 `rev`，且在 `deny.toml` 的 `[sources] allow-git` 中登记来源；升级 MSRV 需说明破坏性影响。
- 根级 `AGENTS.md` §3.2 默认禁止 `unsafe`，`inox-protocols` 不在例外名单内。

考虑过但未被采纳的方案：

- **各组件自行导入 `wayland-protocols` / `wayland-scanner`**：版本漂移无法避免，同一个协议可能在不同组件封装成不同的 Rust 类型，接口无法直接互换；没有单一入口也就没有单一事实来源，协议矩阵与代码会逐渐脱节 —— 不成立。
- **改用现成的第三方 IPC 协议（如 `wlr-foreign-toplevel-management` 或 D-Bus）承载合成器↔Shell 控制**：`wlr-foreign-toplevel` 面向的是通用的窗口列表/激活，无法表达 Inox 专有的配置重载等控制语义；D-Bus 在 PRD §2.2 中已被保留给通知、设置、portal 等**外部服务与跨进程标准接口**，把内部高频控制通道塞进 D-Bus 会混淆边界 —— 不成立。
- **把生成的绑定作为源码提交进仓库（checked-in generated code）**：生成的代码体积大、与 XML 来源无法在 diff 中对齐，且每次升级 scanner 都会产生噪音提交；与「XML 为唯一事实来源」相反 —— 不成立。

## 决策

1. **所有 Wayland 协议绑定集中在 L2 的 `inox-protocols`**。`inox-compositor` 与 Shell/会话组件从该 crate 获取协议类型，**不得**在其他仓库临时引入协议 crate；该 crate 是协议面的唯一入口（PRD §3.1、§2.3）。
2. **标准协议绑定通过再导出（re-export）已锁定的上游 crate 获得**：`wayland-protocols` 0.32（启用 `client` / `server` / `staging` / `unstable`）与 `wayland-protocols-wlr` 0.3，由 `inox-protocols` 统一再导出，使上游版本在一处固定、消费者只面对一个入口。核心 `wl_*` 接口来自 `wayland-client` / `wayland-server`（`wayland-rs`），不重复封装。
3. **Inox 私有协议 `inox-ipc` 在仓库内定义，XML 为唯一事实来源**：源码为 `protocol/inox-ipc.xml`，在编译期由 `wayland-scanner` 生成 client + server 绑定。它承载合成器与 Inox Shell/会话之间的私有控制消息（PRD §2.2、§7.2）。当前 v1 协议面如下：

   | 元素 | 名称 | 说明 |
   |------|------|------|
   | 全局 | `inox_ipc` | 私有 IPC 全局对象 |
   | 请求 | `destroy` | 销毁该 IPC 对象 |
   | 请求 | `get_version` | 查询协议版本 |
   | 请求 | `reload_config` | 请求重新加载配置 |
   | 事件 | `version(uint)` | 告知协商到的协议版本 |
   | 事件 | `config_reloaded(uint enum reload_result)` | 配置重载结果 |
   | 枚举 | `reload_result` | `success` / `rejected` / `io_error` / `unsupported` |

4. **`inox-ipc` 在 Inox 1.0 之前为 unstable**：客户端必须通过 `version` 事件协商版本，并容忍新增请求/事件；向后不兼容的变更必须新写 ADR，并按 PRD §7.2 更新版本化的协议矩阵。`inox-ipc` **不属于**上游 `wayland-protocols`，也不作为对外稳定接口对外承诺。
5. **依赖与获取方式**：`inox-compositor` 通过锁定已发布 `tag` 的 Git 依赖引入 `inox-protocols`（根级 `AGENTS.md` §8），并在其 `deny.toml` 的 `[sources] allow-git` 中登记来源；`inox-protocols` 自身的 `rust-version` 提升到 **1.86**，因为 `wayland-protocols` 0.32 要求 rustc ≥ 1.86，其余 L1 仓库（`inox-common` / `inox-config`）保持 1.85。
6. **新增协议的统一流程**：先在 `inox-protocols` 定义（私有协议，附 XML）或引用（标准协议，经再导出），并写明安全与生命周期约束；随后由 `inox-compositor` 集成；最后更新 PRD §2.3 的协议矩阵。新增标准协议不得绕过 `inox-protocols` 直接引入。

## 后果

正面：

- 协议面收敛到单一 crate 与单一入口，标准协议版本在一处锁定，避免跨仓库版本漂移与重复封装（PRD §3.1、§2.3）。
- 私有协议以 XML 为唯一事实来源，编译期生成绑定，消除了手写编解码器与「生成代码入库」的两类错误来源。
- `version` 事件 + `reload_result` 枚举使协议演化与错误语义在协议层显式可协商，符合 PRD §5.4 与根级 `AGENTS.md` §7.2 对私有协议 unstable 与版本化的要求。
- 再导出策略让消费者只需一个依赖，也便于未来整体升级上游协议版本时集中修改。

负面：

- `inox-protocols` 的 MSRV 因 `wayland-protocols` 0.32 提升到 1.86，与其余 L1 仓库（1.85）暂时不一致；ADR 0005 中「`inox-protocols` 保持 1.85」的表述就本仓库而言已被本 ADR 取代，需在后续统一 MSRV 时关注。
- `inox-protocols` 从骨架变为被消费的真实 crate，其变更开始具有跨仓库影响，任何破坏性调整都要走「上游打 tag → 下游更新依赖 → 集成验证」的流程（根级 `AGENTS.md` §8）。
- 私有协议为 unstable，在 1.0 前 Shell/会话侧的客户端必须持续容忍新增字段与事件，测试需覆盖版本协商路径。
- 协议矩阵（PRD §2.3）与 `protocol/*.xml` 之间需要人工保持同步；新增协议时的「先定义/引用、再集成、后更新矩阵」顺序依赖纪律。

影响面：本决策直接约束 `inox-protocols` 的 `Cargo.toml`（`wayland-protocols` 0.32、`wayland-protocols-wlr` 0.3、`wayland-scanner`、MSRV 1.86）、`protocol/inox-ipc.xml` 与其 `AGENTS.md`，以及 `inox-compositor` 的 `Cargo.toml`（`inox-protocols` git 依赖）、`deny.toml`（`[sources] allow-git`）。就 M1 范围而言，`inox-protocols` 到达 M1 意味着该 crate 已真实存在并被消费，**PRD §13 中 M1 行的 DoD 本身不变**（其验收对象是 `inox-compositor`）。协议矩阵的任何破坏性变更需同步更新 PRD §2.3 与相关仓库 `AGENTS.md`。

取代条件：若上游 `wayland-protocols` / `wayland-rs` 改变绑定生成或组织方式（例如不再需要独立 scanner、或私有协议改用其他序列化机制），或 `inox-ipc` 在 1.0 前需要一次向后不兼容的大版本重构（届时按 PRD §7.2 以新 ADR 记录并对协议矩阵做版本化），需撰写取代本 ADR 的新 ADR，并同步更新 PRD §2.3、根级 `AGENTS.md` §7.2 与相关仓库的 `Cargo.toml` / `deny.toml` / `AGENTS.md`。
