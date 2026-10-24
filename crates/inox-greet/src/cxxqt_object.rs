//! cxx-qt bridge: the only place QML and Rust meet.
//!
//! `GreetPreview` is a Rust-backed `QObject` the QML module instantiates. It
//! exposes the projection computed in [`crate::view`] as read-only properties —
//! colours already converted to `QColor`, spacing and font size as integers,
//! and every user-visible string as text resolved from Fluent — plus the two
//! pieces of live state the surface renders (the edited account name and the
//! current notice). QML never hard-codes a colour, a size or a string.
//!
//! The reverse direction is deliberately narrow: QML can report an edited name,
//! ask to clear it, ask to activate the focused control, and report that the
//! tree finished loading. None of those can authenticate: [`crate::app::Preview`]
//! owns the state machine and is constructed with the reviewed transport
//! unavailable, so every command it could return is dropped here (ADR 0015).
//!
//! Note: doc comments are not placed on items inside the `#[cxx_qt::bridge]`
//! blocks below; the cxx parser rejects them there. Documentation lives on the
//! Rust types and on the plain-Rust `impl` blocks outside the macro.

use core::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use cxx_qt_lib::{QColor, QString};
use tracing::warn;

use crate::app::{Action, Notice, Preview};
use crate::view::{View, WINDOW_HEIGHT, WINDOW_WIDTH};

/// Set once QML has loaded the module and reported that its tree completed.
pub static QML_READY: AtomicBool = AtomicBool::new(false);

/// Whether QML reported a completed tree.
///
/// This is the self-check's evidence that the Rust-backed element was really
/// instantiated by the engine — not merely that a URL was handed to `load`.
pub fn qml_ready() -> bool {
    QML_READY.load(Ordering::SeqCst)
}

/// Convert a theme colour into the `QColor` QML renders.
///
/// Root AGENTS.md §4 forbids hard-coding colours in the component; ADR 0035
/// §实施执行约束 adds that the conversion happens on the Rust side.
fn color(value: inox_config::Rgba) -> QColor {
    QColor::from_rgba(
        i32::from(value.r),
        i32::from(value.g),
        i32::from(value.b),
        i32::from(value.a),
    )
}

/// The Rust backing struct of the QML `GreetPreview` element.
///
/// Every field named in a `#[qproperty]` below is a property QML can read; the
/// remaining fields are the model behind them.
#[derive(Debug)]
pub struct GreetPreviewRust {
    /// Localised window title.
    pub title: QString,
    /// Localised "preview only" notice.
    pub preview_notice: QString,
    /// Localised label of the account-name field.
    pub user_label: QString,
    /// Localised label of the login button.
    pub login_label: QString,
    /// Localised label of the clear button.
    pub clear_label: QString,
    /// The explanation currently shown below the account field.
    pub notice: QString,
    /// The account name being edited.
    pub username: QString,
    /// Whether the login control is enabled; always false without a reviewed
    /// transport.
    pub can_login: bool,
    /// Window background.
    pub background: QColor,
    /// Colour of raised surfaces such as the account field.
    pub surface: QColor,
    /// Alternate surface colour, for disabled controls.
    pub surface_variant: QColor,
    /// Separator and outline colour.
    pub border: QColor,
    /// Default text colour.
    pub text: QColor,
    /// Secondary text colour.
    pub text_muted: QColor,
    /// Interactive accent colour.
    pub accent: QColor,
    /// Text colour drawn on top of the accent.
    pub accent_text: QColor,
    /// Colour signalling a caution, used by the preview notice.
    pub warning: QColor,
    /// Colour signalling a failure, used by the invalid-name notice.
    pub danger: QColor,
    /// Font size in logical pixels, after the user's font scale.
    pub font_size: i32,
    /// Small spacing token.
    pub spacing_sm: i32,
    /// Medium spacing token.
    pub spacing_md: i32,
    /// Large spacing token.
    pub spacing_lg: i32,
    /// Medium corner-radius token.
    pub radius_md: i32,
    /// Window width in logical pixels.
    pub window_width: i32,
    /// Window height in logical pixels.
    pub window_height: i32,
    preview: Preview,
    view: View,
}

impl GreetPreviewRust {
    /// Project `view` into the property values QML renders.
    pub fn from_view(view: View) -> Self {
        let colors = view.theme.colors;
        Self {
            title: QString::from(&view.strings.title),
            preview_notice: QString::from(&view.strings.preview),
            user_label: QString::from(&view.strings.user),
            login_label: QString::from(&view.strings.login),
            clear_label: QString::from(&view.strings.clear),
            notice: QString::from(&view.strings.blocked),
            username: QString::default(),
            can_login: false,
            background: color(colors.background),
            surface: color(colors.surface),
            surface_variant: color(colors.surface_variant),
            border: color(colors.border),
            text: color(colors.text),
            text_muted: color(colors.text_muted),
            accent: color(colors.accent),
            accent_text: color(colors.on_accent),
            warning: color(colors.warning),
            danger: color(colors.danger),
            font_size: view.font_px(),
            spacing_sm: view.spacing_sm(),
            spacing_md: view.spacing_md(),
            spacing_lg: view.spacing_lg(),
            radius_md: view.radius_md(),
            window_width: WINDOW_WIDTH,
            window_height: WINDOW_HEIGHT,
            preview: Preview::new(),
            view,
        }
    }

    /// The text explaining the current notice.
    fn notice_text(&self) -> &str {
        match self.preview.notice() {
            Notice::Blocked => &self.view.strings.blocked,
            Notice::InvalidUser => &self.view.strings.invalid_user,
        }
    }
}

impl Default for GreetPreviewRust {
    fn default() -> Self {
        Self::from_view(View::installed())
    }
}

/// The QML element and its invokables.
#[cxx_qt::bridge]
pub mod qobject {
    // `type QString` / `type QColor` below are cxx aliases with no Rust-side
    // documentation, and the cxx parser rejects `///` comments inside bridge
    // items, so allow `missing_docs` for the bridge as a whole.
    #[allow(missing_docs)]
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qcolor.h");
        type QColor = cxx_qt_lib::QColor;
    }

    // `auto_cxx_name` makes snake_case Rust items visible to QML in camelCase
    // (`edit_username` becomes `editUsername`); it is only allowed on the block.
    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, title)]
        #[qproperty(QString, preview_notice)]
        #[qproperty(QString, user_label)]
        #[qproperty(QString, login_label)]
        #[qproperty(QString, clear_label)]
        #[qproperty(QString, notice)]
        #[qproperty(QString, username)]
        #[qproperty(bool, can_login)]
        #[qproperty(QColor, background)]
        #[qproperty(QColor, surface)]
        #[qproperty(QColor, surface_variant)]
        #[qproperty(QColor, border)]
        #[qproperty(QColor, text)]
        #[qproperty(QColor, text_muted)]
        #[qproperty(QColor, accent)]
        #[qproperty(QColor, accent_text)]
        #[qproperty(QColor, warning)]
        #[qproperty(QColor, danger)]
        #[qproperty(i32, font_size)]
        #[qproperty(i32, spacing_sm)]
        #[qproperty(i32, spacing_md)]
        #[qproperty(i32, spacing_lg)]
        #[qproperty(i32, radius_md)]
        #[qproperty(i32, window_width)]
        #[qproperty(i32, window_height)]
        type GreetPreview = super::GreetPreviewRust;

        #[qinvokable]
        fn edit_username(self: Pin<&mut Self>, value: QString);

        #[qinvokable]
        fn clear_identity(self: Pin<&mut Self>);

        #[qinvokable]
        fn activate(self: Pin<&mut Self>);

        #[qinvokable]
        fn report_ready(self: Pin<&mut Self>, title: QString, accessible_items: i32);
    }
}

/// Behaviour of the `GreetPreview` QObject.
///
/// Each invokable applies one [`Action`] to the state machine and then refreshes
/// the properties QML binds to. The returned [`crate::state::Command`] is
/// dropped on purpose: the preview has no transport to send it to, and the
/// binary only ships once that boundary has passed security review (ADR 0015).
impl qobject::GreetPreview {
    /// Replace the edited account name with the text QML reports.
    pub fn edit_username(self: Pin<&mut Self>, value: QString) {
        self.apply_action(Action::EditUsername(String::from(&value)));
    }

    /// Clear the identity, as the clear button and Escape do.
    pub fn clear_identity(self: Pin<&mut Self>) {
        self.apply_action(Action::Clear);
    }

    /// Activate the focused control; login stays disabled.
    pub fn activate(self: Pin<&mut Self>) {
        self.apply_action(Action::Activate);
    }

    /// Confirm from QML that the tree completed, that the projected title
    /// arrived intact, and that the semantic tree is populated.
    ///
    /// The readiness flag is only set when all three agree, so the headless
    /// self-check fails when a control loses its `Accessible.role` (root
    /// AGENTS.md §7.4) instead of silently passing on a load that produced no
    /// usable surface.
    pub fn report_ready(self: Pin<&mut Self>, title: QString, accessible_items: i32) {
        let expected = self.title().clone();
        let expected_items = crate::view::ACCESSIBLE_ITEMS;
        if title != expected {
            warn!("the QML tree reported a title that does not match the projection");
            return;
        }
        if accessible_items != expected_items {
            warn!(
                reported = accessible_items,
                expected = expected_items,
                "the QML semantic tree does not match the expected accessible items"
            );
            return;
        }
        QML_READY.store(true, Ordering::SeqCst);
    }

    /// Apply one action and refresh the live properties.
    fn apply_action(mut self: Pin<&mut Self>, action: Action) {
        use cxx_qt::CxxQtType;

        let now = Instant::now();
        let command = self.as_mut().rust_mut().preview.apply(action, now);
        tracing::debug!(?command, "preview action applied; no transport is attached");

        let projected = {
            let this = self.as_ref();
            let rust = this.rust();
            (
                QString::from(rust.preview.username()),
                QString::from(rust.notice_text()),
                rust.preview.can_login(now),
            )
        };
        self.as_mut().set_username(projected.0);
        self.as_mut().set_notice(projected.1);
        self.as_mut().set_can_login(projected.2);
    }
}
