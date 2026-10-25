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
// **premultiplied** pixels the caller copies into the layer surface's `wl_shm`
// buffer.
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

private:
  struct Impl;
  Impl *d;
};

std::unique_ptr<Offscreen> new_offscreen();
