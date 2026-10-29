//! `ext-idle-notify-v1`: a running compositor reports that the user has been away.
//!
//! This is the compositor half of the idle chain that `inox-idle` (L4) is the
//! other end of: the daemon asks for a notification, and the `idled` event it
//! gets back is what makes the session lock itself. Until the global existed the
//! daemon had nothing to subscribe to, so the test asserts the event a real
//! client waits for, on a real compositor process.
//!
//! Two notifications with very different timeouts are created on the same seat.
//! That is the point of the second one: it proves the deadline is per
//! notification and not one global "the desktop is idle" flag, which is the shape
//! a lazier implementation would have.

use std::{
    fs,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread::{self, sleep},
    time::{Duration, Instant},
};

#[cfg(feature = "test-input")]
use std::io::{Read, Write};

use inox_protocols::standard::{
    core::client::{wl_registry, wl_seat},
    ext::idle_notify::client::{
        ext_idle_notification_v1::{self, ExtIdleNotificationV1},
        ext_idle_notifier_v1::ExtIdleNotifierV1,
    },
};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry::WlRegistry,
};

/// The compositor binary built for this test run.
const COMPOSITOR: &str = env!("CARGO_BIN_EXE_inox-compositor");

/// How long the compositor may take to bind its socket before the test gives up.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the compositor may take to exit after being asked to.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the client interaction may take before the test gives up.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a zero-timeout notification may take to arrive.
///
/// The compositor checks its deadlines every 100 ms; ten seconds is slack for a
/// loaded CI machine, not a tolerance the implementation is allowed to use.
const IDLE_TIMEOUT: Duration = Duration::from_secs(10);

/// How many poll rounds the client runs after the event it waited for.
///
/// Each round is one roundtrip plus a sleep, so this covers roughly a second of
/// compositor time: long enough for a duplicate `idled` or a stray `resumed` to
/// arrive and fail the assertion below.
const SETTLE_ROUNDS: usize = 20;

/// How long the client waits for a roundtrip to come back.
const ROUNDTRIP: Duration = Duration::from_millis(20);

/// Which notification an event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    /// A notification with a zero timeout: idle immediately.
    Quick,
    /// A notification that must not fire during the test.
    Slow,
}

/// The two events `ext_idle_notification_v1` can send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    /// The seat has been inactive for at least the notification's timeout.
    Idled,
    /// Activity happened, so the timeout restarted.
    Resumed,
}

/// The idle events the client saw, tagged with the notification that sent them.
#[derive(Debug, Default)]
struct IdleClient {
    /// Every event, in arrival order.
    events: Vec<(Tag, Event)>,
}

impl Dispatch<WlRegistry, GlobalListContents> for IdleClient {
    fn event(
        _state: &mut Self,
        _registry: &WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        // `registry_queue_init` already captured the global list.
    }
}

impl Dispatch<ExtIdleNotificationV1, Tag> for IdleClient {
    fn event(
        state: &mut Self,
        _notification: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        tag: &Tag,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => state.events.push((*tag, Event::Idled)),
            ext_idle_notification_v1::Event::Resumed => state.events.push((*tag, Event::Resumed)),
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(IdleClient: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(IdleClient: ignore ExtIdleNotifierV1);

/// A private `XDG_RUNTIME_DIR` for one compositor process.
struct RuntimeDir(PathBuf);

impl RuntimeDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("inox-idle-{label}-{}", std::process::id()));

        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create the test runtime directory");

        Self(path)
    }

    fn member(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for RuntimeDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Start a compositor with a private runtime directory and a known socket name.
fn spawn(socket_name: &str, runtime_dir: &RuntimeDir) -> Child {
    let mut command = Command::new(COMPOSITOR);

    command.args(["--headless", "--socket", socket_name]);

    // The control socket is the only way to create input in a compositor that has
    // no libinput backend, and the feature that compiles it is off by default.
    // Without it the test still proves the `idled` half.
    #[cfg(feature = "test-input")]
    command
        .arg("--test-input")
        .arg(runtime_dir.member("control.sock"));

    command
        .env("XDG_RUNTIME_DIR", &runtime_dir.0)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn inox-compositor")
}

/// Block until the compositor has bound its Wayland socket.
fn wait_for_socket(path: &Path) {
    let deadline = Instant::now() + STARTUP_TIMEOUT;

    while Instant::now() < deadline {
        if path.exists() {
            return;
        }

        sleep(Duration::from_millis(20));
    }

    panic!(
        "inox-compositor did not bind {} within {STARTUP_TIMEOUT:?}",
        path.display()
    );
}

/// Wait for the compositor to exit, killing it if it takes too long.
fn wait_with_timeout(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;

    loop {
        if let Some(status) = child.try_wait().expect("failed to poll inox-compositor") {
            return status;
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();

            panic!("inox-compositor did not exit within {SHUTDOWN_TIMEOUT:?}");
        }

        sleep(Duration::from_millis(20));
    }
}

/// Send `SIGTERM` to the compositor.
fn send_sigterm(child: &Child) {
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("failed to run kill");

    assert!(status.success(), "kill -TERM exited with {status}");
}

/// Dispatch until `predicate` holds or `limit` passes, and report whether it held.
fn drive_until(
    queue: &mut wayland_client::EventQueue<IdleClient>,
    client: &mut IdleClient,
    limit: Duration,
    predicate: impl Fn(&IdleClient) -> bool,
) -> anyhow::Result<bool> {
    let deadline = Instant::now() + limit;

    while Instant::now() < deadline {
        queue.roundtrip(client)?;

        if predicate(client) {
            return Ok(true);
        }

        sleep(ROUNDTRIP);
    }

    queue.roundtrip(client)?;

    Ok(predicate(client))
}

/// Create the two notifications and watch what they report.
fn watch_idle(connection: &Connection, control: Option<&Path>) -> anyhow::Result<IdleClient> {
    let mut client = IdleClient::default();
    let (globals, mut queue) = registry_queue_init::<IdleClient>(connection)?;
    let queue_handle = queue.handle();

    let seat: wl_seat::WlSeat = globals.bind(&queue_handle, 1..=1, ())?;
    let notifier: ExtIdleNotifierV1 = globals.bind(&queue_handle, 1..=1, ())?;

    // Zero is a valid timeout and means "as soon as the seat is inactive".
    let _quick = notifier.get_idle_notification(0, &seat, &queue_handle, Tag::Quick);
    let _slow = notifier.get_idle_notification(60_000, &seat, &queue_handle, Tag::Slow);

    let idled = drive_until(&mut queue, &mut client, IDLE_TIMEOUT, |client| {
        client
            .events
            .iter()
            .any(|(tag, event)| *tag == Tag::Quick && *event == Event::Idled)
    })?;

    assert!(
        idled,
        "no `idled` arrived within {IDLE_TIMEOUT:?}; the compositor saw {:?}",
        client.events
    );

    // The notification that asked for a minute must still be quiet: a compositor
    // that fires every notification as soon as the seat is idle would fail here.
    assert!(
        !client.events.iter().any(|(tag, _)| *tag == Tag::Slow),
        "the 60 s notification fired early: {:?}",
        client.events
    );

    // Input restarts the countdown, and a client that has already been told the
    // seat is idle has to be told it is not any more — that is the event
    // `inox-idle` uses to cancel a lock it was about to request.
    if let Some(control) = control {
        press_a_key(control)?;

        let resumed = drive_until(&mut queue, &mut client, IDLE_TIMEOUT, |client| {
            client
                .events
                .iter()
                .any(|(tag, event)| *tag == Tag::Quick && *event == Event::Resumed)
        })?;

        assert!(
            resumed,
            "input did not resume the idle notification; the compositor saw {:?}",
            client.events
        );
    }

    // Give a duplicate `idled` (a protocol error) time to show up: every event is
    // still in `events`, and the assertions below read the whole list.
    let _ = drive_until(
        &mut queue,
        &mut client,
        ROUNDTRIP * SETTLE_ROUNDS as u32,
        |_| false,
    );

    Ok(client)
}

/// Send one key press over the test-only control socket.
///
/// `KEY_Q` (`16`) is a key no compositor chord is bound to in a headless run, so
/// the press is activity and nothing else.
#[cfg(feature = "test-input")]
fn press_a_key(control: &Path) -> anyhow::Result<()> {
    let mut stream = UnixStream::connect(control)?;

    stream.write_all(b"key 16 press\n")?;
    stream.flush()?;

    let mut reply = [0_u8; 16];
    let read = stream.read(&mut reply)?;
    let reply = String::from_utf8_lossy(&reply[..read]);

    anyhow::ensure!(
        reply.trim() == "ok",
        "the compositor answered the key press with {reply:?}"
    );

    Ok(())
}

/// Without the control socket there is no way to create input, so this is a no-op.
#[cfg(not(feature = "test-input"))]
fn press_a_key(_control: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[test]
fn an_idle_compositor_reports_the_idle_notification_its_client_asked_for() {
    let runtime_dir = RuntimeDir::new("notify");
    let socket = runtime_dir.member("inox-idle-notify");

    let mut child = spawn("inox-idle-notify", &runtime_dir);
    wait_for_socket(&socket);

    #[cfg(feature = "test-input")]
    let control = Some(runtime_dir.member("control.sock"));
    #[cfg(not(feature = "test-input"))]
    let control: Option<PathBuf> = None;

    let (sender, receiver) = mpsc::channel();
    let socket_for_client = socket.clone();
    let client = thread::spawn(move || {
        let result = (|| -> anyhow::Result<IdleClient> {
            let stream = UnixStream::connect(&socket_for_client)?;
            let connection = Connection::from_socket(stream)?;

            watch_idle(&connection, control.as_deref())
        })();

        let _ = sender.send(result);
    });

    let outcome = match receiver.recv_timeout(CLIENT_TIMEOUT) {
        Ok(result) => result.expect("the idle client interaction failed"),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();

            panic!("the idle client did not finish within {CLIENT_TIMEOUT:?}");
        }
    };

    // The compositor must stay healthy after serving a client that idled.
    send_sigterm(&child);
    let status = wait_with_timeout(&mut child);

    assert_eq!(
        status.code(),
        Some(0),
        "expected a clean exit after serving the idle client, got {status}"
    );

    client.join().expect("the idle client thread panicked");

    // The protocol forbids two `idled` events without a `resumed` in between, and
    // forbids a `resumed` that no `idled` preceded.
    let mut idle = false;
    for (tag, event) in &outcome.events {
        match event {
            Event::Idled => {
                assert!(!idle, "two `idled` events without a `resumed`: {outcome:?}");
                assert_eq!(*tag, Tag::Quick, "the slow notification fired: {outcome:?}");
                idle = true;
            }
            Event::Resumed => {
                assert!(idle, "a `resumed` without an `idled`: {outcome:?}");
                idle = false;
            }
        }
    }
}
