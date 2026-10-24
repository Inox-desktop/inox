//! The login screen, and the two things it refuses to pretend about.
//!
//! `inox-greet` draws the login interface and speaks `auth-v1` to the
//! authentication boundary (ADR 0029). It does not authenticate anybody itself, so
//! it has exactly two ways to start:
//!
//! - **No arguments: the real path.** The boundary socket has to be there. If it is
//!   not, the process says so and exits non-zero *before* a window exists — a login
//!   screen that collected a password and only then discovered there was nobody to
//!   check it would waste the one thing a login screen must not waste. The socket is
//!   `/run/inox/auth.sock` unless `INOX_AUTH_SOCKET` names somewhere else, which is
//!   how a development instance and a host that runs the greeter as its own user
//!   both work without a second binary.
//! - **`--preview`: the interface without a boundary.** It opens the same login
//!   screen in the state this build is in without one — unavailable and not
//!   typable — and starts no session. That is not a login path and must never look
//!   like one, which is why the banner in that state says exactly that.
//!
//! What neither path can do is start a session: the boundary does that, it decides
//! the program, and the login screen only ever learns that it happened.
//!
//! Both paths do read the system's accounts ([`inox_greet::accounts`]), because the
//! screen offers the accounts that exist rather than a text box to spell one into.
//! A database that cannot be read is not fatal: the screen says which problem it is
//! and stays up, since a person with a working account should not be refused a login
//! by a greeter that could not list the others.
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{Result, bail};
use inox_auth::wire::DEFAULT_SOCKET;
use inox_common::i18n::Locale;
use inox_common::paths::XdgEnv;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use inox_greet::{accounts, app, boundary, i18n};
use tracing::{error, info};

/// The environment variable that moves the boundary's socket.
const SOCKET_ENV: &str = "INOX_AUTH_SOCKET";

fn main() -> Result<()> {
    inox_common::logging::init()?;
    let early = i18n::load(&Locale::from_env())?;
    let mut preview = false;

    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--help" | "-h" => {
                info!("{}", early.message("greet-usage"));
                return Ok(());
            }
            "--preview" => preview = true,
            _ => bail!("unrecognized argument: {argument}"),
        }
    }

    let config = InoxConfig::load(&ConfigPaths::discover()?, Appearance::System)?;
    let localizer = Rc::new(i18n::load(&config.locale)?);
    let tokens = config.theme.clone();

    let socket = if preview {
        None
    } else {
        let socket = socket_path();
        if !boundary::is_available(&socket) {
            error!("{}", localizer.message("greet-blocked"));
            bail!(
                "no authentication boundary is listening on {} (start inox-auth, or pass \
                 --preview to open the interface without one)",
                socket.display()
            );
        }
        info!(socket = %socket.display(), "the login screen found its authentication boundary");
        Some(socket)
    };

    // The accounts the screen offers, and the record of which one a session was
    // started for last. The record lives in this component's state directory, so a
    // greeter that has never started a session offers the first account the system
    // lists instead.
    let roster = accounts::Accounts::from_system(accounts::record_file(&XdgEnv::from_process()));
    info!(
        accounts = roster.accounts.len(),
        "the system's accounts were read"
    );

    iced::application(
        move || {
            let state = match socket.clone() {
                Some(socket) => {
                    app::State::connected(localizer.clone(), tokens.clone(), socket, roster.clone())
                }
                None => app::State::new(localizer.clone(), tokens.clone(), roster.clone()),
            };
            let focus = state.focus_task();
            (state, focus)
        },
        app::update,
        app::view,
    )
    .title(app::State::title)
    .theme(app::State::theme)
    .subscription(app::subscription)
    .run()?;

    Ok(())
}

/// The boundary socket this invocation talks to.
fn socket_path() -> PathBuf {
    std::env::var_os(SOCKET_ENV)
        .map(PathBuf::from)
        .filter(|path: &PathBuf| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET))
}
