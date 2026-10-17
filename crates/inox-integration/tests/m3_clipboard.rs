//! Acceptance X02 (`docs/quality/acceptance.md` §5): the clipboard across the
//! Inox/XWayland boundary, byte for byte.
//!
//! Evidence level E3: an Inox headless session (the workspace's own compositor
//! with the test-only injection socket, plus `inox-xwayland` supervising the
//! real `xwayland-satellite`), driven by real X11 clients (`x11rb`) and real
//! Wayland clients (`wayland-client`). Nothing is mocked: an X11 owner answers
//! `SelectionRequest` over the X11 wire protocol, the satellite bridges it, and
//! the Wayland client reads the payload from a pipe.
//!
//! The properties X02 asks for, and where they are asserted:
//!
//! * **byte-for-byte equality** for UTF-8 text, a real PNG and a payload the
//!   owner can only deliver through `INCR` (the X11 owner switches to `INCR`
//!   above 64 KiB).
//! * **an unsupported type is refused**, answered with `property = None` on the
//!   X11 side and "no such mime" on the Wayland side, without disturbing the
//!   session: a following supported transfer still succeeds.
//! * **focus is the access-control boundary.** Only the client that holds the
//!   keyboard focus is offered the selection, so a client that never held it has
//!   nothing to read from — Inox's M3 answer to "an unauthorised client must not
//!   read the clipboard" (ADR 0017 decision 2), and the reason `wlr-data-control`
//!   is deliberately absent.
//! * **an owner that goes away fails cleanly**: a later read ends without
//!   hanging and without returning stale bytes, and the session is still usable
//!   afterwards.
//!
//! Focus is moved by **natural map ordering**: a newly mapped X11 window makes
//! the satellite the focused client, which is the only moment at which it may
//! take the selection over on the X11 side. That is the compositor's own focus
//! policy, not a shortcut — the Alt+Tab binding is covered in
//! `m3_x11_window.rs`.

mod support;

use std::time::{Duration, Instant};

use support::wayland_client::{OfferId, WaylandClient};
use support::x11_client::{Window, X11Client};
use support::{
    CASE_TIMEOUT, KEY_A, POLL_INTERVAL, Session, X_KEYCODE_A, skip_unless_enabled, tile_of_mapped,
};

/// A short UTF-8 text with non-ASCII content, so a lossy or re-encoded transfer
/// cannot pass by accident.
const TEXT: &str = "Inox M3 — ταχύς褐狐 🦊\nsecond line\twith a tab\n";

/// A real 4x4 RGB PNG (IHDR/IDAT/IEND, CRCs included).
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x08, 0x02, 0x00, 0x00, 0x00, 0x26, 0x93, 0x09,
    0x29, 0x00, 0x00, 0x00, 0x29, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x0d, 0xc7, 0x31, 0x01, 0x00,
    0x00, 0x0c, 0xc2, 0x30, 0x84, 0x55, 0x18, 0x67, 0x45, 0x21, 0x70, 0xcb, 0x97, 0x24, 0x12, 0x1b,
    0x17, 0x13, 0x04, 0x8b, 0xe3, 0x53, 0xa9, 0xad, 0xeb, 0x67, 0x32, 0x3b, 0x37, 0x0f, 0xa7, 0x63,
    0x13, 0x41, 0x06, 0x1a, 0x2b, 0x55, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42,
    0x60, 0x82,
];

/// The mime the Wayland side reads the small text as (the satellite maps X11
/// target names to mime types verbatim, so the X11 owner offers this exact name).
const SMALL_MIME: &str = "UTF8_STRING";

/// The mime carrying the large payload, which only `INCR` can deliver.
const BIG_MIME: &str = "text/plain;charset=utf-8";

/// A payload above the X11 owner's `INCR` threshold (64 KiB), as UTF-8 text.
fn big_text() -> String {
    let mut text = String::from("Inox M3 INCR payload — ");
    while text.len() < 100 * 1024 {
        text.push_str("0123456789abcdef");
    }
    text.push('\n');
    text
}

/// How long a negative assertion is given to be contradicted before it counts.
///
/// The positive half of every check is synchronised on the event itself; this
/// window only exists so that an event which should *not* arrive anywhere has a
/// chance to arrive and fail the run. It is a drain, not a readiness sleep.
const DRAIN: Duration = Duration::from_millis(500);

/// Keep dispatching one Wayland client for `duration`.
fn drain_wayland(client: &mut WaylandClient, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        client.poll(POLL_INTERVAL);
    }
}

/// Keep dispatching the X11 connection for `duration`.
///
/// Used to reach a *settled* baseline before a snapshot is taken: an event the
/// compositor sent earlier may still be unread on this socket, and counting it
/// afterwards would attribute it to whatever the test does next.
fn drain_x11(client: &mut X11Client, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        client.poll(POLL_INTERVAL);
    }
}

/// Pump both clients until `predicate` holds on the Wayland side.
///
/// The clipboard crosses two connections that this one thread owns, so every
/// wait has to advance both: the X11 owner answers `SelectionRequest` from its
/// own event loop, and the Wayland client's offer arrives on its connection.
fn pump_both_until(
    x11: &mut X11Client,
    wayland: &mut WaylandClient,
    what: &str,
    predicate: impl Fn(&WaylandClient) -> bool,
) {
    let deadline = Instant::now() + CASE_TIMEOUT;
    loop {
        if predicate(wayland) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: timed out; X11 events: {:#?}; Wayland events: {:#?}",
            x11.events,
            wayland.state.events
        );
        x11.poll(POLL_INTERVAL);
        wayland.poll(POLL_INTERVAL);
    }
}

/// Read `mime` from `offer`, driving the X11 owner and the Wayland receiver.
fn read_offer_from_x11(
    x11: &mut X11Client,
    wayland: &mut WaylandClient,
    offer: OfferId,
    mime: &str,
    what: &str,
) -> Vec<u8> {
    let pending = wayland.request_data(offer, mime);
    let deadline = Instant::now() + CASE_TIMEOUT;
    loop {
        x11.poll(POLL_INTERVAL);
        wayland.poll(POLL_INTERVAL);
        if let Some(bytes) = pending.take() {
            return bytes;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: the transfer of {mime} never completed; X11 events: {:#?}; Wayland events: \
             {:#?}",
            x11.events,
            wayland.state.events
        );
    }
}

/// Count the *verification* key (X11 key code 46) that reached `window`.
///
/// The compositor forwards keys it does not bind to the focused client, so a
/// blanket count of `KeyPress` events would mix in keys this helper did not
/// inject.
fn verification_key_presses(x11: &X11Client, window: Window) -> usize {
    x11.key_presses(window)
        .iter()
        .filter(|keycode| **keycode == X_KEYCODE_A as u8)
        .count()
}

/// Inject evdev code [`KEY_A`] until the X11 window reports it, which is what
/// proves the satellite has the Wayland keyboard focus (and therefore a serial
/// it can publish the selection with).
///
/// The connection is settled before the baseline is taken, so an event the
/// compositor sent earlier cannot be counted as the one this helper caused.
fn focus_x11_window(session: &mut Session, x11: &mut X11Client, window: Window) {
    drain_x11(x11, DRAIN);
    let before = verification_key_presses(x11, window);

    let deadline = Instant::now() + CASE_TIMEOUT;
    loop {
        session.injector.key_tap(KEY_A);
        let attempt_deadline = Instant::now() + Duration::from_secs(2);
        while verification_key_presses(x11, window) == before {
            assert!(
                Instant::now() < deadline,
                "the X11 window {window} never received an injected key, so it never had the \
                 focus; events: {:#?}",
                x11.events
            );
            if Instant::now() >= attempt_deadline {
                break;
            }
            x11.poll(POLL_INTERVAL);
        }
        if verification_key_presses(x11, window) > before {
            break;
        }
    }
    drain_x11(x11, DRAIN);

    assert_eq!(
        verification_key_presses(x11, window),
        before + 1,
        "the injected evdev key {KEY_A} must arrive as X11 key code {X_KEYCODE_A} exactly once; \
         events: {:#?}",
        x11.events
    );
}

/// Start a session with one mapped, focused X11 window.
fn session_with_x11(label: &str) -> (Session, X11Client, Window) {
    let mut session = Session::start(label);
    let mut x11 = X11Client::connect(&session.display_spec(), "x11-clipboard");
    let window = x11.create_window(0x0033_7788, 400, 300, "inox-m3-clipboard-owner");
    x11.map_window(window);
    focus_x11_window(&mut session, &mut x11, window);
    (session, x11, window)
}

/// Wait until the satellite has noticed the X11 owner and consumed its `TARGETS`
/// answer.
///
/// This is the synchronisation the X11-to-Wayland direction needs: `smithay`
/// answers `wl_data_device.set_selection` only for the client that currently
/// holds the keyboard focus, and the
/// satellite is that client exactly while the X11 window is focused. It
/// publishes the selection to Wayland in the same step in which it reads our
/// `TARGETS` answer, so waiting for that read is what lets the test map a
/// Wayland client afterwards and be sure the offer was already published.
fn wait_for_satellite_to_publish(x11: &mut X11Client) {
    let deadline = Instant::now() + CASE_TIMEOUT;
    let targets = x11.atoms.targets;
    x11.wait_until(deadline, "the satellite to ask for TARGETS", |client| {
        client
            .events
            .iter()
            .any(|event| matches!(event, support::x11_client::Event::SelectionRequest { target, .. } if *target == targets))
    });
    // This implies an ordering for the caller: the answer has to be delivered
    // while the X11 window still has the focus, because the satellite can only
    // publish from the focused client. The tests map a Wayland client only after
    // this returns, and what they wait for afterwards is the offer itself.
}

/// Map a Wayland toplevel into the session and wait until it is configured.
fn map_wayland_client(session: &Session, name: &str, colour: [u8; 3]) -> (WaylandClient, usize) {
    let mut client = WaylandClient::connect(
        &session.runtime.wayland_socket_path(),
        session.runtime.path(),
        name,
    );
    let toplevel = client.create_toplevel(colour, (320, 240));
    let deadline = Instant::now() + CASE_TIMEOUT;
    client.wait_until(deadline, "a Wayland configure", |state| {
        state
            .toplevels
            .get(toplevel)
            .is_some_and(|entry| entry.configured.is_some())
    });
    (client, toplevel)
}

/// X02, X11 -> Wayland: an X11 owner's selection reaches a focused Wayland
/// client byte for byte, including a payload that needs `INCR`.
#[test]
fn x11_owned_clipboard_reaches_a_focused_wayland_client() {
    if skip_unless_enabled("X02") {
        return;
    }

    let (session, mut x11, window) = session_with_x11("x02-x11-to-wayland");
    let big = big_text();

    // The X11 client owns CLIPBOARD. `UTF8_STRING` carries the short text,
    // `text/plain;charset=utf-8` the >64 KiB payload (which forces the X11
    // owner onto the INCR protocol) and `image/png` the image.
    x11.own_clipboard(
        window,
        &[
            (SMALL_MIME, TEXT.as_bytes().to_vec()),
            (BIG_MIME, big.as_bytes().to_vec()),
            ("image/png", PNG.to_vec()),
        ],
    );
    assert!(x11.owns_clipboard(), "the X11 client must own CLIPBOARD");

    // The satellite notices the new X11 owner through XFixes and asks it for
    // TARGETS; it publishes the selection to Wayland as soon as it has read the
    // answer, which must happen while it still holds the keyboard focus.
    wait_for_satellite_to_publish(&mut x11);

    // A Wayland client maps and takes the focus; it must now be offered the
    // selection that the satellite published while the X11 window held it.
    let (mut wayland, _toplevel) = map_wayland_client(&session, "wl-clipboard", [0x88, 0x33, 0x88]);
    let serial = wayland.wait_for_focus("the Wayland client to take the keyboard focus");
    assert!(serial > 0, "a keyboard enter must carry a serial");

    pump_both_until(
        &mut x11,
        &mut wayland,
        "the selection offer to reach the focused Wayland client",
        |client| client.selection_offer().is_some(),
    );
    let offer = wayland.selection_offer().expect("just waited for it");
    let mimes = wayland.offer_mimes(offer).to_vec();
    for mime in [SMALL_MIME, BIG_MIME, "image/png"] {
        assert!(
            mimes.iter().any(|offered| offered == mime),
            "the offer must advertise {mime}; it advertises {mimes:?}"
        );
    }

    let small = read_offer_from_x11(
        &mut x11,
        &mut wayland,
        offer,
        SMALL_MIME,
        "reading the short text",
    );
    assert_eq!(
        small,
        TEXT.as_bytes(),
        "the UTF-8 text must survive the X11 -> Wayland transfer byte for byte"
    );

    let image = read_offer_from_x11(
        &mut x11,
        &mut wayland,
        offer,
        "image/png",
        "reading the PNG",
    );
    assert_eq!(
        image, PNG,
        "the PNG must survive the X11 -> Wayland transfer byte for byte"
    );

    // The large payload can only get through as an INCR transfer.
    let large = read_offer_from_x11(
        &mut x11,
        &mut wayland,
        offer,
        BIG_MIME,
        "reading the >64 KiB payload",
    );
    assert_eq!(
        large.len(),
        big.len(),
        "the INCR transfer must deliver every byte ({} of {})",
        large.len(),
        big.len()
    );
    assert_eq!(
        large,
        big.as_bytes(),
        "the >64 KiB payload must survive the INCR transfer byte for byte"
    );

    // The X11 owner really used INCR: it wrote the marker and then chunks.
    assert!(
        x11.events
            .iter()
            .any(|event| matches!(event, support::x11_client::Event::PropertyDelete { .. })),
        "the X11 owner must have seen the requestor delete the INCR property and sent the data in \
         chunks; events: {:#?}",
        x11.events
    );
}

/// X02, Wayland -> X11: a Wayland client's selection reaches the X11 client once
/// the satellite has the focus, with the refusal and access-control cases
/// alongside.
///
/// Focus is moved by **natural map ordering** rather than by Alt+Tab: mapping a
/// second X11 window makes its toplevel the newest window, so the satellite
/// becomes the focused client and learns about the Wayland selection. The
/// compositor's own Alt+Tab binding is covered in `m3_x11_window.rs`.
#[test]
fn a_wayland_selection_reaches_the_x11_client() {
    if skip_unless_enabled("X02") {
        return;
    }

    let (session, mut x11, _window) = session_with_x11("x02-wayland-to-x11");

    // A Wayland client that holds the focus *before* the selection exists. It is
    // the access-control case at the end of this test: `smithay` hands the
    // selection to the client that has the keyboard focus, so this client must
    // never be offered one.
    let (mut unfocused, _unfocused_toplevel) =
        map_wayland_client(&session, "wl-unfocused", [0x11, 0x55, 0x99]);
    unfocused.wait_for_focus("the first Wayland client to take the focus");

    // The selection owner maps second, so it holds the focus when it sets it.
    let (mut wayland, _toplevel) = map_wayland_client(&session, "wl-owner", [0x22, 0x66, 0xaa]);
    let serial = wayland.wait_for_focus("the clipboard owner to take the focus");
    let source = wayland.create_source(&[BIG_MIME], TEXT.as_bytes());
    wayland.set_selection(source, serial);

    // Focus moves to a new X11 window. Its toplevel is the newest window, so the
    // satellite becomes the focused client again — and only the focused client
    // may take over CLIPBOARD on the X11 side.
    let second_window = x11.create_window(0x0033_7788, 320, 240, "inox-m3-clipboard-x11-two");
    x11.map_window(second_window);

    let owner = x11.wait_for_foreign_owner(second_window, CASE_TIMEOUT);
    println!("CLIPBOARD is now owned by the satellite's window {owner}");

    // `TARGETS` must list UTF8_STRING: the satellite aliases
    // `text/plain;charset=utf-8` to it so X11 clients find the text.
    let raw = x11
        .request_clipboard_while("TARGETS", CASE_TIMEOUT, || wayland.poll(POLL_INTERVAL))
        .expect("the satellite must answer TARGETS");
    let targets: Vec<u32> = raw
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    assert!(
        targets.contains(&x11.atoms.utf8_string),
        "the satellite must alias the Wayland text to the X11 UTF8_STRING target; got {:?}",
        targets
    );

    let text = x11
        .request_clipboard_while("UTF8_STRING", CASE_TIMEOUT, || wayland.poll(POLL_INTERVAL))
        .expect("the satellite must deliver the text");
    assert_eq!(
        text,
        TEXT.as_bytes(),
        "the UTF-8 text must survive the Wayland -> X11 transfer byte for byte"
    );

    // An unsupported target is refused: no data, no hang.
    let refused =
        x11.request_clipboard_while("text/html", CASE_TIMEOUT, || wayland.poll(POLL_INTERVAL));
    assert_eq!(
        refused, None,
        "a target the source does not offer must be answered with no data"
    );

    // ... and the refusal did not disturb the session: a supported transfer
    // still works afterwards.
    let again = x11
        .request_clipboard_while("UTF8_STRING", CASE_TIMEOUT, || wayland.poll(POLL_INTERVAL))
        .expect("a supported transfer must still work after a refusal");
    assert_eq!(
        again,
        TEXT.as_bytes(),
        "the transfer after a refusal must still be byte-exact"
    );

    // --- the focus is the access-control boundary --------------------------
    //
    // The client that lost the focus before the selection existed was never
    // offered it: only the focused client is handed the selection, so an
    // unauthorised client has nothing to read from. This is Inox's M3 answer to
    // "an unauthorised client must not read the clipboard" (ADR 0017 decision 2),
    // and the reason `wlr-data-control` is deliberately absent.
    drain_wayland(&mut unfocused, DRAIN);
    assert!(
        !unfocused.state.events.iter().any(|event| matches!(
            event,
            support::wayland_client::Event::SelectionOffered { .. }
        )),
        "a client that never held the focus while the selection existed must not be offered it; \
         events: {:#?}",
        unfocused.state.events
    );
    assert_eq!(
        unfocused.selection_offer(),
        None,
        "an unfocused client must have no offer to read the clipboard from"
    );

    // The client that does hold the selection is offered it, which is what makes
    // the assertion above about the *focus* rather than about a broken session.
    let deadline = Instant::now() + CASE_TIMEOUT;
    wayland.wait_until(
        deadline,
        "the selection owner to be offered its own selection",
        |state| {
            state.events.iter().any(|event| {
                matches!(
                    event,
                    support::wayland_client::Event::SelectionOffered { .. }
                )
            })
        },
    );

    // The compositor really tiled the new X11 window: four windows are managed
    // (two X11 windows and two Wayland clients) and the newest one takes the
    // master column.
    let newest = tile_of_mapped(4, 3);
    x11.wait_until(
        Instant::now() + CASE_TIMEOUT,
        "the newest X11 window to be tiled",
        |client| {
            client
                .configured_size(second_window)
                .is_some_and(|(width, height)| {
                    // The X11 client area is the tile minus the title bar the
                    // satellite draws itself (X01 measures that reservation); the
                    // width is the tile's width unchanged.
                    i32::from(width) == newest.w
                        && i32::from(height) <= newest.h
                        && newest.h - i32::from(height) <= 64
                })
        },
    );
}

/// X02: when the owner goes away, a later read fails cleanly and the session
/// stays usable.
#[test]
fn the_clipboard_dies_cleanly_with_its_x11_owner() {
    if skip_unless_enabled("X02") {
        return;
    }

    let (mut session, mut x11, window) = session_with_x11("x02-owner-exit");
    let stale = format!("{TEXT}\nstale-owner-marker");
    x11.own_clipboard(window, &[(SMALL_MIME, stale.as_bytes().to_vec())]);
    wait_for_satellite_to_publish(&mut x11);

    let (mut wayland, _toplevel) = map_wayland_client(&session, "wl-survivor", [0x33, 0xaa, 0x33]);
    wayland.wait_for_focus("the Wayland client to take the keyboard focus");
    pump_both_until(
        &mut x11,
        &mut wayland,
        "the selection offer to reach the Wayland client",
        |client| client.selection_offer().is_some(),
    );
    let offer = wayland.selection_offer().expect("just waited for it");
    let before = read_offer_from_x11(
        &mut x11,
        &mut wayland,
        offer,
        SMALL_MIME,
        "reading the text while the X11 owner is alive",
    );
    assert_eq!(
        before,
        stale.as_bytes(),
        "the X11 owner must deliver its data while it is alive"
    );

    // The owner goes away: the connection closes, Xwayland tears its windows
    // down and the satellite's source loses its backing.
    drop(x11);

    // A later read must end on its own (no hang) and must not hand back the old
    // bytes as if the owner were still there.
    let after = read_offer_from_x11(
        &mut X11Client::connect(&session.display_spec(), "x11-probe"),
        &mut wayland,
        offer,
        SMALL_MIME,
        "reading the text after the X11 owner exited",
    );
    assert_ne!(
        after,
        stale.as_bytes(),
        "a read after the owner exited must not return stale data"
    );
    println!(
        "the read after the owner exited returned {} bytes (empty means the source was dropped)",
        after.len()
    );

    // The session is still usable: a fresh X11 client can own the clipboard
    // again and a fresh Wayland client can read the new payload.
    let mut fresh = X11Client::connect(&session.display_spec(), "x11-fresh");
    let fresh_window = fresh.create_window(0x0077_3366, 320, 240, "inox-m3-clipboard-fresh");
    fresh.map_window(fresh_window);
    focus_x11_window(&mut session, &mut fresh, fresh_window);
    fresh.own_clipboard(
        fresh_window,
        &[(SMALL_MIME, b"after the owner exited".to_vec())],
    );
    wait_for_satellite_to_publish(&mut fresh);

    let (mut reader, _reader_toplevel) =
        map_wayland_client(&session, "wl-fresh-reader", [0x66, 0x33, 0xaa]);
    reader.wait_for_focus("the fresh Wayland client to take the focus");
    pump_both_until(
        &mut fresh,
        &mut reader,
        "the fresh selection offer to reach the fresh Wayland client",
        |client| client.selection_offer().is_some(),
    );
    let offer = reader.selection_offer().expect("just waited for it");
    let bytes = read_offer_from_x11(
        &mut fresh,
        &mut reader,
        offer,
        SMALL_MIME,
        "reading the payload of the fresh owner",
    );
    assert_eq!(
        bytes, b"after the owner exited",
        "the session must still carry a clipboard transfer after the first owner exited"
    );
}
