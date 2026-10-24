//! Round-trip test for the `inox-glass-v1` protocol over a real Unix socket pair.
//!
//! This drives a real server-side dispatch from a real `wayland-client`: a
//! `wl_surface` is created through `wl_compositor`, a glass object is created for
//! it, and the compositor's answer (`capabilities`) plus the parameters the client
//! sent are asserted on both sides — the values that crossed the socket, not what
//! either half believed.
//!
//! What this file is for: the protocol is new, so the parts that would otherwise
//! only be checked by reading the XML are checked mechanically — that the manager
//! hands out an object for a surface, that `capabilities` precedes any effect,
//! that the radius and region arrive verbatim (including a non-positive size,
//! which the protocol defines as "clear the region"), and that nothing but an
//! integer radius ever travels back to the client.

use std::{
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use inox_protocols::glass::inox_glass::{client, server};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_compositor, wl_registry, wl_surface},
};
use wayland_server::{
    Client, DataInit, Dispatch as ServerDispatch, Display, DisplayHandle, GlobalDispatch, New,
    backend::ClientData,
    protocol::{wl_compositor as swl_compositor, wl_surface as swl_surface},
};

/// The largest radius this fake compositor accepts, in logical pixels.
const MAX_RADIUS: u32 = 64;

/// The version both halves of this test speak.
const VERSION: u32 = 1;

/// One parameter a client sent for its pane, as the server observed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sent {
    /// A blur radius request.
    Radius(u32),
    /// A region request.
    Region {
        /// Region origin and size, surface-local.
        x: i32,
        /// Region origin and size, surface-local.
        y: i32,
        /// Region origin and size, surface-local.
        width: i32,
        /// Region origin and size, surface-local.
        height: i32,
    },
}

#[derive(Default)]
struct App {
    max_radius: Option<u32>,
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

impl Dispatch<wl_compositor::WlCompositor, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &wl_compositor::WlCompositor,
        _event: wl_compositor::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &wl_surface::WlSurface,
        _event: wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<client::inox_glass_manager_v1::InoxGlassManagerV1, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &client::inox_glass_manager_v1::InoxGlassManagerV1,
        _event: client::inox_glass_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<client::inox_glass_v1::InoxGlassV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &client::inox_glass_v1::InoxGlassV1,
        event: client::inox_glass_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let client::inox_glass_v1::Event::Capabilities { max_radius } = event {
            state.max_radius = Some(max_radius);
        }
    }
}

/// The server half: it records every pane parameter it is handed and answers the
/// capabilities event, so the assertions are about what crossed the socket.
#[derive(Default)]
struct ServerState {
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl GlobalDispatch<swl_compositor::WlCompositor, ()> for ServerState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<swl_compositor::WlCompositor>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl ServerDispatch<swl_compositor::WlCompositor, ()> for ServerState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &swl_compositor::WlCompositor,
        request: swl_compositor::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let swl_compositor::Request::CreateSurface { id } = request {
            data_init.init(id, ());
        }
    }
}

impl ServerDispatch<swl_surface::WlSurface, ()> for ServerState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &swl_surface::WlSurface,
        _request: swl_surface::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl GlobalDispatch<server::inox_glass_manager_v1::InoxGlassManagerV1, ()> for ServerState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<server::inox_glass_manager_v1::InoxGlassManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl ServerDispatch<server::inox_glass_manager_v1::InoxGlassManagerV1, ()> for ServerState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &server::inox_glass_manager_v1::InoxGlassManagerV1,
        request: server::inox_glass_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let server::inox_glass_manager_v1::Request::GetGlass { id, surface } = request {
            let _surface = surface;
            let glass = data_init.init(id, ());
            // The capabilities event is what tells a client how much blur it can
            // ask for; it must precede any effect the client relies on.
            glass.capabilities(MAX_RADIUS);
        }
    }
}

impl ServerDispatch<server::inox_glass_v1::InoxGlassV1, ()> for ServerState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &server::inox_glass_v1::InoxGlassV1,
        request: server::inox_glass_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let observed = match request {
            server::inox_glass_v1::Request::SetRadius { radius } => Sent::Radius(radius),
            server::inox_glass_v1::Request::SetRegion {
                x,
                y,
                width,
                height,
            } => Sent::Region {
                x,
                y,
                width,
                height,
            },
            server::inox_glass_v1::Request::Destroy => return,
            _ => return,
        };

        if let Ok(mut sent) = state.sent.lock() {
            sent.push(observed);
        }
    }
}

struct ServerClient;

impl ClientData for ServerClient {}

#[test]
fn inox_glass_round_trips_a_pane_request_for_a_surface() {
    let (client_socket, server_socket) = UnixStream::pair().expect("socket pair");

    let mut display = Display::<ServerState>::new().expect("server display");
    let mut handle = display.handle();
    handle.create_global::<ServerState, swl_compositor::WlCompositor, ()>(4, ());
    handle.create_global::<ServerState, server::inox_glass_manager_v1::InoxGlassManagerV1, ()>(
        VERSION,
        (),
    );
    handle
        .insert_client(server_socket, Arc::new(ServerClient))
        .expect("insert client");

    let sent: Arc<Mutex<Vec<Sent>>> = Arc::default();
    let server_sent = sent.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let server = thread::spawn(move || {
        let mut state = ServerState { sent: server_sent };
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
    let compositor: wl_compositor::WlCompositor =
        globals.bind(&qh, 4..=4, ()).expect("bind wl_compositor");
    let manager: client::inox_glass_manager_v1::InoxGlassManagerV1 = globals
        .bind(&qh, 1..=VERSION, ())
        .expect("bind inox_glass_manager_v1");

    let surface = compositor.create_surface(&qh, ());
    let glass = manager.get_glass(&surface, &qh, ());

    let mut app = App::default();
    queue.roundtrip(&mut app).expect("roundtrip");

    assert_eq!(
        app.max_radius,
        Some(MAX_RADIUS),
        "the compositor must report what it can frost before the client sets anything"
    );

    // A pane: the whole surface, blurred. These are the values a real panel sends.
    glass.set_radius(48);
    glass.set_region(0, 0, 1000, 28);
    queue.roundtrip(&mut app).expect("roundtrip");

    assert_eq!(
        sent.lock().expect("parameter log").as_slice(),
        [
            Sent::Radius(48),
            Sent::Region {
                x: 0,
                y: 0,
                width: 1000,
                height: 28,
            }
        ],
        "the radius and the region must reach the compositor verbatim"
    );

    // Clearing the region is expressed by a non-positive size, so a client can stop
    // frosting a pane without destroying the object. The wire carries the integer
    // the client sent; the compositor is the one that reads it as "nothing".
    glass.set_region(0, 0, 0, 0);
    queue.roundtrip(&mut app).expect("roundtrip");

    assert_eq!(
        sent.lock().expect("parameter log").last().copied(),
        Some(Sent::Region {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        })
    );

    stop.store(true, Ordering::Relaxed);
    drop(glass);
    drop(surface);
    drop(connection);
    thread::sleep(Duration::from_millis(10));
    let _ = server.join();
}
