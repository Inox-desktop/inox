//! Starts the process supervisor; component internals remain behind process boundaries.
use anyhow::{Context, Result, bail};
use inox_common::i18n::Locale;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use inox_session::runtime::{CommandSpec, RunOptions, ServiceSpec, SessionSpec};
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{error, info, warn};

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
    let mut compositor = CommandSpec::new(bin_dir.join("inox-compositor"));
    compositor.arguments = vec![OsString::from("--socket"), OsString::from("wayland-inox")];
    if headless {
        compositor.arguments.push(OsString::from("--headless"));
    }
    let services = [
        ("shell", true, true),
        ("wallpaper", false, false),
        ("panel", true, false),
        ("notifications", false, false),
        ("idle", false, false),
    ]
    .into_iter()
    .map(|(name, required, oneshot)| ServiceSpec {
        name: name.into(),
        command: CommandSpec::new(bin_dir.join(format!("inox-{name}"))),
        required,
        oneshot,
    })
    .collect();
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
