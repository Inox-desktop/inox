//! Round-trip test for the `inox-ipc` protocol over a real Unix socket pair.
//!
//! This drives the composer's real server-side dispatch (v2) from a real
//! `wayland-client`: the version handshake, a configuration reload, the
//! trusted-lock-role claim, and the session lock state the authority broadcasts.
//! A client that never claims the role is the compositor's business to refuse
//! (see `inox-compositor`'s session-lock tests); what this file proves is that
//! the token reaches the server and that the state event reaches the client.

use std::{
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use inox_protocols::ipc::inox_ipc::{client, server};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};
use wayland_server::{
    Client, DataInit, Dispatch as ServerDispatch, Display, DisplayHandle, GlobalDispatch, New,
    backend::ClientData,
};

/// The version both halves of this test speak. The interface is at v2.
const VERSION: u32 = 2;

#[derive(Default)]
struct App {
    version: Option<u32>,
    reload_result: Option<u32>,
    lock_state: Option<client::inox_ipc::LockState>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<client::inox_ipc::InoxIpc, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &client::inox_ipc::InoxIpc,
        event: client::inox_ipc::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            client::inox_ipc::Event::Version { version } => state.version = Some(version),
            client::inox_ipc::Event::ConfigReloaded { result } => {
                state.reload_result = Some(result.into());
            }
            client::inox_ipc::Event::SessionLockState { state: value } => {
                // A `WEnum` would carry an unknown discriminant verbatim; this test
                // wants the value the compositor actually sent, so it refuses one it
                // cannot name rather than comparing raw integers.
                state.lock_state = value.into_result().ok();
            }
            _ => {}
        }
    }
}

/// The server half: it records every token it is asked to accept and echoes the
/// lock state it is told to report, so the assertions are about what crossed the
/// socket rather than about what either side believed.
#[derive(Default)]
struct ServerState {
    claimed: Arc<Mutex<Vec<String>>>,
}

impl GlobalDispatch<server::inox_ipc::InoxIpc, ()> for ServerState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<server::inox_ipc::InoxIpc>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let resource = data_init.init(resource, ());
        resource.version(VERSION);
        resource.session_lock_state(server::inox_ipc::LockState::Unlocked);
    }
}

impl ServerDispatch<server::inox_ipc::InoxIpc, ()> for ServerState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &server::inox_ipc::InoxIpc,
        request: server::inox_ipc::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            server::inox_ipc::Request::GetVersion => resource.version(VERSION),
            server::inox_ipc::Request::ReloadConfig => {
                resource.config_reloaded(server::inox_ipc::ReloadResult::Success);
            }
            server::inox_ipc::Request::ClaimLockRole { token } => {
                // The compositor decides here; the test only records the token so
                // the assertion can be about the bytes that arrived.
                if let Ok(mut claimed) = state.claimed.lock() {
                    claimed.push(token);
                }
                resource.session_lock_state(server::inox_ipc::LockState::Locked);
            }
            server::inox_ipc::Request::Destroy => {}
            _ => {}
        }
    }
}

struct ServerClient;

impl ClientData for ServerClient {}

#[test]
fn inox_ipc_round_trips_version_reload_and_the_lock_role_claim() {
    let (client_socket, server_socket) = UnixStream::pair().expect("socket pair");

    let mut display = Display::<ServerState>::new().expect("server display");
    let mut handle = display.handle();
    handle.create_global::<ServerState, server::inox_ipc::InoxIpc, ()>(VERSION, ());
    handle
        .insert_client(server_socket, Arc::new(ServerClient))
        .expect("insert client");

    let claimed: Arc<Mutex<Vec<String>>> = Arc::default();
    let server_claimed = claimed.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let server = thread::spawn(move || {
        let mut state = ServerState {
            claimed: server_claimed,
        };
        while !server_stop.load(Ordering::Relaxed) {
            if display.dispatch_clients(&mut state).is_err() {
                break;
            }
            if display.flush_clients().is_err() {
                break;
            }
        }
    });

    let connection = Connection::from_socket(client_socket).expect("client connection");
    let (globals, mut queue) = registry_queue_init::<App>(&connection).expect("registry");
    let qh = queue.handle();
    let inox: client::inox_ipc::InoxIpc = globals
        .bind(&qh, 1..=VERSION, ())
        .expect("bind inox_ipc global");

    let mut app = App::default();

    inox.get_version();
    inox.reload_config();
    queue.roundtrip(&mut app).expect("roundtrip");

    assert_eq!(app.version, Some(VERSION));
    assert_eq!(app.reload_result, Some(0));
    assert_eq!(
        app.lock_state,
        Some(client::inox_ipc::LockState::Unlocked),
        "the bound client is told the state it is actually in"
    );

    inox.claim_lock_role("session-lock-token".to_owned());
    queue.roundtrip(&mut app).expect("roundtrip");

    assert_eq!(
        claimed.lock().expect("claim log").as_slice(),
        ["session-lock-token"],
        "the token must reach the server verbatim"
    );
    assert_eq!(
        app.lock_state,
        Some(client::inox_ipc::LockState::Locked),
        "a state change is broadcast to the client that is already bound"
    );

    stop.store(true, Ordering::Relaxed);
    drop(inox);
    drop(connection);
    thread::sleep(Duration::from_millis(10));
    let _ = server.join();
}
