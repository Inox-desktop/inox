//! Acceptance X03 (`docs/quality/acceptance.md` §5): drag and drop inside an
//! Inox headless session.
//!
//! Evidence level E3: the workspace's own compositor is driven as a process (with
//! the test-only injection socket) and two real `wayland-client` connections do
//! the dragging. The pointer is injected for real — press, motion, release — so
//! the drag runs through the compositor's own implicit grab and `DnDGrab`, and
//! the serial the source quotes in `start_drag` is the one the compositor handed
//! out for that press.
//!
//! Covered here: accept/drop with both mimes advertised and the bytes compared,
//! the source's single terminal event (`dnd_drop_performed` followed by
//! `dnd_finished`), a target that accepts nothing (the source is cancelled and
//! the target gets no drop), a drag released outside every surface (again exactly
//! one terminal event, and a later drag still works, which is the grab-cleanup
//! assertion), and a target destroyed in mid-drag (the session survives and a new
//! client can still map).
//!
//! # Limitation: the X11 <-> Wayland leg cannot be tested, and is not skipped
//!
//! The cross-boundary leg — dragging from an X11 window to a Wayland surface or
//! back — is blocked upstream, not by Inox: `xwayland-satellite` v0.8.3
//! implements no XDND at all (no `XdndEnter`, `XdndDrop` or `XdndAware`
//! handling), so the host bridge has nothing to carry a cross-protocol drag.
//! That is recorded as the XWayland clipboard and DnD work, rule 5, and X03 is therefore a *partial*
//! delivery of M3 that must not be read as X03 passing whole. X11-to-X11 drag
//! and drop keeps working natively inside Xwayland and needs nothing from Inox.
//!
//! Writing a test that quietly skips this leg would hide exactly the gap the ADR
//! records, so this binary announces the limitation in its output instead; see
//! [`announce_limitation`].

mod support;

use std::time::{Duration, Instant};

use support::wayland_client::{Event, OfferId, ToplevelId, WaylandClient};
use support::{
    BTN_LEFT, POLL_INTERVAL, Session, expected_tiles, skip_unless_enabled, tile_of_mapped,
};

/// `wl_data_device_manager::DndAction::Copy` (the protocol's bitflag value).
const ACTION_COPY: u32 = 1;

/// The mime carrying the dragged payload.
const TEXT_MIME: &str = "text/plain;charset=utf-8";
/// The second mime a dragged file list offers.
const URI_MIME: &str = "text/uri-list";

/// The payload the source serves for the text mime.
const TEXT: &str = "Inox M3 — dragged text 🦊\n";
/// The payload the source serves for the uri list mime.
const URIS: &str = "file:///tmp/inox-m3-dragged.txt\r\n";

/// How long a drag is given to complete, from press to terminal event.
const DRAG_TIMEOUT: Duration = Duration::from_secs(20);

/// Announce, once per test, that the X11/Wayland drag boundary is not covered.
///
/// See the module documentation: this is an upstream limitation recorded in
/// the XWayland clipboard and DnD work, rule 5, not a test that gave up.
fn announce_limitation() {
    println!(
        "X03 covers Wayland<->Wayland drag and drop only: the X11<->Wayland leg needs XDND, which \
         xwayland-satellite v0.8.3 does not implement; X03 is a partial \
         delivery of M3"
    );
}

/// A two-client session: `source` on the master tile, `target` on the stack tile.
struct Pair {
    session: Session,
    source: WaylandClient,
    target: WaylandClient,
    source_top: ToplevelId,
}

impl Pair {
    /// Start a compositor-only session with two mapped Wayland clients.
    fn start(label: &str) -> Self {
        let session = Session::start_compositor(label);

        let mut source = WaylandClient::connect(
            &session.runtime.wayland_socket_path(),
            session.runtime.path(),
            "drag-source",
        );
        let source_top = source.create_toplevel([0xcc, 0x44, 0x44], (320, 240));
        source.wait_for_configure(source_top, "the source window to be configured");

        let mut target = WaylandClient::connect(
            &session.runtime.wayland_socket_path(),
            session.runtime.path(),
            "drag-target",
        );
        let target_top = target.create_toplevel([0x44, 0x44, 0xcc], (320, 240));
        target.wait_for_configure(target_top, "the target window to be configured");

        // Both windows are managed and the compositor's workspace puts every
        // newly mapped window in the master column, so the target ends up on the
        // left and the source is pushed into the stack column. Each client only
        // sees its own re-configure on its own connection.
        let (source_tile, target_tile) = (tile_of_mapped(2, 0), tile_of_mapped(2, 1));
        source.wait_for_size(
            source_top,
            (source_tile.w, source_tile.h),
            "the source window to be tiled",
        );
        target.wait_for_size(
            target_top,
            (target_tile.w, target_tile.h),
            "the target window to be tiled",
        );

        Self {
            session,
            source,
            target,
            source_top,
        }
    }

    /// The centres of the two windows' tiles.
    ///
    /// The source was mapped first, so it sits in the stack column; the target,
    /// mapped second, is the master.
    fn centres(&self) -> ((f64, f64), (f64, f64)) {
        let (sx, sy) = tile_of_mapped(2, 0).centre();
        let (tx, ty) = tile_of_mapped(2, 1).centre();
        (
            (f64::from(sx), f64::from(sy)),
            (f64::from(tx), f64::from(ty)),
        )
    }

    /// Press the left button over the source window and return the serial of
    /// *this* press, which is the only serial `start_drag` accepts.
    ///
    /// Counting the presses is what makes a second drag in the same session
    /// work: reading "the latest press" would hand the previous press's serial
    /// to `start_drag`, and the compositor rejects a serial that is not the
    /// current implicit grab (it logs "denying drag from client without implicit
    /// grab").
    fn press_on_source(&mut self) -> u32 {
        let before = self
            .source
            .count(|event| matches!(event, Event::PointerButton { pressed: true, .. }));

        let (source_centre, _) = self.centres();
        self.session
            .injector
            .motion(source_centre.0, source_centre.1);
        self.session.injector.button(BTN_LEFT, true);

        let deadline = Instant::now() + DRAG_TIMEOUT;
        self.source.wait_until(
            deadline,
            "the injected pointer press to reach the source window",
            |state| {
                state
                    .events
                    .iter()
                    .filter(|event| matches!(event, Event::PointerButton { pressed: true, .. }))
                    .count()
                    > before
            },
        );
        self.source
            .state
            .events
            .iter()
            .filter_map(|event| match event {
                Event::PointerButton {
                    serial,
                    pressed: true,
                    ..
                } => Some(*serial),
                _ => None,
            })
            .nth(before)
            .expect("just waited for the press")
    }

    /// Start a drag from the source window by `serial`.
    ///
    /// The text and uri-list mimes carry different payloads, so a transfer that
    /// quietly served the wrong one would be caught by the byte comparison.
    fn start_drag(&mut self, serial: u32, mimes: &[&str]) -> usize {
        let payloads: Vec<(&str, Vec<u8>)> = mimes
            .iter()
            .map(|mime| {
                if *mime == URI_MIME {
                    (*mime, URIS.as_bytes().to_vec())
                } else {
                    (*mime, TEXT.as_bytes().to_vec())
                }
            })
            .collect();

        let source = self
            .source
            .create_source_with(payloads.iter().map(|(mime, bytes)| (*mime, bytes.clone())));
        // Copy is the action this source is willing to perform.
        self.source.source_set_actions(source, ACTION_COPY);
        self.source
            .start_drag(source, self.source_top, None, serial);
        source
    }

    /// Move the pointer onto the target window and wait for *this* drag to enter.
    ///
    /// The wait counts `enter` events rather than looking for any: a test that
    /// starts a second drag would otherwise match the first drag's `enter` and
    /// hand a dead offer to `accept`.
    fn move_onto_target(&mut self) -> OfferId {
        let before = self
            .target
            .count(|event| matches!(event, Event::DragEnter { .. }));

        let (_, target_centre) = self.centres();
        self.session
            .injector
            .motion(target_centre.0, target_centre.1);

        let deadline = Instant::now() + DRAG_TIMEOUT;
        loop {
            let enters = self
                .target
                .count(|event| matches!(event, Event::DragEnter { .. }));
            if enters > before {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the drag never entered the target window (this is target enter number {}); \
                 target events: {:#?}; source events: {:#?}; compositor log:\n{}",
                before + 1,
                self.target.state.events,
                self.source.state.events,
                self.session.compositor.diagnostics()
            );
            self.source.poll(POLL_INTERVAL);
            self.target.poll(POLL_INTERVAL);
        }
        self.target
            .state
            .events
            .iter()
            .filter_map(|event| match event {
                Event::DragEnter { offer } => Some(*offer),
                _ => None,
            })
            .nth(before)
            .expect("just waited for the drag enter")
    }

    /// Read the dropped payload from the target, driving both connections.
    ///
    /// The bytes travel over a pipe the *target* reads and the *source* writes,
    /// so both clients have to be dispatched for the transfer to complete; a
    /// helper that only drove the target would sit there until its timeout.
    fn read_payload(&mut self, offer: OfferId) -> Vec<u8> {
        support::wayland_client::transfer(
            &mut self.source,
            &mut self.target,
            offer,
            TEXT_MIME,
            DRAG_TIMEOUT,
        )
    }

    /// Drive both clients until the source's terminal drag events are exactly
    /// `expected`, and fail with both event logs when they never are.
    ///
    /// A drag ends for the source once. Waiting for the *whole* list rather than
    /// for "some event" is what makes "exactly one terminal event" an assertion
    /// instead of a race: `dnd_drop_performed` arrives before `dnd_finished`, so
    /// an earlier snapshot legitimately contains only the first of the two.
    fn wait_for_terminal(&mut self, expected: &[Event], what: &str) {
        let deadline = Instant::now() + DRAG_TIMEOUT;
        loop {
            let terminal = self.source.terminal_events();
            if terminal == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: the source's terminal drag events should be {expected:?}, they are \
                 {terminal:?}; source events: {:#?}; target events: {:#?}",
                self.source.state.events,
                self.target.state.events
            );
            self.source.poll(POLL_INTERVAL);
            self.target.poll(POLL_INTERVAL);
        }
    }
}

/// X03: an accepted drop transfers the bytes and finishes on the source.
#[test]
fn an_accepted_drop_transfers_the_bytes_and_finishes() {
    if skip_unless_enabled("X03") {
        return;
    }
    announce_limitation();

    let mut pair = Pair::start("x03-accept");
    let serial = pair.press_on_source();
    pair.start_drag(serial, &[TEXT_MIME, URI_MIME]);

    let offer = pair.move_onto_target();
    let mimes = pair.target.offer_mimes(offer).to_vec();
    for mime in [TEXT_MIME, URI_MIME] {
        assert!(
            mimes.iter().any(|offered| offered == mime),
            "the drag offer must advertise {mime}; it advertises {mimes:?}"
        );
    }

    // The target accepts the text mime and a Copy action.
    let (_, target_centre) = pair.centres();
    pair.target
        .accept_drag(offer, 0, Some(TEXT_MIME), ACTION_COPY);
    pair.session
        .injector
        .motion(target_centre.0 - 4.0, target_centre.1);

    // Release over the target: that is the drop.
    pair.session.injector.button(BTN_LEFT, false);
    let deadline = Instant::now() + DRAG_TIMEOUT;
    pair.target
        .wait_until(deadline, "the drop on the target", |state| {
            state
                .events
                .iter()
                .any(|event| matches!(event, Event::DragDrop))
        });

    let bytes = pair.read_payload(offer);
    assert_eq!(
        bytes,
        TEXT.as_bytes(),
        "the dropped payload must equal what the source offered, byte for byte"
    );

    // Finishing is what turns the drop into a completed drag on the source.
    pair.target.finish_offer(offer);
    pair.wait_for_terminal(
        &[Event::SourceDropPerformed, Event::SourceFinished],
        "after the target finished the offer",
    );

    assert_eq!(
        pair.target.drop_count(),
        1,
        "the target must see exactly one drop; events: {:#?}",
        pair.target.state.events
    );
    assert!(
        pair.source.state.events.iter().any(
            |event| matches!(event, Event::SourceAction { action } if action.contains("Copy"))
        ),
        "the negotiated action must reach the source; events: {:#?}",
        pair.source.state.events
    );
}

/// X03: a target that accepts nothing cancels the drag rather than dropping it.
#[test]
fn a_target_that_accepts_nothing_cancels_the_drag() {
    if skip_unless_enabled("X03") {
        return;
    }
    announce_limitation();

    let mut pair = Pair::start("x03-reject");
    let serial = pair.press_on_source();
    pair.start_drag(serial, &[TEXT_MIME, URI_MIME]);

    let offer = pair.move_onto_target();
    // The target refuses: `accept(None)` and no action is ever chosen.
    pair.target.reject_drag(offer, 0);

    pair.session.injector.button(BTN_LEFT, false);
    pair.wait_for_terminal(
        &[Event::SourceCancelled],
        "after the target refused the drop",
    );

    assert_eq!(
        pair.target.drop_count(),
        0,
        "a refused drag must not deliver a drop to the target; events: {:#?}",
        pair.target.state.events
    );
    assert!(
        pair.source
            .state
            .events
            .iter()
            .all(|event| !matches!(event, Event::SourceDropPerformed | Event::SourceFinished)),
        "a refused drag must not report a performed drop; events: {:#?}",
        pair.source.state.events
    );
}

/// X03: releasing outside every surface cancels, and a later drag still works.
#[test]
fn a_drag_released_outside_every_surface_cancels_and_leaves_no_grab_behind() {
    if skip_unless_enabled("X03") {
        return;
    }
    announce_limitation();

    let mut pair = Pair::start("x03-cancel");
    let serial = pair.press_on_source();
    pair.start_drag(serial, &[TEXT_MIME, URI_MIME]);
    let _offer = pair.move_onto_target();

    // The gap between the master and stack columns belongs to no surface, and
    // neither does the margin around the work area.
    let master = expected_tiles(2)[0];
    let gap_x = f64::from(master.x + master.w + 2);
    let gap_y = f64::from(master.y + master.h / 2);
    pair.session.injector.motion(gap_x, gap_y);

    let deadline = Instant::now() + DRAG_TIMEOUT;
    pair.target
        .wait_until(deadline, "the drag to leave the target window", |state| {
            state
                .events
                .iter()
                .any(|event| matches!(event, Event::DragLeave))
        });

    // Release outside: there is no surface to drop onto.
    pair.session.injector.button(BTN_LEFT, false);
    pair.wait_for_terminal(
        &[Event::SourceCancelled],
        "after releasing outside every surface",
    );

    assert_eq!(
        pair.target.drop_count(),
        0,
        "a drag released outside every surface must not drop on the target; events: {:#?}",
        pair.target.state.events
    );

    // The grab is gone: a fresh drag completes normally.
    let before = pair.source.terminal_events().len();
    let serial = pair.press_on_source();
    pair.start_drag(serial, &[TEXT_MIME, URI_MIME]);
    let offer = pair.move_onto_target();
    pair.target
        .accept_drag(offer, 0, Some(TEXT_MIME), ACTION_COPY);
    let (_, target_centre) = pair.centres();
    pair.session
        .injector
        .motion(target_centre.0 - 4.0, target_centre.1);
    pair.session.injector.button(BTN_LEFT, false);

    let deadline = Instant::now() + DRAG_TIMEOUT;
    pair.target
        .wait_until(deadline, "the second drag's drop", |state| {
            state
                .events
                .iter()
                .any(|event| matches!(event, Event::DragDrop))
        });
    let bytes = pair.read_payload(offer);
    assert_eq!(
        bytes,
        TEXT.as_bytes(),
        "the drag after a cancellation must transfer the payload normally"
    );
    pair.target.finish_offer(offer);
    pair.wait_for_terminal(
        &[
            Event::SourceCancelled,
            Event::SourceDropPerformed,
            Event::SourceFinished,
        ],
        "after the second drag finished",
    );
    assert_eq!(
        pair.source.terminal_events().len(),
        before + 2,
        "the second drag must add exactly one drop-performed and one finished event; events: {:#?}",
        pair.source.state.events
    );
}

/// X03: destroying the target in mid-drag must not take the session down.
#[test]
fn destroying_the_target_mid_drag_keeps_the_session_alive() {
    if skip_unless_enabled("X03") {
        return;
    }
    announce_limitation();

    let mut pair = Pair::start("x03-target-destroyed");
    let serial = pair.press_on_source();
    pair.start_drag(serial, &[TEXT_MIME, URI_MIME]);
    let _offer = pair.move_onto_target();

    // The target's whole connection goes away, as if the application crashed.
    let Pair {
        mut session,
        source,
        target,
        source_top: _,
    } = pair;
    drop(target);

    // Release the button: the drag has nowhere to go any more, and the source
    // must still be told exactly once that the drag ended.
    session.injector.button(BTN_LEFT, false);

    let mut source = source;
    let deadline = Instant::now() + DRAG_TIMEOUT;
    source.wait_until(
        deadline,
        "a terminal event after the target died",
        |state| {
            state.events.iter().any(|event| {
                matches!(
                    event,
                    Event::SourceCancelled | Event::SourceDropPerformed | Event::SourceFinished
                )
            })
        },
    );
    assert_eq!(
        source.terminal_events(),
        vec![Event::SourceCancelled],
        "the source must see exactly one terminal event, and a destroyed target cannot complete a \
         drag; events: {:#?}",
        source.state.events
    );

    // The compositor survived and still manages windows.
    session.assert_compositor_alive("after the drag target was destroyed");
    let mut newcomer = WaylandClient::connect(
        &session.runtime.wayland_socket_path(),
        session.runtime.path(),
        "after-target-destroyed",
    );
    let toplevel = newcomer.create_toplevel([0x44, 0xcc, 0x44], (320, 240));
    let (width, height) = newcomer.wait_for_configure(toplevel, "the newcomer to be configured");
    let newcomer_tile = tile_of_mapped(2, 1);
    assert_eq!(
        (width, height),
        (newcomer_tile.w, newcomer_tile.h),
        "a new client must still be tiled after the destroyed target was cleaned up; events: {:#?}",
        newcomer.state.events
    );
    drop(source);
}
