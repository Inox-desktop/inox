//! `inox-host` — the greeter stage: the compositor, the login screen, and who watches them.
//!
//! This binary is what `inox-greeter@.service` runs (ADR 0037 §决策 1). It does four
//! things, in this order and no more:
//!
//! 1. If the authentication boundary cannot be connected to, it says so and exits
//!    non-zero **before** a compositor or a window exists (ADR 0037 §决策 3.1). The
//!    sentence names both the boundary and `inox-auth`, because the person reading it
//!    needs to know what is missing and what provides it.
//! 2. It starts `inox-compositor --socket wayland-inox-greet`, then waits up to ten
//!    seconds for that socket to appear while the compositor is still alive.
//! 3. It starts `inox-greet` with `WAYLAND_DISPLAY` and `INOX_AUTH_SOCKET` injected and
//!    its standard input closed, so the login screen gets the display this stage owns
//!    and the boundary it must talk to, and nothing else.
//! 4. It supervises: the login screen is restarted within a bounded budget, the
//!    compositor's exit ends the stage, and `SIGTERM`/`SIGINT` stop both children in
//!    reverse order.
//!
//! What it never does is decide whether a login succeeded. The user's session is on
//! another VT, started by `inox-auth`; this VT is paused while that one is active, and
//! by the time it is resumed the login screen is simply on screen again (ADR 0037
//! §决策 4). A host that tried to observe the login would be a second opinion about an
//! authentication decision it is not allowed to have.
//!
//! The exit codes come from [`inox_host::Exit`]: `0` when systemd asked the host to
//! stop, and a distinct non-zero code for each way the stage can fail, so that
//! `systemctl status` classifies the failure without the journal.

use std::convert::Infallible;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use inox_common::i18n::Locale;
use inox_common::paths::XdgEnv;
use inox_host::cli::{self, Defaults, Invocation, Options};
use inox_host::i18n;
use inox_host::process::{AUTH_SOCKET_ENV, CommandSpec, EXEC_CHILD};
use inox_host::runtime;
use inox_host::supervisor::Plan;
use tracing::{error, info};

/// The boundary socket `inox-auth` listens on by default (ADR 0029).
const DEFAULT_AUTH_SOCKET: &str = "/run/inox/auth.sock";

/// The exit code for a command line that cannot be used.
///
/// Distinct from the stage's own failures so that a typo in the unit file is not
/// mistaken for a broken login chain.
const USAGE_EXIT: u8 = 2;

fn main() -> Result<ExitCode> {
    // The internal child-launcher mode runs before anything else, including logging:
    // it exists only to reset the signal mask this process inherited and then become
    // the program the host asked for, and it must not touch the process otherwise.
    if let Some(arguments) = exec_child_arguments() {
        let mut arguments = arguments.into_iter();
        let program = arguments
            .next()
            .context("the internal child launcher needs a program to run")?;
        let never: Infallible = inox_host::process::exec_child(program, arguments.collect())?;
        match never {}
    }

    inox_common::logging::init()?;
    let localizer = i18n::load(&Locale::from_env())?;

    let defaults = Defaults {
        bin_dir: executable_directory()?,
        auth_socket: auth_socket_default(),
    };
    let options = match cli::parse(std::env::args_os().skip(1), defaults) {
        Ok(Invocation::Help) => {
            // Answered from the parse alone: no socket is probed, no directory is
            // resolved, so this works on a machine whose login chain is broken.
            info!("{}", localizer.message("host-usage"));
            return Ok(ExitCode::SUCCESS);
        }
        Ok(Invocation::Version) => {
            info!(
                "{}",
                localizer.message_with(
                    "host-version",
                    &[("version", env!("CARGO_PKG_VERSION").into())],
                )
            );
            return Ok(ExitCode::SUCCESS);
        }
        Ok(Invocation::Run(options)) => options,
        Err(error) => {
            error!("{error}");
            return Ok(ExitCode::from(USAGE_EXIT));
        }
    };

    let plan = plan(options)?;
    info!(
        compositor = %plan.compositor.program.to_string_lossy(),
        greeter = %plan.greeter.program.to_string_lossy(),
        wayland_display = %plan.wayland_display,
        auth_socket = %plan.auth_socket.display(),
        "starting the greeter stage"
    );

    let launcher =
        std::env::current_exe().context("the greeter host cannot find its own executable")?;
    let exit = runtime::run(plan, launcher, localizer)?;
    Ok(ExitCode::from(exit.code()))
}

/// The arguments after `--exec-child`, when this invocation is the internal launcher.
fn exec_child_arguments() -> Option<Vec<OsString>> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(OsStr::new(EXEC_CHILD)) {
        return None;
    }
    Some(arguments.collect())
}

/// The directory `inox-compositor` and `inox-greet` are expected in.
///
/// Installing both binaries side by side is what makes the host independent of `PATH`:
/// the unit runs as the greeter user, whose `PATH` is not a thing this stage should
/// have an opinion about.
fn executable_directory() -> Result<PathBuf> {
    let executable =
        std::env::current_exe().context("the greeter host cannot find its own executable")?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .context("the greeter host's executable has no directory")
}

/// The boundary socket to require, from `$INOX_AUTH_SOCKET` and then the default.
fn auth_socket_default() -> PathBuf {
    std::env::var_os(AUTH_SOCKET_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from(DEFAULT_AUTH_SOCKET))
}

/// Builds the plan: the two commands, and the three paths they need.
fn plan(options: Options) -> Result<Plan> {
    let runtime_dir = XdgEnv::from_process()
        .runtime_dir()
        .context("the greeter host needs $XDG_RUNTIME_DIR to know where the Wayland socket goes")?;

    let mut compositor = CommandSpec::new(options.bin_dir.join("inox-compositor"));
    compositor.arguments = vec![OsString::from("--socket"), OsString::from(&options.socket)];
    compositor
        .arguments
        .extend(options.compositor_args.iter().cloned());

    Ok(Plan {
        wayland_socket: runtime_dir.join(&options.socket),
        wayland_display: options.socket,
        auth_socket: options.auth_socket,
        compositor,
        // The login screen takes no arguments: its display and its boundary arrive in
        // the environment this host sets when it starts it (ADR 0037 §决策 3.3).
        greeter: CommandSpec::new(options.bin_dir.join("inox-greet")),
    })
}
