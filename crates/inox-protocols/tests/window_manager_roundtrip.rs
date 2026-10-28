//! Round-trip test for the `inox-window-manager-v1` protocol over a real Unix
//! socket pair.
//!
//! This drives a real server-side dispatch from a real `wayland-client` and
//! asserts the values that crossed the socket, not what either half believed:
//!
//! - **the snapshot order** — `session_generation`, then `workspace_state`, then
//!   one `window` per managed window in the order the compositor sent them, then
//!   `snapshot_done`. A panel that received the handles before the generation
//!   could match an old handle against a new window, so the order is part of the
//!   contract and is asserted as one.
//! - **the window record** — every field of the `window` event, decoded in the
//!   order the panel decodes it: handle, app id, title, workspace, output, mode,
//!   minimized/visible/focused. This is the panel's whole model.
//! - **the verbs** — activate, minimize, close, set mode, move to workspace and
//!   switch workspace all reach the server with their arguments verbatim.
//! - **the negative outcome** — `request_failed` reaches the client carrying the
//!   handle the failed request named and the reason, which is what a task button
//!   logs instead of hanging on a request that was never carried out.
//!
//! Nothing here reads pixels: the events carry handles, strings and enumerations
//! only, which is the property the protocol promises.

use std::{
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use inox_protocols::window_manager::inox_window_manager::{client, server};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};
use wayland_server::{
    Client, DataInit, Dispatch as ServerDispatch, Display, DisplayHandle, GlobalDispatch, New,
};

/// The version both halves of this test speak.
const VERSION: u32 = 1;

/// One request as the server received it, so the assertions are about the bytes
/// that arrived rather than about the client's intentions.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Received {
    Activate(u32),
    Minimize(u32),
    Close(u32),
    SetMode(u32, u32),
    MoveToWorkspace(u32, u32),
    SwitchWorkspace(u32),
}

/// One event as the client received it, in arrival order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    SessionGeneration(u32),
    WorkspaceState { active: u32, count: u32 },
    Window {
        window: u32,
        app_id: String,
        title: String,
        workspace: u32,
        output: String,
        mode: u32,
        minimized: bool,
        visible: bool,
        focused: bool,
    },
    WindowRemoved(u32),
    SnapshotDone,
    RequestFailed { window: u32, reason: u32 },
}

#[derive(Default)]
struct App {
    seen: Vec<Seen>,
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

impl Dispatch<client::inox_window_manager_v1::InoxWindowManagerV1, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &client::inox_window_manager_v1::InoxWindowManagerV1,
        _event: <client::inox_window_manager_v1::InoxWindowManagerV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<client::inox_window_list_v1::InoxWindowListV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &client::inox_window_list_v1::InoxWindowListV1,
        event: client::inox_window_list_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use client::inox_window_list_v1::Event;

        match event {
            Event::SessionGeneration { generation } => {
                state.seen.push(Seen::SessionGeneration(generation));
            }
            Event::WorkspaceState { active, count } => {
                state.seen.push(Seen::WorkspaceState { active, count });
            }
            Event::Window {
                window,
                app_id,
                title,
                workspace,
                output,
                mode,
                minimized,
                visible,
                focused,
            } => {
                state.seen.push(Seen::Window {
                    window,
                    app_id,
                    title,
                    workspace,
                    output,
                    // The mode is a `WEnum`; an unknown discriminant is refused
                    // rather than rendered as a wrong number, which is the same
                    // rule the panel applies.
                    mode: mode.into_result().expect("a known window mode") as u32,
                    minimized: minimized != 0,
                    visible: visible != 0,
                    focused: focused != 0,
                });
            }
            Event::WindowRemoved { window } => state.seen.push(Seen::WindowRemoved(window)),
            Event::SnapshotDone => state.seen.push(Seen::SnapshotDone),
            Event::RequestFailed { window, reason } => {
                // Same rule for the error enum.
                let reason = reason.into_result().expect("a known request error") as u32;
                state
                    .seen
                    .push(Seen::RequestFailed { window, reason });
            }
            _ => {}
        }
    }
}

/// The server half: it records every request it receives and is told, by the
/// test, which events to send.
#[derive(Default)]
struct ServerState {
    received: Arc<Mutex<Vec<Received>>>,
    /// The list object to send events on, once the client created it.
    list: Arc<Mutex<Option<server::inox_window_list_v1::InoxWindowListV1>>>,
}

impl GlobalDispatch<server::inox_window_manager_v1::InoxWindowManagerV1, ()> for ServerState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<server::inox_window_manager_v1::InoxWindowManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl ServerDispatch<server::inox_window_manager_v1::InoxWindowManagerV1, ()> for ServerState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &server::inox_window_manager_v1::InoxWindowManagerV1,
        request: server::inox_window_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            server::inox_window_manager_v1::Request::GetWindowList { id } => {
                let list = data_init.init(id, ());
                if let Ok(mut slot) = state.list.lock() {
                    *slot = Some(list);
                }
            }
            server::inox_window_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl ServerDispatch<server::inox_window_list_v1::InoxWindowListV1, ()> for ServerState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &server::inox_window_list_v1::InoxWindowListV1,
        request: server::inox_window_list_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use server::inox_window_list_v1::Request;

        let received = match request {
            Request::Activate { window } => Received::Activate(window),
            Request::Minimize { window } => Received::Minimize(window),
            Request::Close { window } => Received::Close(window),
            Request::SetMode { window, mode } => {
                // The mode arrives as a `WEnum`; the test wants the raw value the
                // client sent, so it reads it back out of the enum.
                Received::SetMode(window, u32::from(mode))
            }
            Request::MoveToWorkspace { window, workspace } => {
                Received::MoveToWorkspace(window, workspace)
            }
            Request::SwitchWorkspace { workspace } => Received::SwitchWorkspace(workspace),
            Request::Destroy => return,
            _ => return,
        };

        if let Ok(mut log) = state.received.lock() {
            log.push(received);
        }
    }
}

struct ServerClient;

impl wayland_server::backend::ClientData for ServerClient {}

#[test]
fn inox_window_manager_round_trips_the_snapshot_and_every_verb() {
    let (client_socket, server_socket) = UnixStream::pair().expect("socket pair");

    let mut display = Display::<ServerState>::new().expect("server display");
    let mut handle = display.handle();
    handle.create_global::<ServerState, server::inox_window_manager_v1::InoxWindowManagerV1, ()>(
        VERSION,
        (),
    );
    handle
        .insert_client(server_socket, Arc::new(ServerClient))
        .expect("insert client");

    let received: Arc<Mutex<Vec<Received>>> = Arc::default();
    let list_slot: Arc<Mutex<Option<server::inox_window_list_v1::InoxWindowListV1>>> =
        Arc::default();
    let stop = Arc::new(AtomicBool::new(false));

    let server_received = received.clone();
    let server_list = list_slot.clone();
    let server_stop = stop.clone();
    let server = thread::spawn(move || {
        let mut state = ServerState {
            received: server_received,
            list: server_list,
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
    let manager: client::inox_window_manager_v1::InoxWindowManagerV1 = globals
        .bind(&qh, 1..=VERSION, ())
        .expect("bind inox_window_manager_v1 global");
    let list = manager.get_window_list(&qh, ());

    let mut app = App::default();
    // The client's request has to reach the server before the server can send
    // anything on the list object it creates.
    queue.roundtrip(&mut app).expect("roundtrip");

    // The server now owns the list object; send the whole snapshot shape.
    let mut server_list = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while server_list.is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the server never received get_window_list"
        );
        server_list = list_slot.lock().expect("the list slot").clone();
        thread::sleep(Duration::from_millis(2));
    }
    let server_list = server_list.expect("the list object");

    server_list.session_generation(7);
    server_list.workspace_state(1, 3);
    server_list.window(
        42,
        "org.example.Editor".to_owned(),
        "notes.txt".to_owned(),
        1,
        "eDP-1".to_owned(),
        server::inox_window_list_v1::WindowMode::Maximized,
        0,
        1,
        1,
    );
    server_list.window(
        43,
        String::new(),
        String::new(),
        0,
        String::new(),
        server::inox_window_list_v1::WindowMode::Tiled,
        1,
        0,
        0,
    );
    server_list.snapshot_done();

    queue.roundtrip(&mut app).expect("roundtrip");

    assert_eq!(
        app.seen,
        vec![
            // The generation comes before any handle, and every workspace fact
            // before the windows that live on them.
            Seen::SessionGeneration(7),
            Seen::WorkspaceState {
                active: 1,
                count: 3
            },
            Seen::Window {
                window: 42,
                app_id: "org.example.Editor".to_owned(),
                title: "notes.txt".to_owned(),
                workspace: 1,
                output: "eDP-1".to_owned(),
                mode: u32::from(server::inox_window_list_v1::WindowMode::Maximized),
                minimized: false,
                visible: true,
                focused: true,
            },
            // An empty app id, title and output are legal values, not errors:
            // the panel draws its "unknown application" group for them.
            Seen::Window {
                window: 43,
                app_id: String::new(),
                title: String::new(),
                workspace: 0,
                output: String::new(),
                mode: u32::from(server::inox_window_list_v1::WindowMode::Tiled),
                minimized: true,
                visible: false,
                focused: false,
            },
            Seen::SnapshotDone,
        ],
        "the snapshot must arrive in the order the panel decodes it"
    );

    // Every verb, with its arguments, over the real socket.
    list.activate(42);
    list.minimize(42);
    list.close(42);
    list.set_mode(42, client::inox_window_list_v1::WindowMode::Fullscreen);
    list.move_to_workspace(42, 2);
    list.switch_workspace(2);
    queue.roundtrip(&mut app).expect("roundtrip");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while received.lock().map(|log| log.len()).unwrap_or(0) < 6 {
        assert!(
            std::time::Instant::now() < deadline,
            "the server did not receive every verb: {:?}",
            received.lock().map(|log| log.clone()).unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(2));
    }

    assert_eq!(
        received.lock().expect("request log").as_slice(),
        [
            Received::Activate(42),
            Received::Minimize(42),
            Received::Close(42),
            Received::SetMode(
                42,
                u32::from(server::inox_window_list_v1::WindowMode::Fullscreen)
            ),
            Received::MoveToWorkspace(42, 2),
            Received::SwitchWorkspace(2),
        ],
        "every verb reaches the server with its arguments verbatim"
    );

    // The negative outcome is a first-class event: a task button that sent a
    // request for a window that just closed learns why instead of waiting.
    // Events arrive in the order the server sent them, which is what lets the
    // panel treat the stream as a single ordered log.
    server_list.window_removed(43);
    server_list.request_failed(
        42,
        server::inox_window_list_v1::RequestError::UnknownWindow,
    );
    queue.roundtrip(&mut app).expect("roundtrip");

    assert_eq!(
        &app.seen[5..],
        [
            Seen::WindowRemoved(43),
            Seen::RequestFailed {
                window: 42,
                reason: u32::from(server::inox_window_list_v1::RequestError::UnknownWindow),
            },
        ],
        "a removed window and a refused request are both reported"
    );

    stop.store(true, Ordering::Relaxed);
    drop(list);
    drop(manager);
    drop(connection);
    thread::sleep(Duration::from_millis(10));
    let _ = server.join();
}
