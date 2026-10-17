//! A real Wayland wire peer exercises capability negotiation and idle/resume events.
use inox_protocols::standard::{
    core::server::wl_seat,
    ext::idle_notify::server::{ext_idle_notification_v1, ext_idle_notifier_v1},
};
use std::{
    os::unix::net::UnixListener,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, backend::ClientData,
};

#[derive(Default)]
struct Server {
    notification: Option<ext_idle_notification_v1::ExtIdleNotificationV1>,
}
struct ClientState;
impl ClientData for ClientState {}
impl GlobalDispatch<wl_seat::WlSeat, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_seat::WlSeat>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_seat::WlSeat,
        _: wl_seat::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
impl GlobalDispatch<ext_idle_notifier_v1::ExtIdleNotifierV1, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ext_idle_notifier_v1::ExtIdleNotifierV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}
impl Dispatch<ext_idle_notifier_v1::ExtIdleNotifierV1, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ext_idle_notifier_v1::ExtIdleNotifierV1,
        request: ext_idle_notifier_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let ext_idle_notifier_v1::Request::GetIdleNotification { id, timeout, .. } = request {
            assert_eq!(timeout, 1000);
            state.notification = Some(init.init(id, ()));
        }
    }
}
impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        _: ext_idle_notification_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

#[test]
fn notifier_events_reach_policy_and_missing_capabilities_fail_closed() {
    for supported in [true, false] {
        let root = tempfile::tempdir().expect("runtime");
        let listener = UnixListener::bind(root.path().join("wayland-test")).expect("socket");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let mut display = Display::<Server>::new().expect("display");
        let mut handle = display.handle();
        handle.create_global::<Server, wl_seat::WlSeat, ()>(1, ());
        if supported {
            handle.create_global::<Server, ext_idle_notifier_v1::ExtIdleNotifierV1, ()>(1, ());
        }
        let log_path = root.path().join("idle.log");
        let log = std::fs::File::create(&log_path).expect("log");
        let mut child = Command::new(env!("CARGO_BIN_EXE_inox-idle"))
            .args(["--timeout-seconds", "1"])
            .env("XDG_RUNTIME_DIR", root.path())
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("WAYLAND_DISPLAY", "wayland-test")
            .env("LANG", "en-US")
            .env("LC_ALL", "en-US")
            .env("RUST_LOG", "info")
            .env_remove("WAYLAND_SOCKET")
            .stdout(log.try_clone().expect("log clone"))
            .stderr(log)
            .spawn()
            .expect("idle daemon");
        let mut state = Server::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut sent = false;
        let mut signalled = false;
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    handle
                        .insert_client(stream, Arc::new(ClientState))
                        .expect("client");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("{error}"),
            }
            display.dispatch_clients(&mut state).expect("dispatch");
            if let Some(notification) = &state.notification
                && !sent
            {
                notification.idled();
                notification.resumed();
                notification.idled();
                sent = true;
            }
            display.flush_clients().expect("flush");
            let log = std::fs::read_to_string(&log_path).expect("log");
            if supported
                && !signalled
                && log.matches("Automatic locking is unavailable").count() == 2
            {
                use rustix::process::{Pid, Signal, kill_process};
                kill_process(Pid::from_raw(child.id() as i32).expect("pid"), Signal::TERM)
                    .expect("shutdown");
                signalled = true;
            }
            if let Some(status) = child.try_wait().expect("status") {
                assert_eq!(status.success(), supported, "{log}");
                if supported {
                    assert!(sent && signalled);
                    assert!(log.contains("User activity resumed"));
                } else {
                    assert!(log.contains("ext_idle_notifier_v1"), "{log}");
                }
                break;
            }
            if Instant::now() >= deadline {
                child.kill().expect("stop timed out client");
                child.wait().expect("reap client");
                panic!("idle protocol timed out: {log}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
