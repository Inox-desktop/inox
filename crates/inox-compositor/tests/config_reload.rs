//! Real-process reload, failure retention and SIGHUP pixel evidence.
#![cfg(feature = "test-input")]

use inox_protocols::ipc::inox_ipc::client::inox_ipc::{Event, InoxIpc, ReloadResult};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};

const TIMEOUT: Duration = Duration::from_secs(30);
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[derive(Default)]
struct Client {
    reloads: Vec<ReloadResult>,
}
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
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
impl Dispatch<InoxIpc, ()> for Client {
    fn event(
        state: &mut Self,
        _: &InoxIpc,
        event: Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let Event::ConfigReloaded { result } = event {
            state
                .reloads
                .push(result.into_result().expect("known result"));
        }
    }
}
fn pixel(control: &mut UnixStream, file: &Path) -> Vec<u8> {
    writeln!(control, "dump-frame {}", file.display()).expect("dump command");
    let mut reply = String::new();
    BufReader::new(control)
        .read_line(&mut reply)
        .expect("bounded control reply");
    assert_eq!(reply, "ok\n");
    let ppm = std::fs::read(file).expect("PPM");
    let start = ppm
        .iter()
        .enumerate()
        .filter(|(_, byte)| **byte == b'\n')
        .nth(2)
        .expect("PPM header")
        .0
        + 1;
    ppm[start..start + 3].to_vec()
}
fn near(actual: &[u8], expected: &[u8]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, b)| a.abs_diff(*b) <= 1)
}
fn signal(child: &Child, name: &str) {
    assert!(
        Command::new("kill")
            .arg(name)
            .arg(child.id().to_string())
            .status()
            .expect("kill")
            .success()
    );
}

#[test]
fn reload_changes_pixels_without_restarting_and_bad_candidates_preserve_the_last_frame() {
    let dir = tempfile::tempdir().expect("directory");
    let config_dir = dir.path().join("inox");
    std::fs::create_dir_all(&config_dir).expect("config directory");
    let control_path = dir.path().join("control");
    let child = Command::new(env!("CARGO_BIN_EXE_inox-compositor"))
        .args([
            "--headless",
            "--socket",
            "reload-test",
            "--output-size",
            "64x64",
            "--test-input",
        ])
        .arg(&control_path)
        .env("XDG_RUNTIME_DIR", dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("compositor");
    let mut process = Process(child);
    let deadline = Instant::now() + TIMEOUT;
    while !control_path.exists() || !dir.path().join("reload-test").exists() {
        assert!(Instant::now() < deadline, "compositor startup timed out");
        assert!(
            process.0.try_wait().expect("status").is_none(),
            "compositor exited during startup"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let socket = dir.path().join("reload-test");
    let (commands, requests) = mpsc::channel();
    let (answers, results) = mpsc::channel();
    let client = std::thread::spawn(move || {
        let connection = Connection::from_socket(UnixStream::connect(socket).expect("socket"))
            .expect("connection");
        let (globals, mut queue) = registry_queue_init::<Client>(&connection).expect("registry");
        let ipc: InoxIpc = globals.bind(&queue.handle(), 1..=2, ()).expect("IPC");
        let mut state = Client::default();
        for () in requests {
            ipc.reload_config();
            connection.flush().expect("flush");
            let count = state.reloads.len();
            while state.reloads.len() == count {
                queue.blocking_dispatch(&mut state).expect("dispatch");
            }
            answers.send(state.reloads[count]).expect("answer");
        }
    });
    let mut control = UnixStream::connect(control_path).expect("control");
    control.set_read_timeout(Some(TIMEOUT)).expect("timeout");
    control.set_write_timeout(Some(TIMEOUT)).expect("timeout");
    let frame = dir.path().join("frame.ppm");
    std::fs::write(
        config_dir.join("config.toml"),
        "[theme]\nappearance = 'light'\n",
    )
    .expect("light theme");
    commands.send(()).expect("reload");
    assert_eq!(
        results.recv_timeout(TIMEOUT).expect("reload result"),
        ReloadResult::Success
    );
    let light = inox_config::Theme::builtin_light().colors.background;
    let light_rgb = vec![light.r, light.g, light.b];
    assert!(near(&pixel(&mut control, &frame), &light_rgb));
    std::fs::write(
        config_dir.join("compositor.toml"),
        "[layout]\ngap = 'broken'\n",
    )
    .expect("bad candidate");
    commands.send(()).expect("reload");
    assert_eq!(
        results.recv_timeout(TIMEOUT).expect("rejection"),
        ReloadResult::Rejected
    );
    assert_eq!(
        pixel(&mut control, &frame),
        light_rgb,
        "parse failure must keep the visible state"
    );
    std::fs::write(
        config_dir.join("compositor.toml"),
        "[input]\nkeyboard_layout = 'inox-no-such-layout'\n",
    )
    .expect("bad keymap");
    commands.send(()).expect("reload");
    assert_eq!(
        results.recv_timeout(TIMEOUT).expect("rejection"),
        ReloadResult::Rejected
    );
    assert_eq!(
        pixel(&mut control, &frame),
        light_rgb,
        "device preparation failure must preserve the theme"
    );
    std::fs::write(config_dir.join("compositor.toml"), "[layout]\ngap = 24\n").expect("recovery");
    std::fs::write(
        config_dir.join("config.toml"),
        "[theme]\nappearance = 'dark'\n",
    )
    .expect("dark theme");
    signal(&process.0, "-HUP");
    let dark = inox_config::Theme::builtin_dark().colors.background;
    let dark_rgb = vec![dark.r, dark.g, dark.b];
    let deadline = Instant::now() + TIMEOUT;
    while !near(&pixel(&mut control, &frame), &dark_rgb) {
        assert!(
            Instant::now() < deadline,
            "SIGHUP did not apply the new theme"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // The same Wayland connection remains usable after every transition.
    commands.send(()).expect("reload");
    assert_eq!(
        results.recv_timeout(TIMEOUT).expect("recovered result"),
        ReloadResult::Success
    );
    drop(commands);
    client.join().expect("client");
    signal(&process.0, "-TERM");
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(status) = process.0.try_wait().expect("exit") {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "shutdown timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}
