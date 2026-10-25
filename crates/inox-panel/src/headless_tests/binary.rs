//! Runs the panel's own entry point in a child process.
//!
//! The crate is binary only (migration contract §2.1), so `env!("CARGO_BIN_EXE_…")`
//! does not exist for these unit tests. What does exist is the test executable
//! itself: the child is *this same binary*, filtered down to the one test below,
//! with the panel's own command line in [`ARGUMENTS`]. That test then calls
//! [`crate::main`], and the child's exit status is the panel's.
//!
//! Two things about the child are load-bearing and not incidental:
//!
//! - **It runs serially, on the main thread** (`--test-threads=1`). Qt insists
//!   that its application object belongs to the main thread, and the `calloop`
//!   signal sources block `SIGINT`/`SIGTERM`/`SIGHUP` for the thread that
//!   installs them — a mask that does not cover every thread would let a
//!   `SIGTERM` kill the panel before it can tear its surface down. The test
//!   asserts the thread it is on so that a change in the harness fails here
//!   rather than as a mysterious Qt abort.
//! - **The command line arrives through the environment**, because the arguments
//!   the test harness itself parses (`--exact`, `--test-threads`) are not the
//!   panel's. `crate::main` reads [`ARGUMENTS`] only when it is compiled with
//!   `cfg(test)`, so a shipped binary never looks at it.

use std::process::Command;

/// The one test the child process runs.
pub const ENTRY_POINT: &str = "headless_tests::binary::the_panel_entry_point";

/// The environment variable that carries the panel's own command line.
pub const ARGUMENTS: &str = "INOX_PANEL_ARGUMENTS";

/// A command that runs the panel's entry point in a child process.
///
/// Callers add the panel's own arguments with [`ARGUMENTS`] and its environment
/// as usual, then [`spawn`](Command::spawn) it.
pub fn panel_command() -> Command {
    let executable = std::env::current_exe()
        .expect("the test executable's own path, which is where the panel's entry point is");

    let mut command = Command::new(executable);
    command
        .arg("--exact")
        .arg(ENTRY_POINT)
        .arg("--test-threads=1")
        .arg("--nocapture");

    command
}

/// The panel's entry point, run by the child process.
///
/// In a normal test run the environment variable is absent and this is a no-op;
/// in the child it *is* the panel, and the process ends with the panel's exit
/// status.
///
/// The child is this test binary, so this function runs on a test thread rather
/// than on `main`. Two things in the panel care about threads and both are
/// handled where they are used rather than assumed here: Qt's application object
/// is created by the offscreen host in [`crate::offscreen`], and the signal mask
/// that makes `signalfd` receive the panel's stop signals is inherited from the
/// fixture that spawns this process (see `headless_tests::protocol`).
#[test]
fn the_panel_entry_point() {
    if std::env::var_os(ARGUMENTS).is_none() {
        return;
    }

    match crate::main() {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("{error:?}");
            std::process::exit(1);
        }
    }
}
