//! Round-trip test for the `inox-ipc` protocol over a real Unix socket pair.

use std::{
    os::unix::net::UnixStream,
    sync::{
        Arc,
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

struct App {
    version: Option<u32>,
    reload_result: Option<u32>,
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
            _ => {}
        }
    }
}

#[derive(Default)]
struct ServerState;

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
        resource.version(1);
    }
}

impl ServerDispatch<server::inox_ipc::InoxIpc, ()> for ServerState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        resource: &server::inox_ipc::InoxIpc,
        request: server::inox_ipc::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            server::inox_ipc::Request::GetVersion => resource.version(1),
            server::inox_ipc::Request::ReloadConfig => {
                resource.config_reloaded(server::inox_ipc::ReloadResult::Success);
            }
            server::inox_ipc::Request::Destroy => {}
            _ => {}
        }
    }
}

struct ServerClient;

impl ClientData for ServerClient {}

#[test]
fn inox_ipc_round_trips_version_and_reload() {
    let (client_socket, server_socket) = UnixStream::pair().expect("socket pair");

    let mut display = Display::<ServerState>::new().expect("server display");
    let mut handle = display.handle();
    handle.create_global::<ServerState, server::inox_ipc::InoxIpc, ()>(1, ());
    handle
        .insert_client(server_socket, Arc::new(ServerClient))
        .expect("insert client");

    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let server = thread::spawn(move || {
        let mut state = ServerState;
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
    let inox: client::inox_ipc::InoxIpc =
        globals.bind(&qh, 1..=1, ()).expect("bind inox_ipc global");

    let mut app = App {
        version: None,
        reload_result: None,
    };

    inox.get_version();
    inox.reload_config();
    queue.roundtrip(&mut app).expect("roundtrip");

    assert_eq!(app.version, Some(1));
    assert_eq!(app.reload_result, Some(0));

    stop.store(true, Ordering::Relaxed);
    drop(inox);
    drop(connection);
    thread::sleep(Duration::from_millis(10));
    let _ = server.join();
}
