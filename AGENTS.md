# AGENTS.md

This file provides guidance to Qoder (qoder.com) when working with code in this repository.

## Repository Overview

**Inox** is a modular Wayland desktop environment written in **Rust (stable, 2024 edition)** based on **Smithay + QML/Qt Quick**. It follows an AI-first development workflow where AI generates most code and humans handle product decisions, architecture验收, and real hardware testing.

### Key Characteristics

- **Single Cargo workspace**: All components in [`crates/`](./crates), organized in a strict 4-layer architecture
- **AI-first development**: No local compilation/testing - all validation happens in GitHub Actions CI
- **Modular layered architecture**: Strict dependency direction (L4 → L3 → L2 → L1), no cross-dependencies
- **LGPL-3.0 licensed**: Open-source friendly, Qt ecosystem compatible
- **QML-based UI**: Shell components use QML + Qt Quick bound via cxx-qt (Iced removed as of Oct 2026)

---

## Development Commands

> **CRITICAL**: These are reference commands only. **All validation must pass in CI** - local runs don't count as acceptance.

### Common Operations

```bash
# Format all code
cargo fmt --all

# Run clippy on entire workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Test a single crate (CI uses this pattern)
cargo test -p <crate-name> --all-features

# Test entire workspace
cargo test --workspace --all-features

# Build release binaries
cargo build --workspace --release --all-features

# Check layering rules (must always pass)
cargo test -p inox-integration --test layering

# Check dependencies against whitelist
cargo deny check

# Generate documentation
cargo doc --workspace --no-deps --all-features

# Build developer documentation
mdbook build docs/book
```

### Crate-Specific Testing

Each crate can be tested independently:

```bash
# Example: Test compositor
cargo test -p inox-compositor --all-features

# Example: Test panel with offscreen rendering
QT_QPA_PLATFORM=offscreen cargo test -p inox-panel --all-features

# Example: Test protocols
cargo test -p inox-protocols --all-features
```

### Integration Tests

```bash
# M3 end-to-end tests (X11 interop)
INOX_M3_E2E=1 cargo test -p inox-integration --all-features

# XWayland headless e2e
INOX_XWAYLAND_E2E=1 cargo test -p inox-xwayland --all-features --test headless_x11_mapping

# Session lock E2E
dbus-run-session -- cargo test -p inox-session --test headless --test session_lock -- --nocapture
```

---

## Architecture & Structure

### Four-Layer Architecture

**Dependency Rule**: Dependencies can ONLY flow downward (L4 → L3 → L2 → L1). No upward or same-layer dependencies except explicitly documented exceptions.

```
┌─────────────────────────────────────────────┐
│ Layer 4: Session & System Integration       │
│  inox-session, inox-greet, inox-idle,       │
│  inox-host                                  │
├─────────────────────────────────────────────┤
│ Layer 3: Shell & User Interface             │
│  inox-shell, inox-panel, inox-launcher,     │
│  inox-notifications, inox-settings,         │
│  inox-lock, inox-wallpaper, inox-glass,     │
│  inox-qml-prototype                          │
├─────────────────────────────────────────────┤
│ Layer 2: Compositor & Window Management     │
│  inox-compositor, inox-xwayland,            │
│  inox-auth, inox-protocols                   │
├─────────────────────────────────────────────┤
│ Layer 1: Infrastructure & Shared Libraries  │
│  inox-common, inox-config                    │
└─────────────────────────────────────────────┘
```

### Core Components

| Crate | Layer | Responsibility | Status |
|-------|-------|----------------|--------|
| `inox-common` | L1 | Shared types, error handling, logging, i18n, utilities | Implemented |
| `inox-config` | L1 | Configuration parsing (TOML), default configs | Implemented |
| `inox-protocols` | L2 | Wayland protocol bindings, custom extensions | Implemented |
| `inox-compositor` | L2 | Wayland compositor based on Smithay | Partially implemented (M1-M2 complete, M3 partial) |
| `inox-xwayland` | L2 | XWayland integration (manages xwayland-satellite process) | Implemented |
| `inox-auth` | L2 | Authentication boundary (PAM + auth-v1) | Implemented |
| `inox-shell` | L3 | Shell framework, plugin system | Partially implemented |
| `inox-panel` | L3 | Panel (taskbar, system tray, clock) | Partially implemented |
| `inox-glass` | L3 | Liquid glass material API & QML components | Implemented |
| `inox-notifications` | L3 | Notification daemon (org.freedesktop.Notifications) | Partially implemented |
| `inox-session` | L4 | Session manager, starts compositor & shell | Partially implemented |
| `inox-integration` | Test | End-to-end tests, layering verification | Partially implemented |

### Runtime Data Flow

```
Applications (Wayland/X11 clients)
        │  wayland socket
        ▼
 ┌───────────────────┐   inox-ipc (custom Wayland protocol)
 │  inox-compositor  │◄───────────────────────────┐
 └───────────────────┘                             │
        ▲  layer-shell / xdg-shell              ┌──┴────────────┐
        │                                       │  inox-shell   │
 ┌──────┴────────┐   D-Bus (notifications,      │  & L3组件    │
 │  inox-session │   settings, portals)         └───────────────┘
 └───────────────┘
```

**Important**: Compositor and Shell communicate **only** through Wayland protocols/D-Bus/IPC, NOT through Rust dependencies.

---

## Hard Rules (Must Follow)

### 1. **No Local Compilation Verification**
- All validation MUST pass in GitHub Actions CI
- Local builds/tests are for convenience only, not acceptance criteria
- CI jobs: `fmt`, `clippy`, `deny`, `doc`, `build`, `test`, `m3-e2e`, `xwayland-e2e`, `session-e2e`, `license-audit`, `package`, `aur`, `book`

### 2. **Unsafe Code Policy**
- **Default**: `#[forbid(unsafe_code)]` workspace-wide
- **Exceptions only in**: 
  - `inox-compositor` (FFI glue, EGL/GLES rendering, calloop Generic source)
  - `inox-xwayland` (X11 FFI)
- All unsafe blocks MUST have `// SAFETY:` comments explaining correctness
- Do NOT add new unsafe code without explicit justification

### 3. **Error Handling**
- Non-test code: NO `unwrap()`, `expect()`, `panic!()`, `todo!()`, `unimplemented!()`
- Use proper error propagation with `thiserror` (library) + `anyhow` (application)
- Silent failures forbidden - errors must be logged or propagated

### 4. **Logging**
- MUST use `tracing` framework
- NO `println!` / `eprintln!` in production code
- Default log level: `info` (not verbose per-frame logging)
- Structured logs with appropriate events/levels

### 5. **License Compliance**
- Project license: LGPL-3.0
- **Allowed**: Permissive licenses + LGPL modules only
- **Forbidden**: AGPL/SSPL, GPL-only Qt modules (Qt Wayland Compositor, Qt Virtual Keyboard, Qt QML Compiler)
- `cargo-deny` enforces dependency license whitelist
- `license-audit` CI job checks system Qt `.so` linkage

### 6. **Internationalization**
- ALL user-visible text MUST go through Fluent localization
- NO hardcoded strings (product/component names除外)
- Must support `en-US` and `zh-CN`
- Use design tokens for colors/sizes (never hardcode RGB values)

### 7. **Code Style**
- Identifiers/comments: English ONLY
- Human-facing docs (README, AGENTS): Chinese allowed
- Follow existing comment density and naming patterns
- Consistent directory structure per crate

### 8. **Layering Rules**
- Every crate MUST belong to exactly one layer (defined in root `Cargo.toml`)
- Dependency violations cause CI failure (`cargo test -p inox-integration --test layering`)
- Same-layer exceptions must be documented in `[workspace.metadata.inox.rules]`
- `inox-common` is leaf node (no outward dependencies)
- Compositor and Shell decoupled (no Rust dependency between them)

---

## ADR Process

**Architecture Decision Records required for**:
- Adding/deleting component crates
- Changing layer structure or dependency directions
- Introducing new protocol strategies
- Replacing core dependencies (Smithay/Qt/calloop)
- Breaking IPC or configuration format changes
- Any change requiring `unsafe` exceptions

**Process**:
1. Create new ADR in `docs/adr/` (template: `<number>-<title>.md`)
2. Status: `proposed` → discussion → `accepted`
3. Link ADR in PR description affecting architecture
4. Update layering table if crates affected

Current ADRs: 0040 (Liquid Glass), 0041 (Session Control Socket), 0042 (Release Artifacts), 0043 (AUR Packages), 0044 (Notification Glass Cards), 0045 (Window Management)

---

## Milestones & Acceptance Criteria

### Current Status: **M0, M1, M2 Complete | M3 Partially Implemented**

| Milestone | Goals | Evidence Required |
|-----------|-------|-------------------|
| **M0** | Engineering baseline | CI infrastructure, skeleton/license/constraints |
| **M1** | Headless rendering/lifecycle | Output/pixel assertions, clean exit on signal (E2) |
| **M2** | Window/input semantics | Real client lifecycle, popup/focus, tiled layout ≥2 workspaces (E1+E2) |
| **M2.5** | Real device loopback | Backend CI coverage + human hardware testing, visible pointer (E4) |
| **M3** | X11/clipboard/dnd | Inox session interoperability, clipboard focus-follows (E2+E3) |
| **M4** | Shell tasks/multi-screen/config | Taskbar, hotplug/HIDPI, config reload/builtin fallback, bilingual (E2+E3+E4) |
| **M5** | Secure sessions/daily use | Lockscreen/auth/suspend faults, permission denial, process supervision (E1+E3+E4) |
| **M6** | Release candidate | Performance metrics, stability data, support matrix, upgrade rollback (E4) |

### Evidence Levels

| Level | Environment | Cannot Claim |
|-------|-------------|--------------|
| **E0** | Static checks (CI fmt/clippy/build/doc/deny) | Runtime correctness, device support, desktop usability |
| **E1** | Unit/property/state machine tests in CI | Real protocol chains, GPU performance |
| **E2** | Real Wayland/D-Bus clients in CI (software rendering) | Full Inox session, real KMS/libinput, input latency |
| **E3** | Integration tests driving Inox headless session | Real hardware support, suspend restore, specific GPU |
| **E4** | Human operator using CI artifacts on registered hardware | Untested GPUs/kernels/combinations |

**Reference**: [`docs/quality/acceptance.md`](./docs/quality/acceptance.md) contains full acceptance specifications.

---

## CI Workflow Structure

### Job Breakdown

The CI pipeline consists of parallel and sequential jobs:

1. **Static Checks** (all run on ubuntu-latest):
   - `fmt`: `cargo fmt --all --check`
   - `msrv`: Check Rust 1.89 compatibility
   - `clippy`: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
   - `deny`: `cargo deny check`
   - `doc`: Documentation build with `-D warnings`

2. **Build & Test**:
   - `build`: Release build of entire workspace
   - `plan`: Derives test matrix from layer table
   - `test`: Parallel per-crate testing with appropriate system deps
   - Matrix ensures every crate gets built/tested; uncovered members fail CI

3. **End-to-End Tests**:
   - `xwayland-e2e`: Third-party compositor (Weston) testing
   - `m3-e2e`: Real Inox compositor with X11 clients
   - `session-e2e`: L4 headless session with lock functionality
   - `input-method-e2e`: Fcitx5 Chinese input testing

4. **Quality Checks**:
   - `license-audit`: Verify only LGPL Qt modules linked
   - `book`: Documentation structure and link validation
   - `package`: Build canary tarball with metadata validation
   - `aur`: Arch User Recipe validation

### System Dependencies

Jobs install system packages dynamically based on crate requirements:
- `graphics` crates (with `smithay`): EGL, Mesa, DRM libs
- `ui` crates (with `cxx-qt`): Qt 6 dev packages, fonts
- PAM-support crates: clang for libpam bindings

See `.github/actions/system-deps/action.yml` for exact package lists.

---

## Working with Crates

### Crate Template Pattern

Most crates follow this structure:

```
<crate>/
  ├── Cargo.toml           # Workspace inheritance
  ├── src/
  │   ├── main.rs          # Binary entry point
  │   ├── lib.rs           # Library入口 (if applicable)
  │   ├── config.rs        # CLI/config parsing (before Smithay init)
  │   ├── state.rs         # Main state machine
  │   ├── runtime.rs       # Event loop setup
  │   └── ...
  ├── tests/               # Integration tests (optional)
  ├── i18n/                # Localization files (if L3)
  │   ├── en-US/*.ftl
  │   └── zh-CN/*.ftl
  ├── qml/                 # QML files (if L3)
  ├── AGENTS.md            # Crate-specific guidance
  └── README.md            # Component documentation
```

### Key Patterns by Layer

#### L1 (Infrastructure)
- Pure logic, no GUI dependencies
- Error handling with `thiserror`
- Shared types exported to upper layers
- Thread-safe, testable in isolation

#### L2 (Compositor)
- Smithay-based state management
- `src/state.rs` lists advertised globals
- Real clients in tests via socket pair
- Proptest for layout algorithms
- EGL surfaceless for offscreen rendering

#### L3 (Shell/QML)
- cxx-qt bridge for Rust↔QML communication
- Offscreen rendering with `QT_QPA_PLATFORM=offscreen`
- JSON projection from Rust state to QML
- Fluent localization required
- Design tokens for theming

#### L4 (Session)
- System-level services
- D-Bus endpoints
- Process supervision
- Signal handling before any threading

---

## Testing Strategy

### Test Categories

1. **Unit Tests** (pure logic):
   - Required for all pure functions
   - Can use `unwrap()`/`panic!()`
   - Coverage target: ≥70% for core crates

2. **Property Tests** (proptest):
   - Tiling layout invariants
   - Workspace management properties
   - Protocol encoding/decoding
   - ≥1000 samples per test case

3. **Integration Tests** (process-level):
   - Real clients via `wayland-client`
   - Socket pair connections
   - Offscreen rendering with pixel assertions
   - Timeout protection (30s default)

4. **End-to-End Tests** (integration):
   - Multiple components working together
   - Third-party compositors for E2
   - Real Inox compositor for E3
   - Hardware validation for E4

### Critical Test Files

- `tests/window_management.rs`: Window lifecycle, map门槛, tiling
- `tests/glass_blur.rs`: Glass material pixel evidence
- `tests/clipboard.rs`: Focus-following selection
- `tests/session_lock.rs`: Lock screen security properties
- `headless_tests/`: L3 offscreen rendering tests

---

## Known Limitations & Gaps

### Technical Constraints

1. **Glass Material on Physical GPU**: Not implemented yet (warn+log only)
2. **Floating Window Geometry**: Not persisted across session restarts
3. **Workspace Count**: Fixed at startup, not dynamic
4. **Interactive Move/Resize**: Only mode conversion implemented, not dragging
5. **Cross-X11/Wayland Drag-and-Drop**: Blocked upstream (xwayland-satellite limitation)
6. **AT-SPI Accessibility**: Not bridged on offscreen surfaces
7. **Real Hardware Screenshot Evidence**: Still missing for W05/D03

### Incomplete Milestones

- **M4 Window Management**: E3/E4 evidence (screenshots/hardware) not collected
- **M5 Login Chain**: Hardware evidence (S06-S11) pending human validation
- **Performance Metrics**: P01-P05 budgets established but unmeasured
- **Hardware Compatibility**: H01-H03 still being catalogued

See [`docs/quality/acceptance.md`](./docs/quality/acceptance.md) and individual crate `README.md` for detailed status.

---

## Documentation Standards

### File Types

- **README.md**: Each crate has component documentation
- **AGENTS.md**: Developer guidance per crate (extends root rules)
- **docs/book/**: User/developer handbook (mdBook)
- **docs/adr/**: Architecture Decision Records
- **docs/quality/**: Acceptance criteria and quality standards

### Writing Rules

- Keep documentation focused: maintain content only, don't introduce build scripts or extra tooling
- Component status must be verifiable in corresponding `crates/<name>/`
- Mark unimplemented features clearly as "skeleton", "planned", or "not implemented"
- Links within repo: use relative paths
- External links: skip network checks in CI gates

### Doc Validation

CI validates:
- `mdbook build docs/book` succeeds
- All SUMMARY.md chapters exist
- All relative links resolve
- Markdown linting (`.markdownlint.json`)

---

## Commit Conventions

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <subject>

[optional body]

[optional footer referencing issues]
```

Types: `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `ci`

Scopes match crate names: `compositor`, `panel`, `common`, `config`, etc.

Example:
```
feat(compositor): add fractional scale manager v1 support

Implement wp_fractional_scale_manager_v1 with per-output scale factors.
Includes client-side test cases for 1.5x scaling scenarios.

Closes: #123
```

---

## Emergency & Recovery

### Failed Installation

If package installation fails mid-way:
```bash
# Rollback to greetd
sudo packaging/rollback-to-greetd.sh

# Or try uninstall
sudo packaging/uninstall.sh
```

### Session Hang

If graphical session hangs:
```bash
# From VT3 rescue console (always available)
systemctl --user-stop Inox-*  # Stop Inox user services
journalctl -u 'inox-*' -b     # Debug current boot
```

### Git Safety

- Never merge to main without CI green + 1 human review
- Never use `--no-verify` to bypass hooks/CI
- Never force-push to protected branches
- If commit needs fixing: amend locally, force push feature branches only

---

## Resources

- **Organization**: <https://github.com/Inox-desktop>
- **Complete Architecture Docs**: [`docs/book/`](./docs/book/)
- **Acceptance Criteria**: [`docs/quality/acceptance.md`](./docs/quality/acceptance.md)
- **Protocol Specs**: [`crates/inox-protocols/protocol/`](./crates/inox-protocols/protocol/)
- **ADR Directory**: [`docs/adr/`](./docs/adr/)
- **CI Workflows**: `.github/workflows/`
- **Packaging Scripts**: `packaging/`

---

## Memory & Context Summary

For efficient assistance, remember:

1. **Single workspace, multiple crates**: All under `crates/`, versions coordinated in root `Cargo.toml`
2. **Layering table is truth**: `[workspace.metadata.inox.layers]` determines which crates get tested/built
3. **Test matrix derived automatically**: Plan job reads layer table, so uncovered crates fail CI
4. **CI is absolute authority**: Local success doesn't guarantee acceptance; only CI green counts
5. **Hardware evidence critical**: E4 tests require human operators on registered devices
6. **L3 QML migration complete**: Iced stack removed, everything now uses cxx-qt + Qt Quick
7. **Two evidence tracks per requirement**: E1+E2 (unit/integration) ≠ E3+E4 (system/hardware)

This AGENTS.md consolidates rules from root README, each crate's AGENTS.md, and acceptance specs into one canonical guide for future Qoder instances.
