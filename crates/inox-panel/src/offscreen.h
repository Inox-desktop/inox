#pragma once
#include <QByteArray>
#include <QString>
#include <memory>

// The panel's offscreen QML host.
//
// It owns exactly one `QGuiApplication` for the process, one
// `QQmlApplicationEngine` loading the panel's QML module window, and the
// `QQuickWindow` that engine created. `render()` resizes that window, renders a
// frame with Qt Quick's software scene graph, and returns the RGBA8
// **premultiplied** pixels — with the real alpha channel — that the caller
// copies into the layer surface's `wl_shm` buffer.
//
// The alpha is not something every Qt gives away: before 6.8 the offscreen
// platform's backing store is hard-wired to `QImage::Format_RGB32`, so the grab
// of a translucent frame comes back opaque. `render()` detects that once and
// reconstructs the alpha from two grabs over a black and a white backdrop (see
// `src/offscreen.cpp`); the pixels it returns are the same either way.
//
// It creates no Qt Wayland client, no `QWindow` on any real platform and no
// layer-shell object: the platform is forced to `offscreen` before the
// application exists, so the process cannot become an `xdg_toplevel`.
class Offscreen {
public:
  Offscreen();
  ~Offscreen();

  // Loads the QML module window at `url`. False when the engine produced no
  // root object, or the root is not a window.
  bool load(const QString &url);

  // Hands the current JSON projection to the QML root as `modelJson`. The
  // renderer parses it; nothing here interprets it.
  void setModel(const QString &model);

  // Renders one `width` x `height` frame and returns it as RGBA8888
  // premultiplied bytes (empty on failure).
  QByteArray render(int width, int height);

  // The reason the last call failed, in words an operator can act on.
  QString lastError() const;

  // Whether the alpha channel had to be reconstructed from two opaque grabs,
  // because this Qt's offscreen backing store has none. False before the first
  // frame. A diagnostic, and the way a test can tell which pixel path a frame
  // came from.
  bool alphaReconstructed() const;

private:
  struct Impl;
  Impl *d;
};

std::unique_ptr<Offscreen> new_offscreen();
