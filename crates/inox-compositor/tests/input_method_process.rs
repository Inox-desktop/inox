//! Process-level coverage of private fd handoff and bounded IME supervision.

use inox_protocols::standard::wp::input_method::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2, zwp_input_method_v2::ZwpInputMethodV2,
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_registry, wl_seat::WlSeat},
};

struct Probe;
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
wayland_client::delegate_noop!(Probe: ignore ZwpInputMethodManagerV2);
wayland_client::delegate_noop!(Probe: ignore ZwpInputMethodV2);
wayland_client::delegate_noop!(Probe: ignore WlSeat);

#[test]
fn input_method_child_probe() {
    let Some(output) = std::env::var_os("INOX_IME_PROBE_OUTPUT") else {
        return;
    };
    assert_eq!(std::env::var("WAYLAND_SOCKET").unwrap(), "0");
    let connection = Connection::connect_to_env().unwrap();
    let (globals, mut queue) = registry_queue_init::<Probe>(&connection).unwrap();
    let manager: ZwpInputMethodManagerV2 = globals.bind(&queue.handle(), 1..=1, ()).unwrap();
    let seat: WlSeat = globals.bind(&queue.handle(), 1..=9, ()).unwrap();
    let _method = manager.get_input_method(&seat, &queue.handle(), ());
    queue.roundtrip(&mut Probe).unwrap();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(output)
        .unwrap();
    writeln!(file, "{}", std::process::id()).unwrap();
    if std::env::var_os("INOX_IME_PROBE_HOLD").is_some() {
        while queue.blocking_dispatch(&mut Probe).is_ok() {}
    } else {
        std::process::exit(42);
    }
}

struct Host(Child);
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "process condition timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn start(dir: &std::path::Path, output: &std::path::Path, hold: bool) -> Host {
    let mut command = Command::new(env!("CARGO_BIN_EXE_inox-compositor"));
    command
        .args([
            "--headless",
            "--socket",
            "wayland-ime-process",
            "--input-method",
        ])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--input-method-arg=--exact",
            "--input-method-arg=input_method_child_probe",
            "--input-method-arg=--nocapture",
        ])
        .env("XDG_RUNTIME_DIR", dir)
        .env("INOX_IME_PROBE_OUTPUT", output)
        .env_remove("INOX_IME_PROBE_HOLD")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if hold {
        command.env("INOX_IME_PROBE_HOLD", "1");
    }
    Host(command.spawn().unwrap())
}

fn stop(host: &mut Host) {
    assert!(
        Command::new("kill")
            .args(["-TERM", &host.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let mut status = None;
    wait_until(
        || {
            status = host.0.try_wait().unwrap();
            status.is_some()
        },
        Duration::from_secs(20),
    );
    assert!(status.unwrap().success());
}

#[test]
fn private_connection_restarts_are_bounded_and_children_are_reaped() {
    let runtime = tempfile::tempdir().unwrap();
    let output = runtime.path().join("attempts");
    let mut host = start(runtime.path(), &output, false);
    let count = || {
        fs::read_to_string(&output)
            .unwrap_or_default()
            .lines()
            .count()
    };
    wait_until(|| count() == 4, Duration::from_secs(45));
    // Beyond the largest retry delay: the exhausted service must stay stopped.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        assert_eq!(count(), 4);
        assert!(
            host.0.try_wait().unwrap().is_none(),
            "an IME failure must not stop the desktop"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    stop(&mut host);

    let runtime = tempfile::tempdir().unwrap();
    let output = runtime.path().join("live-child");
    let mut host = start(runtime.path(), &output, true);
    wait_until(
        || fs::read_to_string(&output).is_ok_and(|value| value.trim().parse::<u32>().is_ok()),
        Duration::from_secs(20),
    );
    let pid = fs::read_to_string(&output).unwrap().trim().to_owned();
    stop(&mut host);
    let status = Command::new("kill")
        .args(["-0", &pid])
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "compositor shutdown left an input method process behind"
    );
}
