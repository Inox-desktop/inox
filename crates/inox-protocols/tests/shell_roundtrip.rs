//! Round-trip test for the `inox-shell-v1` protocol over a real Unix socket pair.
//!
//! This drives a real server-side dispatch from a real `wayland-client`: a
//! `wl_surface` is created through `wl_compositor`, gains a shell role, is
//! placed, opens a popup which opens a submenu, and the whole conversation is
//! asserted on both sides — the values that crossed the socket, not what either
//! half believed.
//!
//! What this file proves and what it does not. It proves the *protocol*: that a
//! role can only be assigned once, that placement arrives as one atomic request
//! with every field intact, that a popup is positioned, sized, made dismissable
//! and nested through the same object type, and that `dismissed` / `workarea` /
//! `keyboard_state` travel with the values the compositor sent. It does **not**
//! prove that a compositor honours any of it: advertising the global, reserving
//! the exclusive zone, drawing the transition and dismissing on an outside click
//! are the compositor's implementation (ADR 0046, phase P1) and have their own
//! evidence there. A round trip is necessary and not sufficient.

use std::{
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use inox_protocols::shell::inox_shell::{client, server};
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

/// The capability bits this fake compositor claims: transitions, popup
/// keyboards, the work-area event and nested popups — every bit the protocol
/// defines at version 1.
const CAPABILITIES: u32 = 1 | 2 | 4 | 8;

/// The longest transition this fake compositor will animate, in milliseconds.
const MAX_TRANSITION: u32 = 240;

/// The work area the fake compositor reports, in output-logical pixels.
const WORKAREA: (i32, i32, i32, i32) = (0, 32, 1920, 1048);

/// One request a client sent, as the server observed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sent {
    /// `get_surface`, with the role that was requested.
    Role(u32),
    /// `set_placement`, with every field the request carries.
    Placement {
        /// The shell layer.
        layer: u32,
        /// The anchor bitfield.
        anchor: u32,
        /// The strip to reserve.
        exclusive_zone: i32,
        /// The four margins, in the protocol's order.
        margins: [i32; 4],
        /// The requested size, where 0 means "the compositor decides".
        size: [u32; 2],
    },
    /// `set_keyboard` on a shell surface or a popup.
    Keyboard(u32),
    /// `set_transition` on a shell surface.
    Transition {
        /// Enter and exit effects, in the protocol's order.
        effects: [u32; 2],
        /// Duration in milliseconds.
        duration: u32,
        /// Timing curve.
        curve: u32,
        /// The anchor the motion grows from.
        origin: u32,
    },
    /// A popup was created; `nested` is true when its parent was another popup.
    Popup {
        /// Whether this popup's parent is a popup rather than a shell surface.
        nested: bool,
    },
    /// `set_position` on a popup.
    Position {
        /// The parent's anchor bitfield.
        parent_anchor: u32,
        /// The popup's anchor bitfield.
        popup_anchor: u32,
        /// The offset, popup-logical.
        offset: [i32; 2],
    },
    /// `set_size` on a popup.
    Size([u32; 2]),
    /// `set_dismissable`.
    Dismissable(u32),
    /// `dismiss`, which is the client asking the compositor to close the popup.
    Dismiss,
}

/// The client half: it records the events it was sent.
#[derive(Default)]
struct App {
    capabilities: Option<(u32, u32)>,
    configures: Vec<(u32, u32, u32)>,
    workarea: Option<(i32, i32, i32, i32)>,
    surface_keyboard: Vec<u32>,
    popup_keyboard: Vec<u32>,
    dismissed: Vec<u32>,
    closed: usize,
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

impl Dispatch<client::inox_shell_v1::InoxShellV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &client::inox_shell_v1::InoxShellV1,
        event: client::inox_shell_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let client::inox_shell_v1::Event::Capabilities {
            capabilities,
            max_transition_duration,
        } = event
        {
            state.capabilities = Some((capabilities.into(), max_transition_duration));
        }
    }
}

impl Dispatch<client::inox_shell_surface_v1::InoxShellSurfaceV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &client::inox_shell_surface_v1::InoxShellSurfaceV1,
        event: client::inox_shell_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use client::inox_shell_surface_v1::Event;
        match event {
            Event::Configure {
                serial,
                width,
                height,
            } => state.configures.push((serial, width, height)),
            Event::Workarea {
                x,
                y,
                width,
                height,
            } => state.workarea = Some((x, y, width, height)),
            Event::KeyboardState { focused } => state.surface_keyboard.push(focused),
            Event::Closed => state.closed += 1,
            _ => {}
        }
    }
}

impl Dispatch<client::inox_shell_popup_v1::InoxShellPopupV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &client::inox_shell_popup_v1::InoxShellPopupV1,
        event: client::inox_shell_popup_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use client::inox_shell_popup_v1::Event;
        match event {
            Event::Configure {
                serial,
                width,
                height,
            } => state.configures.push((serial, width, height)),
            Event::Dismissed { reason } => state.dismissed.push(reason.into()),
            Event::KeyboardState { focused } => state.popup_keyboard.push(focused),
            _ => {}
        }
    }
}

/// The server half: it records every request it is handed and answers the
/// events a compositor owes a client, so the assertions are about what crossed
/// the socket.
#[derive(Default)]
struct ServerState {
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl ServerState {
    fn record(&self, observed: Sent) {
        if let Ok(mut sent) = self.sent.lock() {
            sent.push(observed);
        }
    }
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

impl GlobalDispatch<server::inox_shell_v1::InoxShellV1, ()> for ServerState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<server::inox_shell_v1::InoxShellV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let shell = data_init.init(resource, ());
        // The capability event is what a shell decides its fallback from, so it
        // must be the first thing the object does — a client that asked for a
        // slide before being told the compositor only fades would be entitled to
        // complain.
        shell.capabilities(
            server::inox_shell_v1::Capability::Transitions
                | server::inox_shell_v1::Capability::PopupKeyboard
                | server::inox_shell_v1::Capability::Workarea
                | server::inox_shell_v1::Capability::NestedPopups,
            MAX_TRANSITION,
        );
    }
}

impl ServerDispatch<server::inox_shell_v1::InoxShellV1, ()> for ServerState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &server::inox_shell_v1::InoxShellV1,
        request: server::inox_shell_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let server::inox_shell_v1::Request::GetSurface { id, surface, role } = request {
            let role: u32 = role.into();
            state.record(Sent::Role(role));

            let shell_surface = data_init.init(id, ());

            // A compositor may refuse a role outright, and `closed` is how it
            // says so — there is no configure to wait for. This fake compositor
            // refuses the `menu` role because a shell menu is created as a popup
            // in this protocol, and a client that ignores that must find out
            // rather than draw into a surface nobody shows.
            if role == 3 {
                shell_surface.closed();
                let _ = surface;
                return;
            }

            // A shell surface is sized by the compositor, so the answer to
            // "assign this role" is a configure and the work area it may be laid
            // out against. Neither is optional: a client that gets a role and no
            // size has nothing to draw into.
            shell_surface.configure(1, 1920, 32);
            shell_surface.workarea(WORKAREA.0, WORKAREA.1, WORKAREA.2, WORKAREA.3);
            shell_surface.keyboard_state(0);
            let _ = surface;
        }
    }
}

impl ServerDispatch<server::inox_shell_surface_v1::InoxShellSurfaceV1, ()> for ServerState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &server::inox_shell_surface_v1::InoxShellSurfaceV1,
        request: server::inox_shell_surface_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use server::inox_shell_surface_v1::Request;
        match request {
            Request::SetPlacement {
                layer,
                anchor,
                exclusive_zone,
                margin_top,
                margin_right,
                margin_bottom,
                margin_left,
                width,
                height,
            } => state.record(Sent::Placement {
                layer: layer.into(),
                anchor: anchor.into(),
                exclusive_zone,
                margins: [margin_top, margin_right, margin_bottom, margin_left],
                size: [width, height],
            }),
            Request::SetKeyboard { mode } => state.record(Sent::Keyboard(mode.into())),
            Request::SetTransition {
                enter,
                exit,
                duration,
                curve,
                origin,
            } => state.record(Sent::Transition {
                effects: [enter.into(), exit.into()],
                duration,
                curve: curve.into(),
                origin: origin.into(),
            }),
            Request::GetPopup { id, surface } => {
                state.record(Sent::Popup { nested: false });
                let popup = data_init.init(id, ());
                popup.configure(2, 240, 180);
                let _ = surface;
            }
            Request::Destroy => {}
            _ => {}
        }
    }
}

impl ServerDispatch<server::inox_shell_popup_v1::InoxShellPopupV1, ()> for ServerState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &server::inox_shell_popup_v1::InoxShellPopupV1,
        request: server::inox_shell_popup_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use server::inox_shell_popup_v1::Request;
        match request {
            Request::SetPosition {
                parent_anchor,
                popup_anchor,
                x,
                y,
            } => state.record(Sent::Position {
                parent_anchor: parent_anchor.into(),
                popup_anchor: popup_anchor.into(),
                offset: [x, y],
            }),
            Request::SetSize { width, height } => state.record(Sent::Size([width, height])),
            Request::SetKeyboard { needed } => state.record(Sent::Keyboard(needed)),
            Request::SetDismissable { dismissable } => {
                state.record(Sent::Dismissable(dismissable));
                // A dismissed popup is told why: the shell animates a click-
                // outside differently from an Escape.
                resource.dismissed(server::inox_shell_popup_v1::DismissReason::Escape);
                resource.keyboard_state(1);
            }
            Request::Dismiss => state.record(Sent::Dismiss),
            Request::GetPopup { id, surface } => {
                state.record(Sent::Popup { nested: true });
                let popup = data_init.init(id, ());
                popup.configure(3, 160, 120);
                let _ = surface;
            }
            Request::Destroy => {}
            _ => {}
        }
    }
}

struct ServerClient;

impl ClientData for ServerClient {}

/// Spawn the server thread and hand the connected client to `check`.
fn round_trip(
    check: impl FnOnce(
        &client::inox_shell_v1::InoxShellV1,
        &wl_compositor::WlCompositor,
        &mut App,
        &mut wayland_client::EventQueue<App>,
        &Arc<Mutex<Vec<Sent>>>,
    ),
) {
    let (client_socket, server_socket) = UnixStream::pair().expect("socket pair");

    let mut display = Display::<ServerState>::new().expect("server display");
    let mut handle = display.handle();
    handle.create_global::<ServerState, swl_compositor::WlCompositor, ()>(4, ());
    handle.create_global::<ServerState, server::inox_shell_v1::InoxShellV1, ()>(1, ());
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
    let shell: client::inox_shell_v1::InoxShellV1 =
        globals.bind(&qh, 1..=1, ()).expect("bind inox_shell_v1");

    let mut app = App::default();
    queue.roundtrip(&mut app).expect("roundtrip");

    check(&shell, &compositor, &mut app, &mut queue, &sent);

    stop.store(true, Ordering::Relaxed);
    drop(shell);
    drop(compositor);
    drop(connection);
    thread::sleep(Duration::from_millis(10));
    let _ = server.join();
}

#[test]
fn a_panel_is_placed_configured_and_told_its_work_area() {
    round_trip(|shell, compositor, app, queue, sent| {
        assert_eq!(
            app.capabilities,
            Some((CAPABILITIES, MAX_TRANSITION)),
            "the shell must learn what this compositor implements before it asks for anything"
        );

        let surface = compositor.create_surface(&queue.handle(), ());
        let shell_surface = shell.get_surface(
            &surface,
            client::inox_shell_v1::SurfaceRole::Panel,
            &queue.handle(),
            (),
        );
        queue.roundtrip(app).expect("roundtrip");

        assert_eq!(
            app.configures
                .first()
                .map(|(_, width, height)| (*width, *height)),
            Some((1920, 32)),
            "a shell surface is sized by the compositor, not by its client"
        );
        assert_eq!(
            app.workarea,
            Some(WORKAREA),
            "the work area is an event, not something the shell re-derives from config"
        );
        assert_eq!(
            app.surface_keyboard,
            vec![0],
            "a panel that asked for no keyboard is told it has none, rather than assuming it"
        );

        // A top bar: top edge only, reserving its own thickness, sized by the
        // compositor.
        shell_surface.set_placement(
            client::inox_shell_surface_v1::Layer::Top,
            client::inox_shell_surface_v1::Anchor::Top
                | client::inox_shell_surface_v1::Anchor::Left
                | client::inox_shell_surface_v1::Anchor::Right,
            32,
            0,
            0,
            0,
            0,
            0,
            32,
        );
        shell_surface.set_keyboard(client::inox_shell_surface_v1::KeyboardMode::None);
        shell_surface.set_transition(
            client::inox_shell_surface_v1::Effect::Fade,
            client::inox_shell_surface_v1::Effect::Slide,
            180,
            client::inox_shell_surface_v1::Curve::EaseOut,
            client::inox_shell_surface_v1::Anchor::Top,
        );
        surface.commit();
        queue.roundtrip(app).expect("roundtrip");

        let log = sent.lock().expect("server log");
        assert_eq!(
            log[0],
            Sent::Role(0),
            "the panel role travels as its number"
        );
        assert_eq!(
            log[1],
            Sent::Placement {
                layer: 2,
                anchor: 1 | 4 | 8,
                exclusive_zone: 32,
                margins: [0, 0, 0, 0],
                size: [0, 32],
            },
            "every field of one placement arrives together"
        );
        assert_eq!(log[2], Sent::Keyboard(0));
        assert_eq!(
            log[3],
            Sent::Transition {
                effects: [1, 2],
                duration: 180,
                curve: 1,
                origin: 1,
            },
            "a transition names effects, a duration, a curve and an origin"
        );
    });
}

#[test]
fn a_popup_is_positioned_dismissable_and_can_hold_a_submenu() {
    round_trip(|shell, compositor, app, queue, sent| {
        let surface = compositor.create_surface(&queue.handle(), ());
        let shell_surface = shell.get_surface(
            &surface,
            client::inox_shell_v1::SurfaceRole::Panel,
            &queue.handle(),
            (),
        );
        queue.roundtrip(app).expect("roundtrip");

        let menu_surface = compositor.create_surface(&queue.handle(), ());
        let menu = shell_surface.get_popup(&menu_surface, &queue.handle(), ());
        menu.set_position(
            client::inox_shell_popup_v1::Anchor::Bottom
                | client::inox_shell_popup_v1::Anchor::Right,
            client::inox_shell_popup_v1::Anchor::Top | client::inox_shell_popup_v1::Anchor::Left,
            4,
            2,
        );
        menu.set_size(240, 0);
        menu.set_keyboard(1);
        menu.set_dismissable(1);
        menu.set_transition(
            client::inox_shell_popup_v1::Effect::Scale,
            client::inox_shell_popup_v1::Effect::Fade,
            120,
            client::inox_shell_popup_v1::Curve::Spring,
        );
        menu_surface.commit();
        queue.roundtrip(app).expect("roundtrip");

        // The compositor clamps a menu to 180 high and tells the client so.
        assert!(
            app.configures.contains(&(2, 240, 180)),
            "the popup's size comes back as a configure: {:?}",
            app.configures
        );

        // A submenu is the same object again, nested in the popup.
        let submenu_surface = compositor.create_surface(&queue.handle(), ());
        let submenu = menu.get_popup(&submenu_surface, &queue.handle(), ());
        submenu.set_position(
            client::inox_shell_popup_v1::Anchor::Right,
            client::inox_shell_popup_v1::Anchor::Top | client::inox_shell_popup_v1::Anchor::Left,
            -4,
            0,
        );
        submenu.set_size(160, 120);
        submenu_surface.commit();
        queue.roundtrip(app).expect("roundtrip");

        // Closing from the client side is a request: the compositor runs the
        // exit transition and destroys the object, so `dismiss` is not a
        // destructor and carries no serial.
        menu.dismiss();
        queue.roundtrip(app).expect("roundtrip");

        let log = sent.lock().expect("server log");
        let popups: Vec<&Sent> = log
            .iter()
            .filter(|sent| matches!(sent, Sent::Popup { .. }))
            .collect();
        assert_eq!(
            popups,
            [
                &Sent::Popup { nested: false },
                &Sent::Popup { nested: true }
            ],
            "a submenu is a popup whose parent is a popup"
        );
        assert!(
            log.contains(&Sent::Position {
                parent_anchor: 2 | 8,
                popup_anchor: 1 | 4,
                offset: [4, 2],
            }),
            "both anchors and the offset travel: {:?}",
            log
        );
        assert!(log.contains(&Sent::Size([240, 0])));
        assert!(log.contains(&Sent::Keyboard(1)));
        assert!(
            log.contains(&Sent::Dismissable(1)),
            "a menu declares that the compositor may dismiss it"
        );
        assert!(
            log.contains(&Sent::Dismiss),
            "closing a menu is a request the compositor can animate"
        );
    });
}

#[test]
fn a_compositor_can_dismiss_a_popup_and_refuse_a_role() {
    round_trip(|shell, compositor, app, queue, sent| {
        let surface = compositor.create_surface(&queue.handle(), ());
        let shell_surface = shell.get_surface(
            &surface,
            client::inox_shell_v1::SurfaceRole::Dock,
            &queue.handle(),
            (),
        );
        queue.roundtrip(app).expect("roundtrip");

        let menu_surface = compositor.create_surface(&queue.handle(), ());
        let menu = shell_surface.get_popup(&menu_surface, &queue.handle(), ());
        menu.set_dismissable(1);
        menu_surface.commit();
        queue.roundtrip(app).expect("roundtrip");

        assert_eq!(
            sent.lock().expect("server log").last(),
            Some(&Sent::Dismissable(1)),
            "the popup asked to be dismissable before being dismissed"
        );
        assert_eq!(
            app.dismissed,
            vec![1],
            "a dismissal names its reason, so the shell can pick its exit animation"
        );
        assert_eq!(
            app.popup_keyboard,
            vec![1],
            "the popup is told it has the keyboard, instead of assuming it"
        );

        // A role the compositor will not grant is closed, not configured: a
        // shell that is not shown must not keep rendering into nothing.
        let refused_surface = compositor.create_surface(&queue.handle(), ());
        let _refused = shell.get_surface(
            &refused_surface,
            client::inox_shell_v1::SurfaceRole::Menu,
            &queue.handle(),
            (),
        );
        queue.roundtrip(app).expect("roundtrip");

        assert_eq!(
            app.closed, 1,
            "a refused role arrives as `closed` before any configure"
        );
        assert_eq!(
            sent.lock().expect("server log").last(),
            Some(&Sent::Role(3)),
            "the refusal is for the role that was asked for"
        );
    });
}
