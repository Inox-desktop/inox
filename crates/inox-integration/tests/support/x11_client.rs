//! Real X11 clients for the M3 end-to-end tests, built on `x11rb`.
//!
//! These are genuine X11 clients speaking the wire protocol to the Xwayland
//! instance `inox-xwayland` supervises. Nothing here shells out to `xclip`,
//! `xdotool` or `xwininfo`: an assertion that a window was configured to a
//! particular size, that a key arrived as a particular key code, or that a
//! clipboard transfer produced particular bytes has to come from the client that
//! experienced it.
//!
//! Two X11 behaviours the M3 evidence depends on live here:
//!
//! * this client **answers** `SelectionRequest` itself, including the `TARGETS`
//!   query and the `INCR` protocol it uses above [`INCR_THRESHOLD`] bytes, so an
//!   X11-to-Wayland clipboard transfer goes through the real X11 selection
//!   machinery rather than through a shortcut;
//! * it also **requests** through `convert_selection` and handles an `INCR`
//!   reply, with every wait bounded.
//!
//! Every window created for a transfer is created but never mapped, so a
//! clipboard request cannot perturb the tiling the X01 assertions check.

#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use x11rb::{
    COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME,
    connection::Connection as _,
    protocol::Event as XEvent,
    protocol::xproto::{
        Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask,
        PropMode, Property, SelectionNotifyEvent, SelectionRequestEvent, WindowClass,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use super::POLL_INTERVAL;

/// Re-exported so the tests can name window handles without depending on
/// `x11rb` themselves.
pub use x11rb::protocol::xproto::Window;

/// Payload size above which this client uses the X11 `INCR` protocol.
///
/// Once a transfer no longer fits comfortably in a single property write, the
/// ICCCM requires `INCR`; 64 KiB is the long-standing practical boundary. The
/// M3 test that reads a payload above it therefore exercises the requestor's
/// `INCR` handling inside `xwayland-satellite`.
pub const INCR_THRESHOLD: usize = 64 * 1024;

/// Chunk size used when writing an `INCR` transfer.
pub const INCR_CHUNK: usize = 64 * 1024;

/// The atoms this client needs by name.
#[derive(Debug, Clone, Copy)]
pub struct Atoms {
    /// `CLIPBOARD`.
    pub clipboard: Atom,
    /// `PRIMARY`.
    pub primary: Atom,
    /// `TARGETS`.
    pub targets: Atom,
    /// `UTF8_STRING`.
    pub utf8_string: Atom,
    /// `text/plain;charset=utf-8`.
    pub text_plain: Atom,
    /// `image/png`.
    pub image_png: Atom,
    /// `text/html`.
    pub text_html: Atom,
    /// `STRING`.
    pub string: Atom,
    /// `INCR`.
    pub incr: Atom,
    /// `ATOM`.
    pub atom: Atom,
    /// `WM_NAME`.
    pub wm_name: Atom,
}

/// One X11 event this client observed, in arrival order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The window was reconfigured; the size is its client area.
    Configure {
        /// Which window.
        window: Window,
        /// Width in pixels.
        width: u16,
        /// Height in pixels.
        height: u16,
    },
    /// The window was mapped.
    Map {
        /// Which window.
        window: Window,
    },
    /// The window was unmapped.
    Unmap {
        /// Which window.
        window: Window,
    },
    /// The window needs repainting.
    Expose {
        /// Which window.
        window: Window,
    },
    /// The window gained or lost the X input focus.
    Focus {
        /// Which window.
        window: Window,
        /// Whether the window is focused.
        focused: bool,
    },
    /// A key was pressed while the window had the focus.
    KeyPress {
        /// Which window the event was reported on.
        window: Window,
        /// The X11 key code (`evdev` code plus 8).
        keycode: u8,
        /// The modifier state reported with the event.
        state: u16,
    },
    /// A key was released.
    KeyRelease {
        /// Which window the event was reported on.
        window: Window,
        /// The X11 key code.
        keycode: u8,
    },
    /// The window was destroyed.
    Destroy {
        /// Which window.
        window: Window,
    },
    /// This client stopped owning the selection.
    SelectionClear {
        /// The selection atom.
        selection: Atom,
    },
    /// Somebody asked this client for its selection.
    SelectionRequest {
        /// The requested target atom.
        target: Atom,
        /// The window that asked.
        requestor: Window,
    },
    /// An answer to this client's own selection request arrived.
    SelectionNotify {
        /// The target that was answered.
        target: Atom,
        /// The property holding the data, or `0` when nothing was delivered.
        property: Atom,
    },
    /// A property gained a new value.
    PropertyNewValue {
        /// The window holding the property.
        window: Window,
        /// The property.
        property: Atom,
    },
    /// A property was deleted.
    PropertyDelete {
        /// The window holding the property.
        window: Window,
        /// The property.
        property: Atom,
    },
}

/// A transfer this client is feeding, chunk by chunk.
struct IncrDelivery {
    requestor: Window,
    property: Atom,
    target: Atom,
    data: Vec<u8>,
    offset: usize,
}

/// The selection this client currently owns.
struct OwnedSelection {
    entries: Vec<(Atom, Vec<u8>)>,
    deliveries: Vec<IncrDelivery>,
}

/// A real X11 client.
pub struct X11Client {
    conn: RustConnection,
    screen: usize,
    root: Window,
    /// The atoms interned at startup.
    pub atoms: Atoms,
    /// Observations, in arrival order.
    pub events: Vec<Event>,
    /// A label used in failure messages.
    pub name: String,
    extra_atoms: BTreeMap<String, Atom>,
    selection: Option<OwnedSelection>,
    notify: Option<(Atom, Atom)>,
    new_values: Vec<(Window, Atom)>,
    counter: u32,
}

impl X11Client {
    /// Connect to `display` (for example `:97`) and intern the atoms.
    pub fn connect(display: &str, name: &str) -> Self {
        let (conn, screen) = RustConnection::connect(Some(display)).unwrap_or_else(|error| {
            panic!("{name}: cannot connect to X display {display}: {error}")
        });
        let root = conn.setup().roots[screen].root;

        let mut client = Self {
            conn,
            screen,
            root,
            atoms: Atoms {
                clipboard: 0,
                primary: 0,
                targets: 0,
                utf8_string: 0,
                text_plain: 0,
                image_png: 0,
                text_html: 0,
                string: 0,
                incr: 0,
                atom: 0,
                wm_name: 0,
            },
            events: Vec::new(),
            name: name.to_owned(),
            extra_atoms: BTreeMap::new(),
            selection: None,
            notify: None,
            new_values: Vec::new(),
            counter: 0,
        };

        client.atoms = Atoms {
            clipboard: client.intern("CLIPBOARD"),
            primary: client.intern("PRIMARY"),
            targets: client.intern("TARGETS"),
            utf8_string: client.intern("UTF8_STRING"),
            text_plain: client.intern("text/plain;charset=utf-8"),
            image_png: client.intern("image/png"),
            text_html: client.intern("text/html"),
            string: client.intern("STRING"),
            incr: client.intern("INCR"),
            atom: client.intern("ATOM"),
            wm_name: client.intern("WM_NAME"),
        };
        client
    }

    /// Intern `name`, caching the result.
    pub fn intern(&mut self, name: &str) -> Atom {
        if let Some(atom) = self.extra_atoms.get(name) {
            return *atom;
        }
        let atom = self
            .conn
            .intern_atom(false, name.as_bytes())
            .unwrap_or_else(|error| panic!("{}: intern_atom({name}) failed: {error}", self.name))
            .reply()
            .unwrap_or_else(|error| panic!("{}: intern_atom({name}) failed: {error}", self.name))
            .atom;
        self.extra_atoms.insert(name.to_owned(), atom);
        atom
    }

    /// The X screen size.
    ///
    /// Xwayland derives this from the compositor's `wl_output` mode, so a
    /// compositor that never sends a mode with the `Current` flag leaves it at
    /// 0x0 — which is exactly what X01 asserts against.
    pub fn screen_size(&self) -> (u16, u16) {
        let root = &self.conn.setup().roots[self.screen];
        (root.width_in_pixels, root.height_in_pixels)
    }

    /// Create a top-level window with a solid background, without mapping it.
    pub fn create_window(&mut self, colour: u32, width: u16, height: u16, title: &str) -> Window {
        let window = self
            .conn
            .generate_id()
            .unwrap_or_else(|error| panic!("{}: generate_id failed: {error}", self.name));
        let aux = CreateWindowAux::new().background_pixel(colour).event_mask(
            EventMask::STRUCTURE_NOTIFY
                | EventMask::EXPOSURE
                | EventMask::KEY_PRESS
                | EventMask::PROPERTY_CHANGE
                | EventMask::FOCUS_CHANGE,
        );

        self.conn
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                window,
                self.root,
                0,
                0,
                width,
                height,
                0,
                WindowClass::INPUT_OUTPUT,
                COPY_FROM_PARENT,
                &aux,
            )
            .unwrap_or_else(|error| panic!("{}: create_window failed: {error}", self.name))
            .check()
            .unwrap_or_else(|error| panic!("{}: create_window failed: {error}", self.name));

        self.conn
            .change_property8(
                PropMode::REPLACE,
                window,
                self.atoms.wm_name,
                AtomEnum::STRING,
                title.as_bytes(),
            )
            .unwrap_or_else(|error| panic!("{}: setting WM_NAME failed: {error}", self.name));
        self.flush();
        window
    }

    /// Map a window: this is what makes the satellite give it a Wayland toplevel
    /// and the compositor tile it.
    pub fn map_window(&mut self, window: Window) {
        self.conn
            .map_window(window)
            .unwrap_or_else(|error| panic!("{}: map_window failed: {error}", self.name))
            .check()
            .unwrap_or_else(|error| panic!("{}: map_window failed: {error}", self.name));
        self.flush();
    }

    /// Destroy a window, which the satellite turns into the Wayland toplevel
    /// going away.
    pub fn destroy_window(&mut self, window: Window) {
        self.conn
            .destroy_window(window)
            .unwrap_or_else(|error| panic!("{}: destroy_window failed: {error}", self.name))
            .check()
            .unwrap_or_else(|error| panic!("{}: destroy_window failed: {error}", self.name));
        self.flush();
    }

    /// Flush pending requests.
    pub fn flush(&mut self) {
        self.conn
            .flush()
            .unwrap_or_else(|error| panic!("{}: flush failed: {error}", self.name));
    }

    /// The size the window was last configured to, from `ConfigureNotify`.
    pub fn configured_size(&self, window: Window) -> Option<(u16, u16)> {
        self.events.iter().rev().find_map(|event| match event {
            Event::Configure {
                window: w,
                width,
                height,
            } if *w == window => Some((*width, *height)),
            _ => None,
        })
    }

    /// Every `KeyPress` key code that reached `window`, in arrival order.
    pub fn key_presses(&self, window: Window) -> Vec<u8> {
        self.events
            .iter()
            .filter_map(|event| match event {
                Event::KeyPress {
                    window: w, keycode, ..
                } if *w == window => Some(*keycode),
                _ => None,
            })
            .collect()
    }

    /// How many `MapNotify` events arrived for `window`.
    pub fn map_count(&self, window: Window) -> usize {
        self.events
            .iter()
            .filter(|event| matches!(event, Event::Map { window: w } if *w == window))
            .count()
    }

    /// Become the owner of `CLIPBOARD`, offering `entries` (target name ->
    /// bytes) plus a `TARGETS` reply listing exactly those targets.
    ///
    /// Requests are answered from [`Self::poll`], so the caller has to keep
    /// dispatching while a transfer is in flight.
    pub fn own_clipboard(&mut self, owner: Window, entries: &[(&str, Vec<u8>)]) {
        let entries: Vec<(Atom, Vec<u8>)> = entries
            .iter()
            .map(|(name, bytes)| (self.intern(name), bytes.clone()))
            .collect();

        self.conn
            .set_selection_owner(owner, self.atoms.clipboard, CURRENT_TIME)
            .unwrap_or_else(|error| panic!("{}: set_selection_owner failed: {error}", self.name))
            .check()
            .unwrap_or_else(|error| panic!("{}: set_selection_owner failed: {error}", self.name));
        self.flush();

        self.selection = Some(OwnedSelection {
            entries,
            deliveries: Vec::new(),
        });
    }

    /// The window currently owning `CLIPBOARD`, or `0` when nobody does.
    pub fn selection_owner(&mut self) -> Window {
        self.conn
            .get_selection_owner(self.atoms.clipboard)
            .unwrap_or_else(|error| panic!("{}: get_selection_owner failed: {error}", self.name))
            .reply()
            .unwrap_or_else(|error| panic!("{}: get_selection_owner failed: {error}", self.name))
            .owner
    }

    /// Wait until somebody other than this client's own windows owns
    /// `CLIPBOARD`, and return that owner.
    pub fn wait_for_foreign_owner(&mut self, own: Window, timeout: Duration) -> Window {
        let deadline = Instant::now() + timeout;
        loop {
            let owner = self.selection_owner();
            if owner != 0 && owner != own {
                return owner;
            }
            assert!(
                Instant::now() < deadline,
                "{}: CLIPBOARD is still owned by {owner} after {timeout:?}",
                self.name
            );
            self.poll(POLL_INTERVAL);
        }
    }

    /// Whether this client still owns `CLIPBOARD`.
    pub fn owns_clipboard(&self) -> bool {
        self.selection.is_some()
            && !self.events.iter().any(
                |event| matches!(event, Event::SelectionClear { selection } if *selection == self.atoms.clipboard),
            )
    }

    /// The targets this client offers, in the order they were announced.
    pub fn offered_targets(&self) -> Vec<Atom> {
        self.selection
            .as_ref()
            .map(|selection| selection.entries.iter().map(|(atom, _)| *atom).collect())
            .unwrap_or_default()
    }

    /// Ask the current `CLIPBOARD` owner for `target`.
    ///
    /// Returns `None` when the owner refuses (an unsupported target is answered
    /// with `property = None`), or the exact bytes otherwise. `INCR` is handled
    /// transparently. The whole exchange is bounded by `timeout`.
    pub fn request_clipboard(&mut self, target: &str, timeout: Duration) -> Option<Vec<u8>> {
        self.request_clipboard_while(target, timeout, || {})
    }

    /// [`Self::request_clipboard`], with `pump` called on every wait.
    ///
    /// A clipboard transfer spans three parties: this X11 client, the satellite,
    /// and whichever client owns the data. When a *Wayland* client owns it, the
    /// satellite answers the X11 request by asking that client for the bytes, so
    /// the Wayland connection has to keep being dispatched while this side waits
    /// for its `SelectionNotify` — otherwise the satellite blocks on a pipe
    /// nobody is draining and the request simply never completes.
    pub fn request_clipboard_while(
        &mut self,
        target: &str,
        timeout: Duration,
        mut pump: impl FnMut(),
    ) -> Option<Vec<u8>> {
        let target = self.intern(target);
        let requestor = self.create_transfer_window();
        self.counter += 1;
        let property = self.intern(&format!(
            "INOX_M3_SELECTION_{}_{}",
            self.counter,
            std::process::id()
        ));

        self.conn
            .delete_property(requestor, property)
            .unwrap_or_else(|error| panic!("{}: delete_property failed: {error}", self.name))
            .check()
            .ok();
        self.conn
            .convert_selection(
                requestor,
                self.atoms.clipboard,
                target,
                property,
                CURRENT_TIME,
            )
            .unwrap_or_else(|error| panic!("{}: convert_selection failed: {error}", self.name))
            .check()
            .unwrap_or_else(|error| panic!("{}: convert_selection failed: {error}", self.name));
        self.flush();

        // The notification must answer *this* request: a late answer to an
        // earlier one would otherwise be read as the data for the new target.
        self.notify = None;
        let deadline = Instant::now() + timeout;
        let answer_property = loop {
            if let Some((answered, property)) = self.notify {
                if answered == target {
                    self.notify = None;
                    break property;
                }
            }
            assert!(
                Instant::now() < deadline,
                "{}: no SelectionNotify for the requested target within {timeout:?}; events: {:#?}",
                self.name,
                self.events
            );
            pump();
            self.poll(POLL_INTERVAL);
        };

        if answer_property == AtomEnum::NONE.into() {
            // The owner refused the target: the protocol's "no such data".
            self.destroy_window(requestor);
            return None;
        }

        let (bytes, kind) = self.read_property(requestor, answer_property);
        if kind == self.atoms.incr {
            // `INCR`: the property holds the total size; the data itself
            // arrives in chunks, each announced by a `PropertyNotify`.
            let total = bytes
                .get(..4)
                .map(|head| u32::from_ne_bytes([head[0], head[1], head[2], head[3]]) as usize)
                .unwrap_or(0);
            let data = self.read_incr(requestor, answer_property, total, deadline, &mut pump);
            self.destroy_window(requestor);
            return Some(data);
        }

        self.destroy_window(requestor);
        Some(bytes)
    }

    /// Read a property in full, returning its bytes and its type atom.
    fn read_property(&mut self, window: Window, property: Atom) -> (Vec<u8>, Atom) {
        let mut offset = 0u32;
        let mut bytes = Vec::new();
        let mut kind = 0;
        loop {
            let reply = self
                .conn
                .get_property(
                    offset == 0,
                    window,
                    property,
                    AtomEnum::ANY,
                    offset,
                    u32::MAX / 4,
                )
                .unwrap_or_else(|error| panic!("{}: get_property failed: {error}", self.name))
                .reply()
                .unwrap_or_else(|error| panic!("{}: get_property failed: {error}", self.name));
            if kind == 0 {
                kind = reply.type_;
            }
            bytes.extend_from_slice(&reply.value);
            if reply.bytes_after == 0 {
                return (bytes, kind);
            }
            // `long_offset` counts 4-byte units.
            offset += reply.value.len() as u32 / 4;
        }
    }

    /// Collect an `INCR` transfer until the owner signals completion with an
    /// empty property.
    fn read_incr(
        &mut self,
        requestor: Window,
        property: Atom,
        expected: usize,
        deadline: Instant,
        pump: &mut impl FnMut(),
    ) -> Vec<u8> {
        let mut data = Vec::with_capacity(expected);
        loop {
            while !self.take_property_new_value(requestor, property) {
                assert!(
                    Instant::now() < deadline,
                    "{}: INCR transfer stalled after {} of an announced {expected} bytes",
                    self.name,
                    data.len()
                );
                pump();
                self.poll(POLL_INTERVAL);
            }

            let (chunk, _) = self.read_property(requestor, property);
            if chunk.is_empty() {
                return data;
            }
            data.extend_from_slice(&chunk);
        }
    }

    /// Consume a pending `PropertyNotify`/`NewValue` observation.
    fn take_property_new_value(&mut self, window: Window, property: Atom) -> bool {
        match self
            .new_values
            .iter()
            .position(|(w, p)| *w == window && *p == property)
        {
            Some(index) => {
                self.new_values.remove(index);
                true
            }
            None => false,
        }
    }

    /// Create the never-mapped window used as a selection requestor.
    fn create_transfer_window(&mut self) -> Window {
        self.create_window(0, 1, 1, "inox-m3-selection-requestor")
    }

    /// Dispatch everything already pending on the connection.
    pub fn dispatch(&mut self) {
        loop {
            match self.conn.poll_for_event() {
                Ok(Some(event)) => self.handle(event),
                Ok(None) => return,
                Err(error) => panic!("{}: reading X11 events failed: {error}", self.name),
            }
        }
    }

    /// Wait up to `timeout` for something to read, then dispatch it.
    pub fn poll(&mut self, timeout: Duration) {
        self.dispatch();
        let stream = self.conn.stream();
        let mut fds = [PollFd::new(stream, PollFlags::IN)];
        let timespec = Timespec {
            tv_sec: timeout.as_secs() as i64,
            tv_nsec: i64::from(timeout.subsec_nanos()),
        };
        let _ = poll(&mut fds, Some(&timespec));
        self.dispatch();
    }

    /// Poll until `predicate` holds, failing the test on timeout.
    pub fn wait_until(
        &mut self,
        deadline: Instant,
        what: &str,
        predicate: impl Fn(&X11Client) -> bool,
    ) {
        loop {
            if predicate(self) {
                return;
            }
            if Instant::now() >= deadline {
                panic!(
                    "{}: timed out waiting for {what}; observed events: {:#?}",
                    self.name, self.events
                );
            }
            self.poll(POLL_INTERVAL);
        }
    }

    /// Wait until a key press with `keycode` reaches `window`.
    pub fn wait_for_key_press(&mut self, window: Window, keycode: u8, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        self.wait_until(
            deadline,
            &format!("a KeyPress({keycode}) on window {window}"),
            |client| client.key_presses(window).contains(&keycode),
        );
    }

    /// Wait until the window is configured to exactly `size`.
    pub fn wait_for_configure(&mut self, window: Window, size: (u16, u16), timeout: Duration) {
        let deadline = Instant::now() + timeout;
        self.wait_until(
            deadline,
            &format!("a ConfigureNotify of {size:?} for window {window}"),
            |client| client.configured_size(window) == Some(size),
        );
    }

    /// Handle one event, including the selection requests this client answers.
    fn handle(&mut self, event: XEvent) {
        match event {
            XEvent::ConfigureNotify(event) => self.events.push(Event::Configure {
                window: event.window,
                width: event.width,
                height: event.height,
            }),
            XEvent::MapNotify(event) => self.events.push(Event::Map {
                window: event.window,
            }),
            XEvent::UnmapNotify(event) => self.events.push(Event::Unmap {
                window: event.window,
            }),
            XEvent::Expose(event) => self.events.push(Event::Expose {
                window: event.window,
            }),
            XEvent::KeyPress(event) => self.events.push(Event::KeyPress {
                window: event.event,
                keycode: event.detail,
                state: event.state.bits(),
            }),
            XEvent::KeyRelease(event) => self.events.push(Event::KeyRelease {
                window: event.event,
                keycode: event.detail,
            }),
            XEvent::FocusIn(event) => self.events.push(Event::Focus {
                window: event.event,
                focused: true,
            }),
            XEvent::FocusOut(event) => self.events.push(Event::Focus {
                window: event.event,
                focused: false,
            }),
            XEvent::DestroyNotify(event) => self.events.push(Event::Destroy {
                window: event.window,
            }),
            XEvent::SelectionClear(event) => {
                self.selection = None;
                self.events.push(Event::SelectionClear {
                    selection: event.selection,
                });
            }
            XEvent::SelectionRequest(event) => self.answer_selection_request(event),
            XEvent::SelectionNotify(event) => {
                self.notify = Some((event.target, event.property));
                self.events.push(Event::SelectionNotify {
                    target: event.target,
                    property: event.property,
                });
            }
            XEvent::PropertyNotify(event) => {
                if event.state == Property::NEW_VALUE {
                    self.new_values.push((event.window, event.atom));
                    self.events.push(Event::PropertyNewValue {
                        window: event.window,
                        property: event.atom,
                    });
                } else if event.state == Property::DELETE {
                    self.events.push(Event::PropertyDelete {
                        window: event.window,
                        property: event.atom,
                    });
                    self.advance_incr(event.window, event.atom);
                }
            }
            _ => {}
        }
    }

    /// Answer one selection request, including `TARGETS` and `INCR`.
    fn answer_selection_request(&mut self, event: SelectionRequestEvent) {
        // An older convention: a requestor may leave `property` as `None`, in
        // which case the target doubles as the property.
        let property = if event.property == AtomEnum::NONE.into() {
            event.target
        } else {
            event.property
        };

        self.events.push(Event::SelectionRequest {
            target: event.target,
            requestor: event.requestor,
        });

        let Some(selection) = self.selection.as_ref() else {
            // No longer the owner: answer "no data" rather than stay silent.
            self.send_selection_notify(&event, AtomEnum::NONE.into(), property);
            return;
        };

        let targets: Vec<Atom> = selection.entries.iter().map(|(atom, _)| *atom).collect();
        let data = selection
            .entries
            .iter()
            .find(|(atom, _)| *atom == event.target)
            .map(|(_, bytes)| bytes.clone());

        if event.target == self.atoms.targets {
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    event.requestor,
                    property,
                    self.atoms.atom,
                    &targets,
                )
                .unwrap_or_else(|error| panic!("{}: TARGETS reply failed: {error}", self.name))
                .check()
                .ok();
            self.send_selection_notify(&event, self.atoms.atom, property);
            self.flush();
            return;
        }

        let Some(data) = data else {
            // Unsupported target: `property = None` is the protocol's refusal.
            self.send_selection_notify(&event, AtomEnum::NONE.into(), property);
            self.flush();
            return;
        };

        if data.len() > INCR_THRESHOLD {
            // Watch the requestor: deleting the property is how it asks for the
            // next chunk.
            self.conn
                .change_window_attributes(
                    event.requestor,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                )
                .ok();
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    event.requestor,
                    property,
                    self.atoms.incr,
                    &[data.len() as u32],
                )
                .unwrap_or_else(|error| panic!("{}: INCR marker failed: {error}", self.name))
                .check()
                .ok();
            self.send_selection_notify(&event, self.atoms.incr, property);
            if let Some(selection) = self.selection.as_mut() {
                selection.deliveries.push(IncrDelivery {
                    requestor: event.requestor,
                    property,
                    target: event.target,
                    data,
                    offset: 0,
                });
            }
            self.flush();
            return;
        }

        self.conn
            .change_property8(
                PropMode::REPLACE,
                event.requestor,
                property,
                event.target,
                &data,
            )
            .unwrap_or_else(|error| panic!("{}: selection reply failed: {error}", self.name))
            .check()
            .ok();
        self.send_selection_notify(&event, event.target, property);
        self.flush();
    }

    /// Write the next `INCR` chunk after the requestor consumed the previous one.
    fn advance_incr(&mut self, requestor: Window, property: Atom) {
        let Some(selection) = self.selection.as_mut() else {
            return;
        };
        let Some(index) = selection
            .deliveries
            .iter()
            .position(|delivery| delivery.requestor == requestor && delivery.property == property)
        else {
            return;
        };

        let (target, chunk, exhausted) = {
            let delivery = &mut selection.deliveries[index];
            let end = (delivery.offset + INCR_CHUNK).min(delivery.data.len());
            let chunk = delivery.data[delivery.offset..end].to_vec();
            delivery.offset = end;
            (
                delivery.target,
                chunk,
                delivery.offset >= delivery.data.len(),
            )
        };

        // The requestor deletes the property to ask for more, so an empty chunk
        // is only ever produced once the data has been fully delivered — and
        // that zero-length write is exactly how the owner signals the end of the
        // transfer. Removing the delivery at the *last non-empty* chunk instead
        // would leave the requestor waiting for a terminator that never comes.
        self.conn
            .change_property8(PropMode::REPLACE, requestor, property, target, &chunk)
            .unwrap_or_else(|error| panic!("{}: INCR chunk failed: {error}", self.name))
            .check()
            .ok();
        if exhausted && chunk.is_empty() {
            if let Some(selection) = self.selection.as_mut() {
                selection.deliveries.remove(index);
            }
        }
        self.flush();
    }

    /// Send the `SelectionNotify` that closes a request.
    ///
    /// `kind` is the atom the requestor should see in the notification: the
    /// target atom for a completed write, `INCR` when the data follows in
    /// chunks, or `None` to refuse the request. A refusal is the only case in
    /// which the reported property is `None`; for `INCR` the requestor gets the
    /// property name and then reads the marker from it.
    fn send_selection_notify(
        &mut self,
        request: &SelectionRequestEvent,
        kind: Atom,
        property: Atom,
    ) {
        let notify = SelectionNotifyEvent {
            response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if kind == AtomEnum::NONE.into() {
                AtomEnum::NONE.into()
            } else {
                property
            },
        };
        self.conn
            .send_event(false, request.requestor, EventMask::NO_EVENT, notify)
            .unwrap_or_else(|error| panic!("{}: send_event failed: {error}", self.name));
    }
}
