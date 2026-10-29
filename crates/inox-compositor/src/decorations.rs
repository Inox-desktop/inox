//! `zxdg_decoration_manager_v1`: who draws a toplevel's frame.
//!
//! This compositor draws **no** server-side decorations. A toplevel owns its
//! whole rectangle, and the only window frame on screen that is not the client's
//! own is an X11 one, drawn by `xwayland-satellite`. So every decoration object
//! this module creates is answered with `client_side`.
//!
//! Advertising a protocol whose only answer is "you draw it" is still worth
//! doing, because the answer is *information the client cannot otherwise get*.
//! Without the global a toolkit has to guess, and the guesses differ: GTK 3 and
//! GTK 4 draw their own header bars, while SDL, GLFW and several Qt paths fall
//! back to an undecorated surface with no title bar at all — a window the user
//! cannot move and cannot close from itself. `client_side` is the one answer
//! that every client already understands and that matches what this compositor
//! actually draws.
//!
//! Version 1 only. Version 2 relaxes *when* a decoration object may be created
//! (after the toplevel has committed a buffer) and adds the `orphaned` error;
//! advertising 2 while not implementing either would be a claim this code does
//! not keep, so the global stops at 1 and a client that binds 1..=2 gets 1.

use inox_protocols::standard::xdg::decoration::server::{
    zxdg_decoration_manager_v1::{self, ZxdgDecorationManagerV1},
    zxdg_toplevel_decoration_v1::{self, ZxdgToplevelDecorationV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, WEnum, backend::GlobalId,
};

/// The version `zxdg_decoration_manager_v1` and its children are advertised at.
///
/// See the module docs for why this is 1 and not 2.
const VERSION: u32 = 1;

/// Owns the `zxdg_decoration_manager_v1` global.
///
/// Nothing in the compositor reads this value: Smithay's dispatch is generic over
/// the compositor state, so keeping the value alive is the whole point — dropping
/// it removes the global.
#[derive(Debug)]
pub(crate) struct DecorationState {
    /// The global this state owns. Never read, kept so it is not dropped.
    #[allow(dead_code)]
    global: GlobalId,
}

impl DecorationState {
    /// Advertise `zxdg_decoration_manager_v1`.
    pub(crate) fn new<D>(display_handle: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<ZxdgDecorationManagerV1, ()>,
        D: Dispatch<ZxdgDecorationManagerV1, ()>,
        D: Dispatch<ZxdgToplevelDecorationV1, ()>,
        D: 'static,
    {
        let global = display_handle.create_global::<D, ZxdgDecorationManagerV1, _>(VERSION, ());

        Self { global }
    }
}

impl<D> GlobalDispatch<ZxdgDecorationManagerV1, (), D> for DecorationState
where
    D: GlobalDispatch<ZxdgDecorationManagerV1, ()>,
    D: Dispatch<ZxdgDecorationManagerV1, ()>,
    D: Dispatch<ZxdgToplevelDecorationV1, ()>,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _display_handle: &DisplayHandle,
        _client: &Client,
        manager: New<ZxdgDecorationManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ());
    }
}

impl<D> Dispatch<ZxdgDecorationManagerV1, (), D> for DecorationState
where
    D: GlobalDispatch<ZxdgDecorationManagerV1, ()>,
    D: Dispatch<ZxdgDecorationManagerV1, ()>,
    D: Dispatch<ZxdgToplevelDecorationV1, ()>,
    D: 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _manager: &ZxdgDecorationManagerV1,
        request: zxdg_decoration_manager_v1::Request,
        _data: &(),
        _display_handle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            // The answer is sent at creation instead of waiting for the
            // toplevel's first commit: both events are ordered on the same
            // connection, and a client that gets its decoration mode before its
            // initial `xdg_surface.configure` simply has it earlier than it
            // needs it. Waiting would mean tracking each toplevel's commit state
            // to say something that never changes.
            zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id, .. } => {
                let decoration = data_init.init(id, ());
                decoration.configure(zxdg_toplevel_decoration_v1::Mode::ClientSide);
            }
            // The manager is only a factory: the objects it created outlive it.
            zxdg_decoration_manager_v1::Request::Destroy => {}
            // `Request` is `#[non_exhaustive]`.
            _ => {}
        }
    }
}

impl<D> Dispatch<ZxdgToplevelDecorationV1, (), D> for DecorationState
where
    D: GlobalDispatch<ZxdgDecorationManagerV1, ()>,
    D: Dispatch<ZxdgDecorationManagerV1, ()>,
    D: Dispatch<ZxdgToplevelDecorationV1, ()>,
    D: 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        decoration: &ZxdgToplevelDecorationV1,
        request: zxdg_toplevel_decoration_v1::Request,
        _data: &(),
        _display_handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            // Both requests are a *preference*: the protocol lets the compositor
            // enforce its own mode, and this compositor's mode is always the
            // client's. Answering every request with the same `configure` is
            // what makes "`server_side` was requested and refused" observable,
            // rather than a silence the client has to time out on.
            zxdg_toplevel_decoration_v1::Request::SetMode { mode } => {
                if !matches!(mode, WEnum::Value(_)) {
                    // An undefined enum value is a protocol error the client
                    // caused; ignoring it is safer than dropping the connection
                    // for a request whose only effect is a hint.
                    return;
                }
                decoration.configure(zxdg_toplevel_decoration_v1::Mode::ClientSide);
            }
            // `unset_mode` means "no preference"; it still owes a configure, and
            // the mode it falls back to is this compositor's, which is CSD.
            zxdg_toplevel_decoration_v1::Request::UnsetMode => {
                decoration.configure(zxdg_toplevel_decoration_v1::Mode::ClientSide);
            }
            // `destroy` is a destructor: wayland-server removes the object once
            // the request has been dispatched, and nothing here tracks it.
            zxdg_toplevel_decoration_v1::Request::Destroy => {}
            // `Request` is `#[non_exhaustive]`.
            _ => {}
        }
    }
}
