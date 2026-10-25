#include "offscreen.h"

#include <QCoreApplication>
#include <QGuiApplication>
#include <QImage>
#include <QQmlApplicationEngine>
#include <QQmlEngine>
#include <QQuickWindow>
#include <QUrl>

// The offscreen QML host of the panel.
//
// Three things are deliberate and load-bearing:
//
//   1. `QT_QPA_PLATFORM=offscreen` and `QT_QUICK_BACKEND=software` are set
//      **before** the one `QGuiApplication` is constructed. The panel must not
//      acquire a Qt Wayland connection: its surface is the `wlr-layer-shell`
//      one Rust owns, and a Qt client platform would try to become an
//      `xdg_toplevel`.
//   2. There is exactly one `QGuiApplication` per process. Qt allows no
//      second one, and the panel creates one interface per surface.
//   3. `grabWindow()` returns the frame the software scene graph rendered —
//      RGBA8, premultiplied. That is the only pixel format the layer surface's
//      `wl_shm` buffer accepts once the channels are reordered.

struct Offscreen::Impl {
  QQmlApplicationEngine *engine = nullptr;
  QQuickWindow *window = nullptr;
  QString error;
};

namespace {

QGuiApplication *ensure_app() {
  static QGuiApplication *app = nullptr;

  if (!app) {
    // Before the application: a platform plugin is chosen when it is created.
    qputenv("QT_QPA_PLATFORM", "offscreen");
    qputenv("QT_QUICK_BACKEND", "software");

    static int argc = 1;
    static char name[] = "inox-panel";
    static char *argv[] = {name, nullptr};

    app = new QGuiApplication(argc, argv);
  }

  return app;
}

} // namespace

Offscreen::Offscreen() : d(new Impl) { (void)ensure_app(); }

Offscreen::~Offscreen() {
  delete d->engine;
  delete d;
}

bool Offscreen::load(const QString &url) {
  if (d->window) {
    return true;
  }

  d->engine = new QQmlApplicationEngine();
  d->engine->load(QUrl(url));

  const QList<QObject *> roots = d->engine->rootObjects();

  if (roots.isEmpty()) {
    d->error = QStringLiteral("the QML engine produced no root object for %1 "
                              "(a missing module, a syntax error or a missing "
                              "QML import)")
                   .arg(url);
    return false;
  }

  d->window = qobject_cast<QQuickWindow *>(roots.first());

  if (!d->window) {
    d->error = QStringLiteral("%1 is not a window, so there is nothing to grab")
                   .arg(url);
    return false;
  }

  d->error.clear();

  return true;
}

void Offscreen::setModel(const QString &model) {
  if (!d->window) {
    return;
  }

  d->window->setProperty("modelJson", model);
}

QByteArray Offscreen::render(int width, int height) {
  if (!d->window) {
    d->error = QStringLiteral("the QML window is not loaded");
    return {};
  }

  if (width <= 0 || height <= 0) {
    d->error = QStringLiteral("refusing a %1x%2 frame").arg(width).arg(height);
    return {};
  }

  d->window->resize(width, height);
  d->window->requestUpdate();

  // The software backend renders in step with the event loop, so the frame
  // that is grabbed is the one after the layout and the painting above.
  for (int i = 0; i < 8; ++i) {
    QCoreApplication::processEvents();
  }

  const QImage image =
      d->window->grabWindow().convertToFormat(QImage::Format_RGBA8888_Premultiplied);

  if (image.isNull()) {
    d->error = QStringLiteral("the QML window produced no image to grab");
    return {};
  }

  if (image.width() != width || image.height() != height) {
    d->error = QStringLiteral("the QML window rendered %1x%2 for a %3x%4 "
                              "surface")
                   .arg(image.width())
                   .arg(image.height())
                   .arg(width)
                   .arg(height);
    return {};
  }

  d->error.clear();

  return QByteArray(reinterpret_cast<const char *>(image.constBits()),
                    image.sizeInBytes());
}

QString Offscreen::lastError() const { return d->error; }

std::unique_ptr<Offscreen> new_offscreen() { return std::make_unique<Offscreen>(); }
