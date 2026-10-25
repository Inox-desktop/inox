//! Shared harness for the M3 end-to-end tests (acceptance X01-X03).
//!
//! Everything here exists to make the M3 tests *evidence* rather than a demo:
//!
//! * every session gets a private `XDG_RUNTIME_DIR` (mode 0700), private
//!   config/data/cache directories, a private Wayland socket name and a free X
//!   display number, so two sessions can run side by side and neither touches
//!   the developer's or the CI runner's own session (`docs/quality/acceptance.md`
//!   item B02);
//! * every wait is bounded and fails fast when a child dies. There is no
//!   "sleep and hope" synchronisation anywhere: the tests synchronise on the
//!   injection socket's `ok` acknowledgement, on readiness files, on sockets and
//!   on events the clients observe themselves;
//! * every child runs in its own process group and is reaped, group and all, so
//!   a failing assertion cannot leave a stray Xwayland behind (items R03/X01);
//! * the components under test are the workspace's own binaries, located next to
//!   the test executable. This crate depends on no Inox crate (the XWayland clipboard and DnD work
//!   decision 8), so `tests/layering.rs` keeps its meaning.
//!
//! The tests are opt-in through `INOX_M3_E2E=1`: the default per-crate CI shard
//! has neither the system X11 tooling nor the built `test-input` compositor, and
//! a missing prerequisite must fail loudly rather than skip silently.

#![allow(dead_code)] // every test binary includes the whole harness

pub mod wayland_client;
pub mod x11_client;

use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream, process::CommandExt as _},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use rustix::process::{Pid, Signal, kill_process, kill_process_group};

/// Re-exported so the tests can signal children without depending on `rustix`
/// themselves. Not every test binary needs it, hence the `allow`.
#[allow(unused_imports)]
pub use rustix::process::Signal as ProcessSignal;

/// Opt-in switch for the M3 end-to-end tests.
pub const E2E_ENV: &str = "INOX_M3_E2E";
/// Opt-in switch for the "real application" leg (`xeyes`/`xwininfo`).
pub const REAL_APPS_ENV: &str = "INOX_M3_E2E_REAL_APPS";

/// Headless output size, as pinned by the M3 contract.
pub const OUTPUT_WIDTH: u32 = 1280;
/// Headless output height, as pinned by the M3 contract.
pub const OUTPUT_HEIGHT: u32 = 720;
/// Headless output refresh rate, as pinned by the M3 contract.
pub const REFRESH_RATE: u32 = 60;

/// Poll interval shared by every bounded wait.
pub const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long a session may take to come up (compositor socket, then Xwayland).
pub const START_TIMEOUT: Duration = Duration::from_secs(30);
/// Default budget for a single assertion (acceptance item B02: 30 s per case).
pub const CASE_TIMEOUT: Duration = Duration::from_secs(30);
/// Budget for a whole GUI scenario (acceptance item B02: 300 s).
pub const SCENARIO_TIMEOUT: Duration = Duration::from_secs(300);
/// Grace period given to a child before it is killed outright.
pub const KILL_GRACE: Duration = Duration::from_secs(5);

/// `evdev` code for the left `Alt` key (`KEY_LEFTALT`).
pub const KEY_LEFTALT: u32 = 56;
/// `evdev` code for `Tab` (`KEY_TAB`).
pub const KEY_TAB: u32 = 15;
/// `evdev` code for `a` (`KEY_A`), used as the "did the key arrive" probe.
///
/// The injection socket takes an `evdev` code, so this is 30, not the 38 an X11
/// client sees: X key codes are the evdev code plus 8.
pub const KEY_A: u32 = 30;
/// The X11 keycode `KEY_A` arrives as: X key codes are the evdev code plus 8.
pub const X_KEYCODE_A: u32 = KEY_A + 8;
/// `evdev`/`linux/input-event-codes.h` `BTN_LEFT`, the pointer button the drag
/// tests press and release.
pub const BTN_LEFT: u32 = 0x110;

/// The tiling parameters the compositor uses by default
/// (`crates/inox-compositor/src/layout.rs`, `LayoutConfig::default`).
pub const MASTER_RATIO: i32 = 60;
/// Number of windows tiled in the master column by default.
pub const MASTER_COUNT: usize = 1;
/// Gap between the work area edge and its tiles, and between tiles.
pub const GAP: i32 = 8;

/// The vertical space `xwayland-satellite` v0.8.3 reserves above every X11
/// window for the title bar it draws itself.
///
/// M3 deliberately does not advertise `zxdg_decoration_manager_v1`, so the
/// satellite is the one drawing X11 decorations (`DecorationsDataSatellite::
/// TITLEBAR_HEIGHT` in its `src/server/decoration.rs`). The tests do not take
/// this on faith: `X01` measures the delta between the configured Wayland size
/// and the X11 client area and asserts it stays constant across reconfigures.
pub const SATELLITE_TITLEBAR_HEIGHT: i32 = 25;

/// A rectangle in the compositor's logical coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in pixels.
    pub w: i32,
    /// Height in pixels.
    pub h: i32,
}

impl Rect {
    /// Build a rectangle.
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// The centre of the rectangle, rounded down.
    pub fn centre(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    /// A point inside the rectangle, `offset` pixels below the top edge.
    pub fn point_below_top(&self, offset: i32) -> (i32, i32) {
        (self.x + self.w / 2, self.y + offset)
    }

    /// The rectangle shrunk by `top` pixels at the top edge.
    pub fn below(&self, top: i32) -> Self {
        Self::new(self.x, self.y + top, self.w, (self.h - top).max(0))
    }

    /// Whether the point is inside the rectangle (half-open on the right/bottom).
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// The tiles the compositor's master/stack layout produces for `count` windows
/// on the configured output.
///
/// This is a deliberate re-derivation of the *documented* geometry in
/// `crates/inox-compositor/src/layout.rs` (inner area shrunk by `gap` on every
/// side, master column `master_ratio` per cent of the inner width, the stack
/// column starting one gap later, rows split evenly with a gap between them).
/// Deriving it here rather than writing the resulting numbers down means a
/// layout change has to be reflected in both places, and the M3 test fails
/// instead of silently accepting a stale expectation. The values for
/// 1280x720 are pinned by `the_expected_tiles_match_the_documented_layout`.
pub fn expected_tiles(count: usize) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }

    let inner = inner_area(
        Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32),
        GAP,
    );
    let master_count = MASTER_COUNT.min(count);
    let stack_count = count - master_count;

    if stack_count == 0 {
        return split_rows(inner, count, GAP)
            .into_iter()
            .map(|(y, h)| Rect::new(inner.x, y, inner.w, h))
            .collect();
    }

    let master_width = inner.w * MASTER_RATIO / 100;
    let stack_x = (inner.x + master_width + GAP).min(inner.x + inner.w);
    let stack_width = (inner.x + inner.w - stack_x).max(0);

    let mut tiles: Vec<Rect> = split_rows(inner, master_count, GAP)
        .into_iter()
        .map(|(y, h)| Rect::new(inner.x, y, master_width, h))
        .collect();
    tiles.extend(
        split_rows(inner, stack_count, GAP)
            .into_iter()
            .map(|(y, h)| Rect::new(stack_x, y, stack_width, h)),
    );
    tiles
}

/// The tile the window that was mapped *`mapped_index`-th* gets, with `count`
/// windows managed in total.
///
/// The layout order is *not* the mapping order: `Workspace::add` in
/// `crates/inox-compositor/src/workspace.rs` inserts every new window at the
/// front of the list, so the most recently mapped window is the layout master
/// and the windows mapped before it shift towards the stack column. X03's
/// assertions depend on knowing which client ended up in which column.
pub fn tile_of_mapped(count: usize, mapped_index: usize) -> Rect {
    let tiles = expected_tiles(count);
    assert!(
        mapped_index < count,
        "cannot ask for window {mapped_index} of {count} managed windows"
    );
    tiles[count - 1 - mapped_index]
}

/// The work area shrunk by `gap`, clamped to stay inside it.
fn inner_area(work_area: Rect, gap: i32) -> Rect {
    let right = work_area.x + work_area.w;
    let bottom = work_area.y + work_area.h;
    let left = (work_area.x + gap).min(right);
    let top = (work_area.y + gap).min(bottom);
    let inner_right = (right - gap).max(work_area.x);
    let inner_bottom = (bottom - gap).max(work_area.y);
    Rect::new(
        left,
        top,
        (inner_right - left).max(0),
        (inner_bottom - top).max(0),
    )
}

/// Split `area` into `parts` rows separated by `gap`, top to bottom.
fn split_rows(area: Rect, parts: usize, gap: i32) -> Vec<(i32, i32)> {
    if parts == 0 {
        return Vec::new();
    }
    let separators = gap * (parts as i32 - 1);
    let usable = area.h - separators;
    if usable <= 0 {
        return vec![(area.y, 0); parts];
    }
    let base = usable / parts as i32;
    let remainder = usable % parts as i32;
    let mut rows = Vec::with_capacity(parts);
    let mut cursor = area.y;
    for index in 0..parts {
        let size = base + i32::from((index as i32) < remainder);
        rows.push((cursor, size));
        cursor += size + gap;
    }
    rows
}

/// Whether the M3 end-to-end tests should run in this process.
pub fn e2e_enabled() -> bool {
    std::env::var_os(E2E_ENV).is_some()
}

/// Whether the "real X11 application" leg should run (`xeyes`, `xwininfo`).
pub fn real_apps_enabled() -> bool {
    std::env::var_os(REAL_APPS_ENV).is_some()
}

/// Print the "skipping" line and report whether the caller should return early.
///
/// Usage: `if skip_unless_enabled("X01") { return; }`
pub fn skip_unless_enabled(acceptance_id: &str) -> bool {
    if e2e_enabled() {
        return false;
    }
    println!(
        "{E2E_ENV} is not set; skipping the {acceptance_id} M3 end-to-end test. \
         Run it with `INOX_M3_E2E=1 cargo test -p inox-integration --all-features` \
         after `cargo build --workspace --features inox-compositor/test-input`."
    );
    true
}

/// Locate a binary built by the workspace next to this test executable.
///
/// `cargo test` runs a test binary from `<target>/<profile>/deps/`, so the
/// grandparent directory holds the ordinary workspace binaries.
pub fn workspace_binary(name: &str) -> PathBuf {
    let exe = std::env::current_exe()
        .unwrap_or_else(|error| panic!("cannot locate the running test executable: {error}"));
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("no profile directory above {}", exe.display()));
    let candidate = profile_dir.join(name);
    assert!(
        candidate.is_file(),
        "the binary under test is missing: {}\n\
         Build the workspace first:\n    \
         cargo build --workspace --features inox-compositor/test-input\n\
         (`inox-compositor` must be built with the non-default `test-input` feature, which \
         provides the event injection socket these tests drive.)",
        candidate.display()
    );
    candidate
}

/// Locate an executable on `PATH`.
pub fn which(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(tool))
        .find(|candidate| is_executable(candidate))
}

/// Whether `path` is an executable regular file.
pub fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Fail with a clear message when a required tool is missing from `PATH`.
///
/// Only used for the legs that are opt-in themselves: when
/// `INOX_M3_E2E_REAL_APPS` is set, a missing `xeyes` must fail the run instead of
/// quietly shrinking the evidence (`docs/quality/acceptance.md` §2: a skipped
/// dependency is not a pass).
pub fn require_tools(tools: &[&str]) {
    let missing: Vec<&str> = tools
        .iter()
        .copied()
        .filter(|tool| which(tool).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "{REAL_APPS_ENV} is set but these required tools are missing from PATH: {}. \
         Install them (for example `x11-apps x11-utils`) and retry.",
        missing.join(", ")
    );
}

/// Discard a value we intentionally do not act on (best-effort teardown).
pub fn ignore<T>(_value: T) {}

/// A private session directory tree: runtime, config, data and cache.
///
/// `XDG_RUNTIME_DIR` must be mode 0700, or the compositor refuses to bind its
/// socket there.
pub struct RuntimeDir {
    runtime: tempfile::TempDir,
    config: tempfile::TempDir,
    data: tempfile::TempDir,
    cache: tempfile::TempDir,
    wayland_socket: String,
    display: u16,
}

impl RuntimeDir {
    /// Create a private directory set with a free X display number.
    pub fn new(label: &str) -> Self {
        let runtime = private_dir(&format!("inox-m3-{label}-run"));
        let config = private_dir(&format!("inox-m3-{label}-cfg"));
        let data = private_dir(&format!("inox-m3-{label}-data"));
        let cache = private_dir(&format!("inox-m3-{label}-cache"));

        Self {
            runtime,
            config,
            data,
            cache,
            wayland_socket: format!("inox-m3-{label}"),
            display: free_display_number(),
        }
    }

    /// The private `XDG_RUNTIME_DIR`.
    pub fn path(&self) -> &Path {
        self.runtime.path()
    }

    /// The X display number this session owns, without the leading colon.
    pub fn display_number(&self) -> u16 {
        self.display
    }

    /// The X display specification, for example `:97`.
    pub fn display_spec(&self) -> String {
        format!(":{}", self.display)
    }

    /// The Wayland socket name advertised by the compositor.
    pub fn wayland_socket(&self) -> &str {
        &self.wayland_socket
    }

    /// Absolute path of the compositor's Wayland socket.
    pub fn wayland_socket_path(&self) -> PathBuf {
        self.path().join(&self.wayland_socket)
    }

    /// Absolute path of the compositor's test-only input injection socket.
    pub fn inject_socket_path(&self) -> PathBuf {
        self.path().join("test-input.sock")
    }

    /// Absolute path of the service's readiness file.
    pub fn ready_file_path(&self) -> PathBuf {
        self.path().join("xwayland-ready")
    }

    /// A log file path inside the runtime directory.
    pub fn log_path(&self, name: &str) -> PathBuf {
        self.path().join(format!("{name}.log"))
    }

    /// A scratch file path inside the runtime directory.
    pub fn scratch_path(&self, name: &str) -> PathBuf {
        self.path().join(name)
    }

    /// The environment every child process is started with.
    ///
    /// `RUST_LOG` is forwarded when the test runner sets it, so a failing case
    /// can be re-run with `RUST_LOG=inox_compositor=debug` and read the
    /// compositor's own account of it from the log the harness keeps. Unset, it
    /// stays unset, and each binary falls back to its documented default level.
    pub fn env(&self) -> Vec<(String, String)> {
        let mut env = vec![
            (
                "XDG_RUNTIME_DIR".to_owned(),
                self.path().display().to_string(),
            ),
            (
                "XDG_CONFIG_HOME".to_owned(),
                self.config.path().display().to_string(),
            ),
            (
                "XDG_DATA_HOME".to_owned(),
                self.data.path().display().to_string(),
            ),
            (
                "XDG_CACHE_HOME".to_owned(),
                self.cache.path().display().to_string(),
            ),
            ("HOME".to_owned(), self.config.path().display().to_string()),
            ("WAYLAND_DISPLAY".to_owned(), self.wayland_socket.clone()),
            ("DISPLAY".to_owned(), self.display_spec()),
            // The runner has no GPU; force Mesa's llvmpipe software rasteriser.
            ("LIBGL_ALWAYS_SOFTWARE".to_owned(), "1".to_owned()),
        ];

        if let Ok(directives) = std::env::var("RUST_LOG") {
            env.push(("RUST_LOG".to_owned(), directives));
        }

        env
    }
}

/// Create a private directory with mode 0700.
pub fn private_dir(prefix: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .unwrap_or_else(|error| panic!("failed to create temp dir {prefix}: {error}"));
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700))
        .unwrap_or_else(|error| panic!("failed to chmod {}: {error}", dir.path().display()));
    dir
}

/// Pick an X display number that is free on this host.
///
/// A developer machine or a CI runner may already run Xwayland on `:0`, so the
/// tests use a high range and check both the `/tmp/.X<n>-lock` file and the
/// `/tmp/.X11-unix/X<n>` socket that Xwayland would claim.
pub fn free_display_number() -> u16 {
    for number in 90..=140u16 {
        let lock = Path::new("/tmp").join(format!(".X{number}-lock"));
        let socket = Path::new("/tmp/.X11-unix").join(format!("X{number}"));
        if !lock.exists() && !socket.exists() {
            return number;
        }
    }
    panic!("no free X display number in 90..=140; the host is running too many X servers");
}

/// A child process killed, with its whole process group, on drop.
pub struct Proc {
    name: String,
    child: Child,
    log: PathBuf,
}

impl Proc {
    /// Spawn `command` as the leader of a fresh process group, logging stdout
    /// and stderr to `log`.
    ///
    /// A dedicated process group is what lets teardown reap grandchildren too —
    /// the `Xwayland` process `xwayland-satellite` spawns, for example — and what
    /// lets the tests assert that a shutdown left no strays behind.
    pub fn spawn(name: &str, mut command: Command, log: &Path) -> Self {
        let stdout = File::create(log)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", log.display()));
        let stderr = stdout
            .try_clone()
            .unwrap_or_else(|error| panic!("failed to duplicate {}: {error}", log.display()));
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .process_group(0);

        let child = command
            .spawn()
            .unwrap_or_else(|error| panic!("failed to spawn {name}: {error}"));
        Self {
            name: name.to_owned(),
            child,
            log: log.to_path_buf(),
        }
    }

    /// The child's process id, which is also its process group id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The child's name, for messages.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The path of the child's captured output.
    pub fn log_path(&self) -> &Path {
        &self.log
    }

    /// The child's exit status if it has already exited.
    pub fn try_wait(&mut self) -> Option<ExitStatus> {
        self.child
            .try_wait()
            .unwrap_or_else(|error| panic!("failed to poll {}: {error}", self.name))
    }

    /// Wait up to `timeout` for the child to exit.
    pub fn wait_timeout(&mut self, timeout: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.try_wait() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// Wait for the child to exit, failing the test on timeout.
    pub fn wait_ok(&mut self, what: &str, timeout: Duration) -> ExitStatus {
        self.wait_timeout(timeout).unwrap_or_else(|| {
            panic!(
                "{what}: {} did not exit within {timeout:?}:\n{}",
                self.name,
                self.diagnostics()
            )
        })
    }

    /// Assert the child is still running, failing with its own log if it is not.
    pub fn assert_alive(&mut self, what: &str) {
        if let Some(status) = self.try_wait() {
            panic!(
                "{what}: {} exited early with {status}:\n{}",
                self.name,
                self.diagnostics()
            );
        }
    }

    /// Send `signal` to the child process only (best effort).
    ///
    /// Used where the child's own signal handling is under test: signalling the
    /// whole group would kill its children out of band.
    pub fn signal(&self, signal: Signal) {
        ignore(kill_process(Pid::from_child(&self.child), signal));
    }

    /// Send `signal` to the child and its whole process group (teardown).
    pub fn signal_group(&self, signal: Signal) {
        let pid = Pid::from_child(&self.child);
        ignore(kill_process(pid, signal));
        ignore(kill_process_group(pid, signal));
    }

    /// Best-effort dump of this child's captured output, for failure messages.
    pub fn diagnostics(&self) -> String {
        match fs::read_to_string(&self.log) {
            Ok(contents) if !contents.trim().is_empty() => format!(
                "--- {} log ({}) ---\n{contents}",
                self.name,
                self.log.display()
            ),
            Ok(_) => format!(
                "--- {} log ({}) is empty ---",
                self.name,
                self.log.display()
            ),
            Err(error) => format!(
                "--- {} log ({}) could not be read: {error} ---",
                self.name,
                self.log.display()
            ),
        }
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        // Never panic while unwinding: use raw, non-panicking polling here.
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        self.signal_group(Signal::TERM);

        let deadline = Instant::now() + KILL_GRACE;
        loop {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(POLL_INTERVAL);
        }

        self.signal_group(Signal::KILL);
        // Whatever still holds the process group (for example a satellite that
        // ignores SIGTERM) gets the same treatment.
        kill_group_members(self.pid(), Signal::KILL);
        ignore(self.child.kill());
        ignore(self.child.wait());
    }
}

/// One process listed under `/proc`, as the group tests need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMember {
    /// Process id.
    pub pid: u32,
    /// Process name from `/proc/<pid>/stat`.
    pub comm: String,
}

/// Every live process that belongs to process group `pgid`.
///
/// Zombies are skipped: a child that has exited but has not been reaped yet is
/// still listed under `/proc`, and "did this group leave a stray process
/// running" is about processes that still execute, not about entries the parent
/// has yet to collect.
pub fn group_members(pgid: u32) -> Vec<GroupMember> {
    let entries = match fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut members = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // `pid (comm) state ppid pgrp ...`; `comm` may itself contain spaces and
        // parentheses, so parse from the last ')' as the kernel documents.
        let Some(close) = stat.rfind(')') else {
            continue;
        };
        let comm = stat
            .get(stat.find('(').map_or(0, |open| open + 1)..close)
            .unwrap_or_default()
            .to_owned();
        let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        // After `comm`: state(0) ppid(1) pgrp(2) ...
        if fields.first() == Some(&"Z") {
            continue;
        }
        let Some(pgrp) = fields.get(2).and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        if pgrp == pgid {
            members.push(GroupMember { pid, comm });
        }
    }
    members.sort_by_key(|member| member.pid);
    members
}

/// Send `signal` to one process (best effort).
pub fn signal_pid(pid: u32, signal: Signal) {
    if let Some(pid) = Pid::from_raw(pid as i32) {
        ignore(kill_process(pid, signal));
    }
}

/// Send `signal` to every live member of process group `pgid` (best effort).
pub fn kill_group_members(pgid: u32, signal: Signal) {
    for member in group_members(pgid) {
        if let Some(pid) = Pid::from_raw(member.pid as i32) {
            ignore(kill_process(pid, signal));
        }
    }
}

/// The first process in `pgid` whose name starts with `prefix`, if any.
///
/// The name comes from `/proc/<pid>/stat`, which the kernel truncates to 15
/// characters, so `xwayland-satellite` shows up as `xwayland-satell` and callers
/// have to pass a prefix rather than a full name.
pub fn find_in_group(pgid: u32, prefix: &str) -> Option<GroupMember> {
    group_members(pgid)
        .into_iter()
        .find(|member| member.comm.starts_with(prefix))
}

/// Wait until process group `pgid` has no live members left, failing on timeout.
///
/// `label` names the check for the failure message. `pgid`'s leader has already
/// been reaped by the caller, so any remaining member is a stray.
pub fn assert_group_empty(pgid: u32, label: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let members = group_members(pgid);
        if members.is_empty() {
            return;
        }
        if Instant::now() >= deadline {
            let listing: Vec<String> = members
                .iter()
                .map(|member| format!("  pid {} ({})", member.pid, member.comm))
                .collect();
            // Do not leave the strays behind for the next test in this binary.
            kill_group_members(pgid, Signal::KILL);
            panic!(
                "{label}: process group {pgid} still has {} member(s) after {timeout:?}:\n{}",
                members.len(),
                listing.join("\n")
            );
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Wait until `path` exists, failing fast if the owner process dies first.
pub fn wait_for_path(owner: &mut Proc, path: &Path, what: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if path.exists() {
            return;
        }
        if let Some(status) = owner.try_wait() {
            panic!(
                "{what}: {} exited early with {status} before creating {}:\n{}",
                owner.name(),
                path.display(),
                owner.diagnostics()
            );
        }
        if Instant::now() >= deadline {
            panic!(
                "{what}: timed out after {timeout:?} waiting for {}:\n{}",
                path.display(),
                owner.diagnostics()
            );
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Wait until `path` exists (no owning process to watch).
pub fn wait_for_path_alone(path: &Path, what: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        if Instant::now() >= deadline {
            panic!(
                "{what}: timed out after {timeout:?} waiting for {}",
                path.display()
            );
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Wait until reading `path` yields a non-empty trimmed string.
pub fn read_ready_file(owner: &mut Proc, path: &Path, timeout: Duration) -> String {
    wait_for_path(owner, path, "readiness file", timeout);
    let deadline = Instant::now() + timeout;
    loop {
        match fs::read_to_string(path) {
            Ok(contents) if !contents.trim().is_empty() => return contents.trim().to_owned(),
            Ok(_) => {}
            Err(error) => panic!("failed to read {}: {error}", path.display()),
        }
        if Instant::now() >= deadline {
            panic!(
                "readiness file {} stayed empty for {timeout:?}:\n{}",
                path.display(),
                owner.diagnostics()
            );
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Read a whole stream, replacing invalid UTF-8.
pub fn read_all(reader: &mut impl Read) -> String {
    let mut buffer = Vec::new();
    ignore(reader.read_to_end(&mut buffer));
    String::from_utf8_lossy(&buffer).into_owned()
}

/// Captured output of a short-lived command.
pub struct Captured {
    /// Exit status.
    pub status: ExitStatus,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
}

/// Run `program` with `args`, capturing stdout/stderr, bounded by `timeout`.
pub fn run_captured(
    program: &str,
    args: &[&str],
    env: &[(String, String)],
    timeout: Duration,
) -> Captured {
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .unwrap_or_else(|error| panic!("failed to spawn {program}: {error}"));

    let mut stdout_pipe = child.stdout.take().expect("stdout is piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr is piped");
    let stdout_reader = thread::spawn(move || read_all(&mut stdout_pipe));
    let stderr_reader = thread::spawn(move || read_all(&mut stderr_pipe));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => panic!("failed to poll {program}: {error}"),
        }
        if Instant::now() >= deadline {
            ignore(child.kill());
            ignore(child.wait());
            panic!("{program} did not finish within {timeout:?}");
        }
        thread::sleep(POLL_INTERVAL);
    };

    Captured {
        status,
        stdout: stdout_reader.join().expect("stdout reader panicked"),
        stderr: stderr_reader.join().expect("stderr reader panicked"),
    }
}

/// A decoded `dump-frame` result: a binary PPM (`P6`) read back from disk.
///
/// The compositor writes the headless output's pixels here, which is what makes
/// "the X11 window is actually composited and visible" an assertion about
/// pixels rather than about the compositor's internal bookkeeping.
pub struct Frame {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// `RGB` triples, row major.
    pixels: Vec<u8>,
}

impl Frame {
    /// Read and decode a binary PPM written by `dump-frame`.
    pub fn read_ppm(path: &Path) -> Self {
        let bytes = fs::read(path).unwrap_or_else(|error| {
            panic!("failed to read frame dump {}: {error}", path.display())
        });
        let (header_len, width, height) = parse_ppm_header(&bytes, path);
        let expected = width as usize * height as usize * 3;
        let pixels = bytes
            .get(header_len..header_len + expected)
            .unwrap_or_else(|| {
                panic!(
                    "frame dump {} is truncated: header says {width}x{height} ({expected} bytes) \
                     but only {} bytes follow",
                    path.display(),
                    bytes.len() - header_len
                )
            })
            .to_vec();
        Self {
            width,
            height,
            pixels,
        }
    }

    /// The colour at `(x, y)`, or `None` when the point is outside the frame.
    pub fn pixel(&self, x: i32, y: i32) -> Option<[u8; 3]> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        let index = (y as usize * self.width as usize + x as usize) * 3;
        Some([
            self.pixels[index],
            self.pixels[index + 1],
            self.pixels[index + 2],
        ])
    }

    /// How many pixels inside `area` are within `tolerance` of `colour`.
    pub fn count_matching(&self, area: Rect, colour: [u8; 3], tolerance: u8) -> usize {
        let mut count = 0;
        for y in area.y.max(0)..(area.y + area.h).min(self.height as i32) {
            for x in area.x.max(0)..(area.x + area.w).min(self.width as i32) {
                if let Some(pixel) = self.pixel(x, y)
                    && near(pixel, colour, tolerance)
                {
                    count += 1;
                }
            }
        }
        count
    }

    /// The most common colour inside `area`, for failure messages.
    pub fn dominant_colour(&self, area: Rect) -> ([u8; 3], usize) {
        let mut counts: std::collections::HashMap<[u8; 3], usize> =
            std::collections::HashMap::new();
        for y in area.y.max(0)..(area.y + area.h).min(self.height as i32) {
            for x in area.x.max(0)..(area.x + area.w).min(self.width as i32) {
                if let Some(pixel) = self.pixel(x, y) {
                    *counts.entry(pixel).or_default() += 1;
                }
            }
        }
        counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .unwrap_or(([0, 0, 0], 0))
    }

    /// Assert that `area` is substantially covered by `colour`.
    ///
    /// "Substantially" is half the area: an X11 window that is only partly
    /// composited (a stale frame, a missing damage) must not pass.
    pub fn assert_area_covered(&self, area: Rect, colour: [u8; 3], tolerance: u8, label: &str) {
        let expected = (area.w.max(0) * area.h.max(0)) as usize;
        let matched = self.count_matching(area, colour, tolerance);
        let (dominant, dominant_count) = self.dominant_colour(area);
        assert!(
            matched * 2 >= expected,
            "{label}: expected {area:?} ({} pixels) to be mostly {colour:?} (+/-{tolerance}), \
             but only {matched} pixels matched; the dominant colour there is \
             {dominant:?} ({dominant_count} pixels). Frame is {}x{}.",
            expected,
            self.width,
            self.height
        );
    }
}

/// Whether two colours differ by no more than `tolerance` on every channel.
pub fn near(actual: [u8; 3], expected: [u8; 3], tolerance: u8) -> bool {
    actual
        .iter()
        .zip(expected.iter())
        .all(|(a, e)| a.abs_diff(*e) <= tolerance)
}

/// Parse the ASCII header of a binary PPM, returning the offset of the pixels.
fn parse_ppm_header(bytes: &[u8], path: &Path) -> (usize, u32, u32) {
    let mut fields: Vec<String> = Vec::new();
    let mut index = 0;
    while fields.len() < 4 && index < bytes.len() {
        // Skip whitespace and comments.
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'#') {
            if bytes[index] == b'#' {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            } else {
                index += 1;
            }
        }
        let start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if start == index {
            break;
        }
        fields.push(String::from_utf8_lossy(&bytes[start..index]).into_owned());
    }

    assert_eq!(
        fields.first().map(String::as_str),
        Some("P6"),
        "frame dump {} is not a binary PPM (magic {:?})",
        path.display(),
        fields.first()
    );
    let width: u32 = fields
        .get(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("frame dump {} has no width in its header", path.display()));
    let height: u32 = fields
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("frame dump {} has no height in its header", path.display()));
    assert_eq!(
        fields.get(3).map(String::as_str),
        Some("255"),
        "frame dump {} must use 8 bits per channel",
        path.display()
    );
    // Exactly one whitespace byte follows the maximum value.
    let pixels = index + 1;
    (pixels, width, height)
}

/// Client for the compositor's test-only input injection socket.
///
/// The protocol is line based, one reply per command, terminated by `\n`
/// (`ok` on success, `err <message>` otherwise). The reply is an
/// acknowledgement that the events have been flushed to clients, which is what
/// lets the tests synchronise on real state instead of sleeping.
pub struct Injector {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    socket: PathBuf,
    timeout: Duration,
}

impl Injector {
    /// Connect to the injection socket, waiting for it to appear.
    pub fn connect(socket: &Path, owner: &mut Proc, timeout: Duration) -> Self {
        wait_for_path(owner, socket, "input injection socket", timeout);

        let deadline = Instant::now() + timeout;
        loop {
            match UnixStream::connect(socket) {
                Ok(stream) => {
                    ignore(stream.set_read_timeout(Some(timeout)));
                    ignore(stream.set_write_timeout(Some(timeout)));
                    let reader = BufReader::new(
                        stream
                            .try_clone()
                            .unwrap_or_else(|error| panic!("failed to clone socket: {error}")),
                    );
                    return Self {
                        writer: stream,
                        reader,
                        socket: socket.to_path_buf(),
                        timeout,
                    };
                }
                Err(error) => {
                    if Instant::now() >= deadline {
                        panic!(
                            "could not connect to the injection socket {}: {error}\n{}",
                            socket.display(),
                            owner.diagnostics()
                        );
                    }
                    owner.assert_alive("connecting to the injection socket");
                    thread::sleep(POLL_INTERVAL);
                }
            }
        }
    }

    /// Send one command and return the acknowledgement line's payload.
    ///
    /// # Panics
    ///
    /// Panics on a timeout, on a closed connection and on an `err <message>`
    /// reply: every caller in these tests wants a passing acknowledgement, and a
    /// silent failure would turn into a mysterious later timeout.
    pub fn command(&mut self, command: &str) -> Result<(), String> {
        self.writer
            .write_all(command.as_bytes())
            .and_then(|()| self.writer.write_all(b"\n"))
            .and_then(|()| self.writer.flush())
            .unwrap_or_else(|error| {
                panic!(
                    "failed to write {command:?} to the injection socket {}: {error}",
                    self.socket.display()
                )
            });

        let mut line = String::new();
        let read = self.reader.read_line(&mut line).unwrap_or_else(|error| {
            panic!(
                "no reply to {command:?} within {:?} from the injection socket {}: {error}",
                self.timeout,
                self.socket.display()
            )
        });
        assert!(
            read > 0,
            "the injection socket {} closed on {command:?}",
            self.socket.display()
        );

        let reply = line.trim_end();
        match reply.split_once(' ') {
            None if reply == "ok" => Ok(()),
            Some(("err", message)) => Err(message.to_owned()),
            _ => Err(format!("unexpected reply {reply:?}")),
        }
    }

    /// Send a command that must be acknowledged, panicking with context if not.
    pub fn ack(&mut self, command: &str) {
        if let Err(message) = self.command(command) {
            panic!("the compositor rejected {command:?}: {message}");
        }
    }

    /// Inject an `evdev` key press or release.
    pub fn key(&mut self, evdev_code: u32, press: bool) {
        self.ack(&format!(
            "key {evdev_code} {}",
            if press { "press" } else { "release" }
        ));
    }

    /// Inject an `evdev` key press followed by its release.
    pub fn key_tap(&mut self, evdev_code: u32) {
        self.key(evdev_code, true);
        self.key(evdev_code, false);
    }

    /// Inject an absolute pointer motion.
    pub fn motion(&mut self, x: f64, y: f64) {
        self.ack(&format!("motion {x} {y}"));
    }

    /// Inject a pointer button press or release.
    pub fn button(&mut self, code: u32, press: bool) {
        self.ack(&format!(
            "button {code} {}",
            if press { "press" } else { "release" }
        ));
    }

    /// Move the pointer to a rectangle's centre.
    pub fn motion_to(&mut self, x: i32, y: i32) {
        self.motion(f64::from(x) + 0.5, f64::from(y) + 0.5);
    }

    /// Ask the compositor to write the current frame to `path` and read it back.
    pub fn dump_frame(&mut self, path: &Path) {
        let _ = fs::remove_file(path);
        self.ack(&format!("dump-frame {}", path.display()));
        wait_for_path_alone(path, "frame dump", CASE_TIMEOUT);
    }

    /// Dump the current frame and decode it.
    pub fn frame(&mut self, path: &Path) -> Frame {
        self.dump_frame(path);
        Frame::read_ppm(path)
    }
}

/// Serialises the sessions inside one test binary.
///
/// Two tests starting a session at the same time would race for the same free X
/// display number (`free_display_number` cannot reserve a number Xwayland has
/// not bound yet), and a CI runner has no reason to host two software-rendered
/// compositors plus two satellites at once. Tests in *different* binaries still
/// run in parallel, which is where the parallelism is worth having.
static SESSION_LOCK: Mutex<()> = Mutex::new(());

/// A live M3 session: compositor, optional XWayland service, injection client.
pub struct Session {
    /// The private directory set this session owns.
    pub runtime: RuntimeDir,
    /// The compositor under test.
    pub compositor: Proc,
    /// The XWayland service, when this session runs one.
    pub service: Option<Proc>,
    /// The compositor's input injection client.
    pub injector: Injector,
    /// Held for the whole session, so only one session runs at a time.
    _serialised: MutexGuard<'static, ()>,
}

impl Session {
    /// Start a compositor and the XWayland service, and wait for both.
    pub fn start(label: &str) -> Self {
        let mut session = Self::start_compositor(label);
        session.start_service();
        session
    }

    /// Start only the compositor.
    ///
    /// The Wayland-to-Wayland drag-and-drop evidence (X03) needs no X11 host, so
    /// it does not pay for a satellite start-up; the display number is still
    /// reserved so the session stays self-contained.
    pub fn start_compositor(label: &str) -> Self {
        let serialised = SESSION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let runtime = RuntimeDir::new(label);
        let env = runtime.env();

        let mut command = Command::new(workspace_binary("inox-compositor"));
        command
            .args([
                "--backend=headless",
                "--socket",
                runtime.wayland_socket(),
                "--test-input",
                &runtime.inject_socket_path().display().to_string(),
                "--output-size",
                &format!("{OUTPUT_WIDTH}x{OUTPUT_HEIGHT}"),
                "--refresh-rate",
                &REFRESH_RATE.to_string(),
            ])
            .envs(env.iter().cloned());

        let mut compositor = Proc::spawn(
            "inox-compositor",
            command,
            &runtime.log_path("inox-compositor"),
        );

        let socket_path = runtime.wayland_socket_path();
        wait_for_path(
            &mut compositor,
            &socket_path,
            "Wayland socket",
            START_TIMEOUT,
        );

        let injector = Injector::connect(
            &runtime.inject_socket_path(),
            &mut compositor,
            START_TIMEOUT,
        );

        // The `ok` reply to a trivial command proves the injection path is live
        // before any test starts driving input through it.
        let mut injector = injector;
        injector.motion(1.0, 1.0);

        Self {
            runtime,
            compositor,
            service: None,
            injector,
            _serialised: serialised,
        }
    }

    /// Start the XWayland service for this session and wait for real readiness.
    pub fn start_service(&mut self) -> &mut Proc {
        assert!(
            self.service.is_none(),
            "this session already runs an XWayland service"
        );

        let runtime = &self.runtime;
        let env = runtime.env();
        let ready_file = runtime.ready_file_path();

        let mut command = Command::new(workspace_binary("inox-xwayland"));
        command
            .args([
                "--display",
                &runtime.display_spec(),
                "--ready-file",
                &ready_file.display().to_string(),
                "--x11-socket-dir",
                "/tmp/.X11-unix",
                "--ready-timeout",
                "25",
                "--shutdown-grace",
                "5",
            ])
            .envs(env.iter().cloned());

        let mut service = Proc::spawn("inox-xwayland", command, &runtime.log_path("inox-xwayland"));

        // A readiness file whose content is not this session's display means the
        // service answered for a display it does not own; that is a real defect,
        // not a race, so it is asserted rather than waited out.
        let contents = read_ready_file(&mut service, &ready_file, START_TIMEOUT);
        assert_eq!(
            contents,
            runtime.display_spec(),
            "the readiness file must name this session's display"
        );

        self.service = Some(service);
        self.service.as_mut().expect("just stored")
    }

    /// The display specification of this session, for X11 clients.
    pub fn display_spec(&self) -> String {
        self.runtime.display_spec()
    }

    /// The environment X11 and Wayland clients are started with.
    pub fn env(&self) -> Vec<(String, String)> {
        self.runtime.env()
    }

    /// Capture the compositor's current frame.
    pub fn frame(&mut self) -> Frame {
        let path = self.runtime.scratch_path("frame.ppm");
        self.injector.frame(&path)
    }

    /// Assert the compositor is still alive and serving clients.
    pub fn assert_compositor_alive(&mut self, what: &str) {
        self.compositor.assert_alive(what);
    }
}

/// Write an executable satellite stand-in that never makes Xwayland ready.
///
/// The script does nothing at all: it never creates `<socket_dir>/X<n>`, so the
/// service's readiness probe can never succeed and `--ready-timeout` has to
/// fire. The companion case — a socket that exists, accepts a connection and
/// then never answers — is covered by
/// `crates/inox-xwayland/tests/lifecycle.rs`; what this leg adds is that the
/// *real* service binary reaches its timeout and cleans up its process group.
pub fn write_idle_satellite_stand_in(runtime: &Path, name: &str) -> PathBuf {
    let path = runtime.join(name);
    let script = "#!/bin/sh\n# Satellite stand-in: never brings Xwayland up.\nexec sleep 300\n";
    fs::write(&path, script)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|error| panic!("failed to chmod {}: {error}", path.display()));
    path
}

/// The result of [`dead_x11_socket`].
///
/// Keep it alive for as long as the negative case runs: dropping it closes the
/// listener.
pub struct DeadX11Socket {
    _listener: std::os::unix::net::UnixListener,
    /// The socket path that exists on disk.
    pub path: PathBuf,
}

/// Bind — and never accept on — an X11 socket for `display`.
///
/// A connected client gets a live socket and then silence, which is exactly the
/// state X01 says must **not** count as ready: the socket exists, `connect(2)`
/// succeeds, and no X11 setup round trip ever completes.
pub fn dead_x11_socket(socket_dir: &Path, display: u16) -> DeadX11Socket {
    fs::create_dir_all(socket_dir)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", socket_dir.display()));
    let path = socket_dir.join(format!("X{display}"));
    let _ = fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path)
        .unwrap_or_else(|error| panic!("failed to bind {}: {error}", path.display()));
    DeadX11Socket {
        _listener: listener,
        path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_expected_tiles_match_the_documented_layout() {
        // Pins the derivation above against the numbers
        // `crates/inox-compositor/src/layout.rs` produces for a 1280x720 work
        // area with the default `LayoutConfig` (master_ratio 60, master_count 1,
        // gap 8): the inner area is inset by the gap on every side.
        assert_eq!(expected_tiles(0), Vec::new());
        assert_eq!(expected_tiles(1), vec![Rect::new(8, 8, 1264, 704)]);
        assert_eq!(
            expected_tiles(2),
            vec![Rect::new(8, 8, 758, 704), Rect::new(774, 8, 498, 704)]
        );
        assert_eq!(
            expected_tiles(3),
            vec![
                Rect::new(8, 8, 758, 704),
                Rect::new(774, 8, 498, 348),
                Rect::new(774, 364, 498, 348),
            ]
        );
    }

    #[test]
    fn the_newest_window_takes_the_master_column() {
        // Two windows mapped in order: the second one mapped is the master and
        // the first one is the stack column.
        assert_eq!(tile_of_mapped(2, 0), expected_tiles(2)[1]);
        assert_eq!(tile_of_mapped(2, 1), expected_tiles(2)[0]);
        // Padding counts too, so the arithmetic is checked against the real
        // geometry rather than against itself.
        assert_eq!(tile_of_mapped(2, 1), Rect::new(8, 8, 758, 704));
        assert_eq!(tile_of_mapped(2, 0), Rect::new(774, 8, 498, 704));
        // Three windows: the newest is the master, the two earlier ones are the
        // stack column in mapping order.
        assert_eq!(tile_of_mapped(3, 2), expected_tiles(3)[0]);
        assert_eq!(tile_of_mapped(3, 1), expected_tiles(3)[1]);
        assert_eq!(tile_of_mapped(3, 0), expected_tiles(3)[2]);
    }

    #[test]
    fn the_expected_tiles_stay_inside_the_output_and_do_not_overlap() {
        for count in 1..=5 {
            let tiles = expected_tiles(count);
            assert_eq!(tiles.len(), count);
            for tile in &tiles {
                assert!(tile.w > 0 && tile.h > 0, "degenerate tile {tile:?}");
                assert!(tile.x >= 0 && tile.y >= 0, "tile outside output {tile:?}");
                assert!(
                    tile.x + tile.w <= OUTPUT_WIDTH as i32
                        && tile.y + tile.h <= OUTPUT_HEIGHT as i32,
                    "tile overflows the output {tile:?}"
                );
            }
            for (index, left) in tiles.iter().enumerate() {
                for right in &tiles[index + 1..] {
                    let disjoint = left.x + left.w <= right.x
                        || right.x + right.w <= left.x
                        || left.y + left.h <= right.y
                        || right.y + right.h <= left.y;
                    assert!(disjoint, "tiles overlap: {left:?} and {right:?}");
                }
            }
        }
    }

    #[test]
    fn a_free_display_number_has_no_lock_or_socket() {
        let number = free_display_number();
        assert!((90..=140).contains(&number));
        assert!(!Path::new("/tmp").join(format!(".X{number}-lock")).exists());
    }

    #[test]
    fn process_group_members_lists_this_process() {
        // Not `std::process::id()`: a test binary inherits the process group of
        // whatever started it (cargo, a shell job), which is only the same value
        // when nothing grouped it differently.
        let pgid = rustix::process::getpgrp();
        let pgid = u32::try_from(pgid.as_raw_pid()).expect("a positive process group id");
        let members = group_members(pgid);
        assert!(
            members
                .iter()
                .any(|member| member.pid == std::process::id()),
            "the running process must appear in its own group {pgid}: {members:?}"
        );
    }
}
