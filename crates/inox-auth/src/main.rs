//! The privileged helper: parse the deployment, then serve the login screen.
//!
//! This binary is the only thing in the workspace that runs with privilege, so its
//! argument handling is deliberately narrow. Everything it decides is decided once,
//! before a socket exists:
//!
//! - **The session program comes from here and never from the wire.** A client can
//!   name an account and a credential; it cannot name a command, an argument, an
//!   environment variable or a uid.
//! - **The system socket requires root.** Serving `/run/inox/auth.sock` without
//!   privilege is refused at startup rather than discovered by the first person who
//!   tries to log in, because a boundary that cannot read the account database can
//!   only ever answer "rejected". An explicit `--socket` naming some other path is
//!   treated as a development instance and says so in its log line.
//! - **The greeter's own uid and group are the default peers**, resolved from the
//!   account database rather than written down: on a machine with the `greeter`
//!   account (the one this project's packaging creates) the shipped defaults work
//!   with no arguments at all, and `--allow-uid`/`--socket-group` override them.
//! - **`--session-leader` is an internal mode, not a second program.** The service
//!   starts itself with it for one login; it is never typed by a
//!   person and it is only accepted from root, which is what makes "the arguments
//!   came from the root parent, not from the wire" true by construction.
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use inox_auth::i18n;
use inox_auth::leader::{self, LeaderRequest};
use inox_auth::logind::VtRange;
use inox_auth::pam::PamFactory;
use inox_auth::server::{self, Config, DEFAULT_SESSION_PROGRAM, DEFAULT_SOCKET};
use inox_auth::session;
use inox_common::i18n::{Arg, Locale};
use rustix::process::geteuid;
use tracing::{info, warn};

/// The account the greeter is expected to run as.
const GREETER_ACCOUNT: &str = "greeter";

fn main() -> Result<()> {
    inox_common::logging::init()?;
    let early = i18n::load(&Locale::from_env())?;

    let mut config = Config::new(DEFAULT_SOCKET, DEFAULT_SESSION_PROGRAM);
    let mut explicit_peers = false;
    let mut leader_requested = false;
    let mut leader_user: Option<String> = None;
    let mut leader_vt: Option<u32> = None;
    let mut vt_min: Option<u32> = None;
    let mut vt_max: Option<u32> = None;
    let mut arguments = std::env::args_os().skip(1);

    while let Some(argument) = arguments.next() {
        let mut value = || -> Result<OsString> {
            arguments
                .next()
                .context("the option needs a value after it")
        };

        match argument.to_str() {
            Some("--help" | "-h") => {
                info!("{}", early.message("auth-usage"));
                return Ok(());
            }
            Some("--socket") => {
                config.socket = PathBuf::from(value()?);
            }
            Some("--socket-group") => {
                config.socket_group = Some(parse(&early, "--socket-group", &value()?)?);
                explicit_peers = true;
            }
            Some("--allow-uid") => {
                config
                    .allowed_uids
                    .push(parse(&early, "--allow-uid", &value()?)?);
                explicit_peers = true;
            }
            Some("--allow-gid") => {
                config
                    .allowed_gids
                    .push(parse(&early, "--allow-gid", &value()?)?);
                explicit_peers = true;
            }
            Some("--service") => {
                config.service = utf8("--service", &value()?)?;
            }
            Some("--session-service") => {
                config.session_service = utf8("--session-service", &value()?)?;
            }
            Some("--minimum-uid") => {
                config.minimum_uid = parse(&early, "--minimum-uid", &value()?)?;
            }
            Some("--session-program") => {
                config.session_program = PathBuf::from(value()?);
            }
            Some("--session-arg") => {
                config.session_arguments.push(value()?);
            }
            Some("--seat") => {
                config.seat = utf8("--seat", &value()?)?;
            }
            Some("--session-vt-min") => {
                vt_min = Some(parse(&early, "--session-vt-min", &value()?)?);
            }
            Some("--session-vt-max") => {
                vt_max = Some(parse(&early, "--session-vt-max", &value()?)?);
            }
            Some("--attempts") => {
                config.rate.attempts = parse(&early, "--attempts", &value()?)?;
            }
            Some("--window-seconds") => {
                config.rate.window = Duration::from_secs(u64::from(parse::<u32>(
                    &early,
                    "--window-seconds",
                    &value()?,
                )?));
            }
            Some("--exit-after-session") => config.exit_after_session = true,
            // The internal mode the service starts itself with (the boot login chain
            // §决策 5). Its arguments are validated in `leader::run`.
            Some("--session-leader") => leader_requested = true,
            Some("--user") => leader_user = Some(utf8("--user", &value()?)?),
            Some("--vt") => leader_vt = Some(parse(&early, "--vt", &value()?)?),
            Some(other) => bail!("unrecognized argument: {other}"),
            None => bail!("unrecognized argument: {}", argument.to_string_lossy()),
        }
    }

    if vt_min.is_some() || vt_max.is_some() {
        let min = vt_min.unwrap_or(config.session_vt.min());
        let max = vt_max.unwrap_or(config.session_vt.max());
        config.session_vt = VtRange::new(min, max).map_err(|_| {
            anyhow::anyhow!(
                "{}",
                early.message_with(
                    "auth-vt-range",
                    &[
                        ("min", Arg::from(min.to_string())),
                        ("max", Arg::from(max.to_string())),
                    ]
                )
            )
        })?;
    }

    if leader_requested {
        let user = leader_user
            .ok_or_else(|| anyhow::anyhow!("{}", early.message("auth-leader-arguments")))?;
        let vt = leader_vt
            .ok_or_else(|| anyhow::anyhow!("{}", early.message("auth-leader-arguments")))?;

        return leader::run(&config, &LeaderRequest { user, vt }, &early);
    }

    if leader_user.is_some() || leader_vt.is_some() {
        bail!("{}", early.message("auth-leader-only"));
    }

    if !explicit_peers {
        admit_the_greeter_account(&mut config);
    }

    let serving_the_system_socket = config.socket == Path::new(DEFAULT_SOCKET);
    if serving_the_system_socket && geteuid().as_raw() != 0 {
        anyhow::bail!("{}", early.message("auth-requires-root"));
    }
    if !serving_the_system_socket {
        warn!(
            socket = %config.socket.display(),
            "serving a socket other than the system one: this instance is a development instance, \
             not the display manager's authentication boundary"
        );
    }

    if !session::is_executable(&config.session_program) {
        bail!(
            "{}",
            early.message_with(
                "auth-session-program",
                &[(
                    "path",
                    Arg::from(config.session_program.to_string_lossy().to_string())
                )]
            )
        );
    }

    info!(
        socket = %config.socket.display(),
        service = %config.service,
        session_service = %config.session_service,
        session = %config.session_program.display(),
        vt_range = %config.session_vt,
        seat = %config.seat,
        "the Inox authentication boundary is starting"
    );

    // The factory is built from the same value the server is about to be given, so
    // the PAM stack the boundary authenticates through cannot drift from the one it
    // logs.
    let factory = PamFactory::new(config.service.clone());

    server::run(config, factory)?;

    Ok(())
}

/// Adds the greeter account's uid and group to the defaults, when it exists.
///
/// An account that is not there is not an error: a deployment that runs the greeter
/// as some other user passes `--allow-uid` and `--socket-group`, and a development
/// instance may have neither.
fn admit_the_greeter_account(config: &mut Config) {
    let Ok(account) = session::account(
        GREETER_ACCOUNT,
        // The greeter is a system account by construction, so the login floor does
        // not apply to it: what is looked up here is a uid to admit, not a session
        // to start.
        0,
    ) else {
        warn!(
            account = GREETER_ACCOUNT,
            "the greeter account does not exist; serving root only, as configured"
        );
        return;
    };

    config.allowed_uids.push(account.uid);
    if config.socket_group.is_none() {
        config.socket_group = Some(account.gid);
        // The session program of *this* lookup is irrelevant; the greeter's own
        // system account is refused as a login account below regardless of the
        // floor, because `account()` refuses uid 0 and the packaging gives the
        // greeter a uid of its own.
    }
    // The start-up hand-back recognises the greeter's own logind session by this
    // uid.
    config.greeter_uid = Some(account.uid);

    info!(
        uid = account.uid,
        gid = account.gid,
        "the greeter account may connect"
    );
}

/// Parses a numeric argument, reporting the option rather than the position.
fn parse<T: std::str::FromStr>(
    localizer: &inox_common::i18n::Localizer,
    option: &str,
    value: &OsString,
) -> Result<T> {
    let text = value
        .to_str()
        .with_context(|| format!("{option} needs a UTF-8 value"))?;

    text.parse::<T>().map_err(|_| {
        anyhow::anyhow!(
            "{}",
            localizer.message_with(
                "auth-argument",
                &[
                    ("value", Arg::from(text.to_owned())),
                    ("option", Arg::from(option.to_owned())),
                ]
            )
        )
    })
}

/// A string argument that must be UTF-8.
fn utf8(option: &str, value: &OsString) -> Result<String> {
    value
        .clone()
        .into_string()
        .map_err(|_| anyhow::anyhow!("{option} needs a UTF-8 value"))
}
