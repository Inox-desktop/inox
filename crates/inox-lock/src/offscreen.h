#pragma once
#include <QByteArray>
#include <QString>
#include <memory>

// One offscreen Qt Quick host for one lock surface.
//
// The process owns a single `QGuiApplication` (Qt allows exactly one), created
// on first use on the offscreen platform with the software scene graph. Each
// `Offscreen` has its own `QQmlApplicationEngine` and its own `QQuickWindow`, so
// several of them can coexist at different sizes: a session with two outputs has
// two lock surfaces, and one host per surface is what lets each frame be grabbed
// at the size its own compositor configure asked for.
class Offscreen {
public:
  Offscreen();
  ~Offscreen();

  // Load the QML module's window at an initial size. The size has to be known
  // before the window exists because it is what the QML root binds its
  // `width`/`height` to; later frames resize the window directly.
  bool load(const QString &url, int width, int height);

  // Push the projection QML renders. The JSON is the only data that crosses
  // into the scene; the credential never does.
  void setModel(const QString &json);

  // Grab the window at `width` x `height` as RGBA8888 premultiplied pixels.
  // Empty when the window is not loaded, or when the grab is not the size that
  // was asked for.
  QByteArray render(int width, int height);

private:
  struct Impl;
  Impl *d;
};

std::unique_ptr<Offscreen> new_offscreen();
