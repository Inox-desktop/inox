//! Starts the process supervisor; component internals remain behind process boundaries.
use anyhow::{Context, Result, bail};
use inox_common::i18n::Locale;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use inox_session::runtime::{CommandSpec, RunOptions, ServiceSpec, SessionSpec};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{error, info, warn};

/// The environment variable that asks the session to start the trusted lock
/// client, which takes the session lock for the rest of the session.
const LOCK_ON_START: &str = "INOX_SESSION_TAKE_LOCK";

fn main() -> Result<()> {
    let mut raw = std::env::args_os().skip(1);
    if raw.next().is_some_and(|arg| arg == "--exec-child") {
        // calloop signalfd masks are inherited across exec. Reset them in the
        // child process, where doing so cannot interrupt the supervisor.
        let signals = calloop::signals::Signals::new(&[
            calloop::signals::Signal::SIGINT,
            calloop::signals::Signal::SIGTERM,
            calloop::signals::Signal::SIGHUP,
        ])?;
        drop(signals);
        let program = raw.next().context("child launcher requires a program")?;
        use std::os::unix::process::CommandExt;
        return Err(std::process::Command::new(program).args(raw).exec().into());
    }
    inox_common::logging::init()?;
    let early = inox_session::i18n::load(Locale::from_env())?;
    let mut args = std::env::args_os().skip(1);
    let mut headless = false;
    let mut input_method = Vec::new();
    let mut bin_dir = std::env::current_exe()?
        .parent()
        .context("executable has no directory")?
        .to_path_buf();
    let mut exit_after = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                info!("{}", early.message("usage"));
                return Ok(());
            }
            Some("--headless") => headless = true,
            Some("--input-method") => {
                let executable = args
                    .next()
                    .context("--input-method requires an executable")?;
                if executable.is_empty() || !input_method.is_empty() {
                    bail!("--input-method requires one nonempty executable, specified once");
                }
                input_method.extend([OsString::from("--input-method"), executable]);
            }
            Some("--input-method-arg") => {
                let value = args.next().context("--input-method-arg requires a value")?;
                if input_method.is_empty() {
                    bail!("--input-method-arg must follow --input-method");
                }
                input_method.extend([OsString::from("--input-method-arg"), value]);
            }
            Some("--bin-dir") => {
                bin_dir = PathBuf::from(args.next().context("--bin-dir requires a path")?)
            }
            Some("--exit-after-seconds") => {
                let value = args
                    .next()
                    .context("--exit-after-seconds requires a value")?;
                let seconds: u64 = value.to_str().context("duration is not UTF-8")?.parse()?;
                if !(1..=86400).contains(&seconds) {
                    bail!("duration must be 1..=86400 seconds");
                }
                exit_after = Some(Duration::from_secs(seconds));
            }
            _ => bail!("unrecognized argument: {}", arg.to_string_lossy()),
        }
    }
    let config = InoxConfig::load(&ConfigPaths::discover()?, Appearance::System)?;
    let localizer = inox_session::i18n::load(config.locale)?;

    // One capability per session, delivered to exactly two processes: the
    // compositor, which decides whether a claim is honoured, and the trusted lock
    // client, which claims the role with it. It is never
    // logged, never written to disk, and never given to any other component — the
    // shell would otherwise hold the right to take the session lock.
    let lock_token = inox_session::policy::session_lock_token()?;

    let mut compositor = CommandSpec::new(bin_dir.join("inox-compositor"));
    compositor.arguments = vec![OsString::from("--socket"), OsString::from("wayland-inox")];
    if headless {
        compositor.arguments.push(OsString::from("--headless"));
    }
    compositor
        .arguments
        .extend([OsString::from("--lock-token"), OsString::from(&lock_token)]);

    compositor.arguments.extend(input_method);

    let mut services: Vec<ServiceSpec> = [
        // One process per surface: a `wlr-layer-shell` surface belongs to one
        // client connection, so the desktop home — a menu bar along the top edge
        // and a dock along the bottom one — is two panel instances. `--surface`
        // says which one each instance is; a bare `inox-panel` keeps using the
        // configured default (`[panel] surface`), which is what a developer runs
        // by hand.
        ("shell", "inox-shell", &[][..], true, true),
        ("wallpaper", "inox-wallpaper", &[][..], false, false),
        (
            "panel-menubar",
            "inox-panel",
            &["--surface", "menubar"][..],
            true,
            false,
        ),
        (
            "panel-dock",
            "inox-panel",
            &["--surface", "dock"][..],
            true,
            false,
        ),
        ("notifications", "inox-notifications", &[][..], false, false),
        ("idle", "inox-idle", &[][..], false, false),
    ]
    .into_iter()
    .map(
        |(name, program, arguments, required, oneshot)| ServiceSpec {
            name: name.into(),
            command: CommandSpec {
                program: bin_dir.join(program).into(),
                arguments: arguments.iter().map(OsString::from).collect(),
            },
            required,
            oneshot,
            environment: BTreeMap::new(),
        },
    )
    .collect();

    // The trusted lock client is started only when the session is asked for it.
    // `inox-lock --take-lock` takes the real session lock and has no way to
    // release it again, so starting it unconditionally would lock a session that
    // nobody asked to lock; the switch is how an operator or an end-to-end test
    // asks. Manual and idle-triggered locking are still open.
    if std::env::var_os(LOCK_ON_START).is_some_and(|value| value == "1") {
        let mut lock = CommandSpec::new(bin_dir.join("inox-lock"));
        lock.arguments = vec![OsString::from("--take-lock")];
        services.push(ServiceSpec {
            name: "lock".into(),
            command: lock,
            // A crashed lock UI keeps the session locked and is restarted within
            // the same bounded budget as every other optional component (ADR
            // 0015). Once the budget is spent the session stays locked and stops
            // retrying, which is the fail-closed direction.
            required: false,
            oneshot: false,
            environment: BTreeMap::from([("INOX_LOCK_TOKEN".to_owned(), lock_token)]),
        });
    }
    let spec = SessionSpec {
        compositor,
        services,
        environment: config.root.environment,
        runtime_dir: PathBuf::from(
            std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is required")?,
        ),
        startup_timeout: Duration::from_secs(10),
    };
    info!("{}", localizer.message("starting"));
    warn!("{}", localizer.message("limited"));
    if let Err(failure) = inox_session::runtime::run(
        spec,
        RunOptions {
            signals: true,
            exit_after,
            child_launcher: Some(std::env::current_exe()?),
        },
    ) {
        error!(
            "{}",
            localizer.message_with("failed", &[("reason", failure.to_string().into())])
        );
        return Err(failure.into());
    }
    Ok(())
}
