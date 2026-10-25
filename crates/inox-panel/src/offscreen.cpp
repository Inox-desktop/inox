#include "offscreen.h"

#include <QColor>
#include <QCoreApplication>
#include <QGuiApplication>
#include <QImage>
#include <QQmlApplicationEngine>
#include <QQmlEngine>
#include <QQuickWindow>
#include <QSurfaceFormat>
#include <QUrl>

// The offscreen QML host of the panel.
//
// Four things are deliberate and load-bearing:
//
//   1. `QT_QPA_PLATFORM=offscreen` and `QT_QUICK_BACKEND=software` are set
//      **before** the one `QGuiApplication` is constructed. The panel must not
//      acquire a Qt Wayland connection: its surface is the `wlr-layer-shell`
//      one Rust owns, and a Qt client platform would try to become an
//      `xdg_toplevel`.
//   2. There is exactly one `QGuiApplication` per process. Qt allows no
//      second one, and the panel creates one interface per surface.
//   3. `grabWindow()` returns the frame the software scene graph rendered —
//      RGBA8, premultiplied, **with the real alpha channel**. That is the only
//      pixel format the layer surface's `wl_shm` buffer accepts once the
//      channels are reordered, and the alpha is what makes a glass pane a pane
//      instead of an opaque square.
//   4. The alpha channel is not something every Qt gives away, so `render()`
//      asks for it and, when the answer is no, reconstructs it from two opaque
//      grabs (`reconstruct_alpha` below).
//
// The request (`ensure_app`) is the first attempt: the window is created with an
// 8-bit alpha buffer, and Qt honours it wherever the platform backing store can
// carry one. On Qt 6.8 and later the offscreen plugin's backing store is
// `QImage::Format_ARGB32_Premultiplied` when the window's format has alpha, so
// the grab simply has the channel. Up to Qt 6.4 (what the CI image ships),
// `QOffscreenBackingStore::resize` in
// `src/plugins/platforms/offscreen/qoffscreencommon.cpp` hard-wires the image to
// `QGuiApplication::primaryScreen()->handle()->format()`, which is
// `QImage::Format_RGB32`: no window flag, surface format or `setDefaultAlphaBuffer`
// can change that, and the grab is opaque by construction. Hence the second
// path — it is not dead code.

struct Offscreen::Impl {
  QQmlApplicationEngine *engine = nullptr;
  QQuickWindow *window = nullptr;
  QString error;
  // Which pixel path this host takes: -1 until the first frame decides, 1 when
  // the platform's grab carries alpha, 0 when the alpha has to be reconstructed
  // from two opaque grabs. The decision costs one grab and is cached, so an
  // alpha-capable Qt never pays for a second render.
  int alpha_path = -1;
  // Test hook (`INOX_PANEL_OFFSCREEN_NO_ALPHA_GRAB`): behave like a Qt whose
  // grab has no alpha channel, so the reconstruction path can be exercised on a
  // Qt that would take the fast one.
  bool force_reconstruction = false;
};

namespace {

QGuiApplication *ensure_app() {
  static QGuiApplication *app = nullptr;

  if (!app) {
    // Before the application: a platform plugin is chosen when it is created.
    qputenv("QT_QPA_PLATFORM", "offscreen");
    qputenv("QT_QUICK_BACKEND", "software");

    // Every surface is structural transparency with a translucent glass pane (or
    // an opaque strip) drawn on it — the layer surface is `Argb8888` and the
    // root background is `#00000000` — so the frame's alpha is content, not
    // decoration. A window carries an alpha channel in its grab only when its
    // surface format asked for one, and the request has to be made *before* the
    // window exists: the QML engine creates it inside `load()`. This is the
    // cheap half of the answer; a platform whose backing store cannot hold the
    // channel ignores it, and `render()` reconstructs the alpha there.
    QQuickWindow::setDefaultAlphaBuffer(true);

    QSurfaceFormat format = QSurfaceFormat::defaultFormat();
    format.setAlphaBufferSize(8);
    QSurfaceFormat::setDefaultFormat(format);

    static int argc = 1;
    static char name[] = "inox-panel";
    static char *argv[] = {name, nullptr};

    app = new QGuiApplication(argc, argv);
  }

  return app;
}

// How many event-loop passes the software backend needs to lay out, paint and
// publish the frame that is about to be grabbed.
constexpr int PUMP_EVENTS = 8;

// Renders the window over `backdrop` and grabs it, in whatever format the
// platform produced. The backdrop is the *clear* colour: the projection's own
// background is drawn by the QML canvas, so swapping it changes nothing the
// panel draws.
QImage grab_over(QQuickWindow *window, int width, int height,
                 const QColor &backdrop) {
  window->setProperty("hostBackdrop", backdrop);
  window->resize(width, height);
  window->requestUpdate();

  // The software backend renders in step with the event loop, so the frame that
  // is grabbed is the one after the layout and the painting above.
  for (int i = 0; i < PUMP_EVENTS; ++i) {
    QCoreApplication::processEvents();
  }

  return window->grabWindow();
}

// The frame's bytes, or a readable reason there are none.
//
// `image` is the frame `render()` decided to hand back; it must describe the
// size the surface was configured to — a frame of another size would write
// pixels for one surface into the memory declared for another.
QByteArray frame_bytes(const QImage &image, int width, int height,
                       QString *error) {
  if (image.isNull()) {
    *error = QStringLiteral("the QML window produced no image to grab");
    return {};
  }

  if (image.width() != width || image.height() != height) {
    *error = QStringLiteral("the QML window rendered %1x%2 for a %3x%4 surface")
                 .arg(image.width())
                 .arg(image.height())
                 .arg(width)
                 .arg(height);
    return {};
  }

  const QImage rgba =
      image.convertToFormat(QImage::Format_RGBA8888_Premultiplied);

  if (rgba.isNull()) {
    *error = QStringLiteral("the QML window's frame could not be read back as "
                            "premultiplied RGBA8");
    return {};
  }

  error->clear();

  return QByteArray(reinterpret_cast<const char *>(rgba.constBits()),
                    rgba.sizeInBytes());
}

// The alpha a grab could not carry, recovered from two opaque ones.
//
// A frame with premultiplied colour C and alpha a, cleared over a backdrop B and
// read back without an alpha channel, is `C + (1 - a)*B`. So over black (B = 0)
// the grab *is* C, and over white (B = 255) it is `C + (1 - a)*255`: the largest
// per-channel difference between the two is `(1 - a)*255`, which gives the
// alpha, while the black grab gives the premultiplied colour. Together they are
// a real ARGB grab.
//
// Qt rounds each channel of each pass to 8 bits, so a recovered alpha can sit
// one step away from what an ARGB grab would have produced; every colour channel
// is clamped to the alpha it is premultiplied by, so the result is always a
// valid premultiplied pixel. Measured on this crate's dock frame against a real
// ARGB grab: the colour channels match everywhere, and 1.3% of the pixels (the
// anti-aliased edges) differ by one step of alpha. Fully opaque and fully
// transparent pixels are exact, which is what the panel's pixel assertions rest
// on — `headless_tests::qml` runs all of them through both paths.
QImage reconstruct_alpha(const QImage &over_black, const QImage &over_white,
                         QString *error) {
  if (over_black.isNull() || over_white.isNull() ||
      over_black.size() != over_white.size()) {
    *error = QStringLiteral(
        "the offscreen QML window could not be grabbed over a backdrop pair "
        "(%1x%2 and %3x%4), so the panel's alpha could not be recovered")
                 .arg(over_black.width())
                 .arg(over_black.height())
                 .arg(over_white.width())
                 .arg(over_white.height());
    return {};
  }

  const QImage black =
      over_black.convertToFormat(QImage::Format_RGBA8888_Premultiplied);
  const QImage white =
      over_white.convertToFormat(QImage::Format_RGBA8888_Premultiplied);

  if (black.isNull() || white.isNull()) {
    *error = QStringLiteral("the offscreen QML window's frame could not be read "
                            "back as premultiplied RGBA8");
    return {};
  }

  QImage image(black.size(), QImage::Format_RGBA8888_Premultiplied);

  for (int y = 0; y < image.height(); ++y) {
    const uchar *black_line = black.constScanLine(y);
    const uchar *white_line = white.constScanLine(y);
    uchar *out = image.scanLine(y);

    for (int x = 0; x < image.width(); ++x) {
      const int at = x * 4;
      int spread = 0;

      for (int channel = 0; channel < 3; ++channel) {
        const int difference =
            int(white_line[at + channel]) - int(black_line[at + channel]);

        spread = qMax(spread, difference);
      }

      const int alpha = qBound(0, 255 - spread, 255);

      out[at + 0] = uchar(qMin(int(black_line[at + 0]), alpha));
      out[at + 1] = uchar(qMin(int(black_line[at + 1]), alpha));
      out[at + 2] = uchar(qMin(int(black_line[at + 2]), alpha));
      out[at + 3] = uchar(alpha);
    }
  }

  return image;
}

} // namespace

Offscreen::Offscreen() : d(new Impl) {
  (void)ensure_app();

  // Documented as a test hook: it makes this host behave like the Qt builds that
  // cannot hand back an alpha channel, so the reconstruction path runs — and is
  // asserted on — on a Qt that would take the fast one. Nothing in production
  // sets it.
  d->force_reconstruction =
      qEnvironmentVariableIsSet("INOX_PANEL_OFFSCREEN_NO_ALPHA_GRAB");
}

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

  // The first frame decides which pixel path this host is on, and pays for the
  // answer: one grab with the clear colour left transparent. If it has an alpha
  // channel, the platform hands back the real thing and every later frame is a
  // single render. Otherwise this Qt's offscreen backing store cannot carry
  // alpha at all (see the note at the top of this file), and every frame is
  // reconstructed from a black and a white pass.
  if (d->alpha_path < 0) {
    if (!d->force_reconstruction) {
      const QImage probe =
          grab_over(d->window, width, height, QColor(Qt::transparent));

      if (probe.isNull()) {
        d->error = QStringLiteral("the QML window produced no image to grab");
        return {};
      }

      if (probe.hasAlphaChannel()) {
        d->alpha_path = 1;

        return frame_bytes(probe, width, height, &d->error);
      }
    }

    d->alpha_path = 0;
  }

  if (d->alpha_path == 1) {
    const QImage grabbed =
        grab_over(d->window, width, height, QColor(Qt::transparent));

    if (!grabbed.isNull() && !grabbed.hasAlphaChannel()) {
      // The platform promised an alpha channel on the first frame and stopped
      // delivering one. There is no sensible frame to hand back from here: an
      // opaque one would hide the desktop behind the pane.
      d->error = QStringLiteral("the offscreen QML window grabbed a %1x%2 image "
                                "with no alpha channel, where the first frame "
                                "had one")
                     .arg(grabbed.width())
                     .arg(grabbed.height());
      return {};
    }

    return frame_bytes(grabbed, width, height, &d->error);
  }

  // The reconstruction: the same frame over black and over white. The clear
  // colour is the only thing that differs between the two passes — the
  // projection's background is drawn by the QML canvas, not by the clear.
  const QImage over_black =
      grab_over(d->window, width, height, QColor(Qt::black));
  const QImage over_white =
      grab_over(d->window, width, height, QColor(Qt::white));
  const QImage reconstructed =
      reconstruct_alpha(over_black, over_white, &d->error);

  if (reconstructed.isNull()) {
    // `reconstruct_alpha` has said why: one of the two passes produced nothing
    // usable, and an opaque frame is not a fallback this host will take.
    return {};
  }

  return frame_bytes(reconstructed, width, height, &d->error);
}

QString Offscreen::lastError() const { return d->error; }

bool Offscreen::alphaReconstructed() const { return d->alpha_path == 0; }

std::unique_ptr<Offscreen> new_offscreen() { return std::make_unique<Offscreen>(); }
