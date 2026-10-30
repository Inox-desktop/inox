#pragma once

#include <QByteArray>
#include <QSize>
#include <QString>
#include <QtGlobal>

#include "shell_surface.h"

// The panel's native host: one `QGuiApplication`, one QML window, one shell
// role.
//
// It is the native counterpart of `offscreen.cpp`, and the difference is the
// whole point of ADR 0046: the window is a real Wayland surface created by Qt's
// wayland platform plugin, drawn by Qt's own scene graph on a
// `wl_surface.frame` clock, composited by the compositor — instead of a
// `QT_QPA_PLATFORM=offscreen` window whose pixels are grabbed and copied into a
// `wl_shm` buffer this process uploaded itself.
//
// # Who owns the event loop
//
// Neither this class nor Qt. The panel's `calloop` loop owns the process (it
// also serves the window service and the clock), and Qt's wayland connection is
// pumped from it by calling `pump()`. That is deliberate: two blocking event
// loops in one thread is not a thing, and moving either to a thread would put
// the projection — one `State`, mutated only by `app::update` — behind a
// channel for no gain.
//
// # The boundary
//
// Rust hands over the projection as one JSON string (`/view.rs`), exactly as
// the offscreen host did, and reads two queues: the shell events above and the
// actions QML produced. Nothing in this file knows what a task or a workspace
// is.
class Host {
public:
  Host();
  ~Host();

  Host(const Host &) = delete;
  Host &operator=(const Host &) = delete;

  // Creates the QML window at `url`, installs `role` on it and shows it.
  //
  // This has to happen in this order — the role before the first show — because
  // a `wl_surface` has exactly one role and Qt creates an `xdg_toplevel` for a
  // window with none (ADR 0046 §1). Returns false with a readable reason when
  // the platform is not Wayland, when the compositor offers no `inox_shell_v1`,
  // or when the document produces no window.
  bool start(const QString &url, quint32 role, QString &error);

  // Replaces the projection the QML root parses.
  bool set_model(const QString &json, QString &error);

  // Sends the placement decided by `placement.rs`.
  //
  // One argument per protocol field rather than a struct: the Rust bridge is
  // `cxx`, which can carry a struct only if it owns the definition, and the
  // structs in `shell_surface.h` belong to the C++ side (they are what the
  // protocol's own request takes).
  bool place(quint32 layer, quint32 anchor, qint32 exclusive_zone, qint32 margin_top,
             qint32 margin_right, qint32 margin_bottom, qint32 margin_left, quint32 width,
             quint32 height, QString &error);

  // Sends the motion hint decided by the configuration's motion tokens.
  bool transition(quint32 enter, quint32 exit, quint32 duration, quint32 curve, quint32 origin,
                  QString &error);

  // Sends the keyboard mode decided by `placement.rs` (D01: `none` for ordinary
  // presentation).
  bool keyboard(quint32 mode, QString &error);

  // Opens a popup of this surface, positioned by relation to it.
  //
  // A popup is a second window with its own surface and the popup role; the
  // compositor draws it directly above its parent and dismisses it on a click
  // outside, on Escape, or when the parent goes away. Returns the popup's id,
  // which is what the `popup dismissed / configured / keyboard` events refer to,
  // or 0 with a readable `error`.
  //
  // The window's *content* arrives with the panel's menus (ADR 0046 P3): this
  // creates the surface and the role, which is what a menu needs before it has
  // anything to draw.
  quint32 open_popup(quint32 parent_anchor, quint32 popup_anchor, qint32 x, qint32 y,
                     quint32 width, quint32 height, bool dismissable, QString &error);

  // Asks the compositor to close a popup, exit transition included.
  bool dismiss_popup(quint32 popup);

  // Creates this surface's glass object, if the compositor offers one.
  //
  // Called after the shell role exists — a surface with no known desktop layer
  // is answered with a zero maximum (ADR 0046 §6) — and only when the material
  // asks for an effect at all (`crate::frost::asks_for_glass`). A compositor
  // without `inox-glass-v1` is a degradation the caller reports once: the pane
  // is drawn as plain translucency.
  bool start_glass(QString &error);

  // Sends one `inox-glass-v1` parameter set, in the protocol's order.
  //
  // One argument per protocol field, like `place`: the numbers are what
  // `inox_glass::Request` decided, already clamped to what the compositor
  // answered.
  bool glass(quint32 radius, qint32 x, qint32 y, qint32 width, qint32 height, quint32 top_left,
             quint32 top_right, quint32 bottom_right, quint32 bottom_left, bool refraction,
             quint32 band_height, quint32 band_amount, quint32 flags, QString &error);

  // The version of this surface's glass object: 0 for none, 1 or 2 otherwise.
  //
  // What the caller needs it for is one decision: whether the version 2
  // refraction vocabulary may be sent at all.
  quint32 glass_version() const;

  // Drains Qt's event dispatch once. Called by the panel's loop.
  void pump();

  // Unmaps the surface without destroying it.
  //
  // Hiding is what makes the compositor dismiss this surface's popups with
  // `parent_hidden`: a menu cannot outlive the surface it belongs to, and the
  // compositor is the one that decides that, not the client (ADR 0046 §2).
  void hide();

  // Pops the next shell event: 1 capabilities, 2 configure, 3 workarea,
  // 4 keyboard state, 5 closed, 6 popup dismissed, 7 popup configured,
  // 8 popup keyboard state. 0 when the queue is empty.
  quint32 take_event(qint32 &a, qint32 &b, qint32 &c, qint32 &d);

  // Pops the next action QML produced (`{"type":"activate","id":7}`), or an
  // empty array when there is none.
  QByteArray take_action();

  // The size the compositor configured, in logical pixels.
  QSize size() const;

  // The same size as two integers, for the bridge: `QSize` would have to be a
  // shared type and there is nothing else to send.
  qint32 width() const;
  qint32 height() const;

  // Whether the window has a shell role and the compositor has configured it.
  bool configured() const;

  // The reason the last failure happened, in words an operator can act on.
  QString last_error() const;

  // The window the role belongs to. Null before `start`.
  QWindow *window() const;

private:
  // Moves what QML appended to the root's `outbox` into the action queue.
  void collect_actions();

  struct Impl;
  Impl *d;
};

// Creates a host. The one `QGuiApplication` comes with it, and the platform is
// whatever the session set: this host needs `wayland` and reports the failure
// rather than choosing one.
std::unique_ptr<Host> new_host();
