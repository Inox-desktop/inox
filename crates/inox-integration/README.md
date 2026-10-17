# inox-integration

端到端集成测试。属于 Inox 桌面环境的 **测试** 层。

## 状态

**分层方向检查已落地**（验收项 B04）：`tests/layering.rs` 读取 `cargo metadata` 的真实依赖图，对照根 `Cargo.toml` 的层级表断言 ADR 0002 的依赖方向规则，失败时给出具体的边与被违反的规则。它带有一组对合成图的单元测试，因此规则本身失效会在 CI 中暴露，而不是静默通过。

端到端测试仍待 **M6**（PRD §13）引入：届时本 crate 才依赖各组件，并在 CI 中驱动完整会话。因此 `src/main.rs` 目前只是骨架（日志初始化）。

```bash
cargo test -p inox-integration --test layering   # 分层方向检查
```

## 开发约定

- 本项目遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，不在本地进行编译验证。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)。

## 许可证

BSD 3-Clause，见仓库根的 [`LICENSE`](../../LICENSE)。
