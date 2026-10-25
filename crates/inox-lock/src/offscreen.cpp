#include "offscreen.h"

#include <QCoreApplication>
#include <QGuiApplication>
#include <QImage>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickWindow>
#include <QStringLiteral>

struct Offscreen::Impl {
  QQmlApplicationEngine *engine = nullptr;
  QQuickWindow *window = nullptr;
  QObject *bridge = nullptr;
  int rendered_width = 0;
  int rendered_height = 0;
};

// The one application object this process may have.
//
// The platform and the scene graph are selected *before* it exists, which is the
// only moment Qt reads either: after `QGuiApplication` is constructed the
// platform cannot be changed. The lock surface is not a Qt window, so
// the offscreen platform is not a compromise here — nothing is ever presented by
// Qt, and the pixels are handed to the crate's own `wl_shm` frame instead.
static QGuiApplication *ensure_app() {
  static QGuiApplication *app = [] {
    qputenv("QT_QPA_PLATFORM", "offscreen");
    qputenv("QT_QUICK_BACKEND", "software");
    static int argc = 1;
    static char program[] = "inox-lock";
    static char *argv[] = {program, nullptr};
    return new QGuiApplication(argc, argv);
  }();

  return app;
}

Offscreen::Offscreen() : d(new Impl) { ensure_app(); }

Offscreen::~Offscreen() {
  delete d->engine;
  delete d;
}

bool Offscreen::load(const QString &url, int width, int height) {
  ensure_app();
  d->engine = new QQmlApplicationEngine();
  // The initial size is handed to the QML root as context properties: a window
  // cannot be created without one, and this crate does not put pixel literals in
  // QML (contract §4).
  d->engine->rootContext()->setContextProperty(QStringLiteral("surfaceWidth"), width);
  d->engine->rootContext()->setContextProperty(QStringLiteral("surfaceHeight"), height);
  d->engine->load(QUrl(url));

  const auto roots = d->engine->rootObjects();
  if (roots.isEmpty()) {
    return false;
  }

  d->window = qobject_cast<QQuickWindow *>(roots.first());
  if (!d->window) {
    return false;
  }

  // The QML tree owns the bridge object; the host finds it by its object name so
  // the projection can be pushed without a QML type being exposed to C++.
  d->bridge = d->window->findChild<QObject *>(QStringLiteral("lockBridge"));
  return d->bridge != nullptr;
}

void Offscreen::setModel(const QString &json) {
  if (d->bridge) {
    d->bridge->setProperty("modelJson", json);
  }
}

QByteArray Offscreen::render(int width, int height) {
  if (!d->window || width <= 0 || height <= 0) {
    return {};
  }

  if (d->window->width() != width || d->window->height() != height) {
    d->window->resize(width, height);
  }
  // A model change only reaches the scene graph once the event loop has run the
  // binding and the window has laid out and painted. There is no window system
  // to signal that, so the pump is explicit: this is the same "process events
  // until it settles" the offscreen probe recorded.
  for (int i = 0; i < 20; ++i) {
    QCoreApplication::processEvents();
  }

  const QImage image =
      d->window->grabWindow().convertToFormat(QImage::Format_RGBA8888_Premultiplied);
  if (image.width() != width || image.height() != height) {
    // A frame of the wrong size is worse than no frame: the caller has a buffer
    // of the configured size, and a grab at another size would write pixels for
    // one surface into the memory declared for another.
    return {};
  }

  // Copied row by row rather than as one blob: `QImage` pads its scanlines, and
  // the lock frame's length is a protocol requirement (`width * height * 4`).
  QByteArray out;
  out.resize(width * height * 4);
  char *target = out.data();
  for (int y = 0; y < height; ++y) {
    memcpy(target, image.constScanLine(y), static_cast<size_t>(width) * 4);
    target += static_cast<size_t>(width) * 4;
  }
  d->rendered_width = width;
  d->rendered_height = height;
  return out;
}

std::unique_ptr<Offscreen> new_offscreen() { return std::make_unique<Offscreen>(); }
