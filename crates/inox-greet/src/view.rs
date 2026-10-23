//! Qt-free projection of the configured theme and localised strings.
//!
//! The QML preview is only allowed to *render*: colours, spacing, radii, font
//! size and every user-visible string are resolved here, in safe Rust, and
//! handed to the `cxx-qt` bridge as plain values. Root AGENTS.md §4 forbids a
//! component from hard-coding colours or sizes, and §7.3 forbids hard-coded
//! user-visible text, so nothing in `qml/main.qml` may contain either: it reads
//! this projection through the bridge instead (ADR 0035 §实施执行约束: colours
//! are turned into `QColor` on the Rust side).
//!
//! Keeping the projection Qt-free means the whole mapping (tokens → numbers,
//! message ids → text) is unit-testable without a display, and the FFI-bearing
//! bridge stays a thin translation layer.

use std::sync::OnceLock;

use inox_common::i18n::Localizer;
use inox_config::Theme;

/// Width of the preview window, in logical pixels.
///
/// Window geometry is a layout decision of this component rather than a theme
/// token, so it lives in Rust (never in QML) and the QML tree binds to it.
pub const WINDOW_WIDTH: i32 = 480;

/// Height of the preview window, in logical pixels.
pub const WINDOW_HEIGHT: i32 = 360;

/// Font size the tokens' `typography.scale` multiplies, in logical pixels.
const BASE_FONT_PX: f32 = 16.0;

/// The projection installed by the binary before QML is loaded.
static INSTALLED: OnceLock<View> = OnceLock::new();

/// The preview's user-visible strings, resolved from the Fluent catalogs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Strings {
    /// Window title.
    pub title: String,
    /// The "preview only, no credentials" notice.
    pub preview: String,
    /// Label of the account-name field.
    pub user: String,
    /// Label of the (disabled) login button.
    pub login: String,
    /// Label of the button that clears the identity.
    pub clear: String,
    /// Why login is unavailable.
    pub blocked: String,
    /// Why the typed account name is not acceptable.
    pub invalid_user: String,
}

impl Strings {
    /// Resolve every string this component shows.
    pub fn localize(localizer: &Localizer) -> Self {
        Self {
            title: localizer.message("title"),
            preview: localizer.message("preview"),
            user: localizer.message("user"),
            login: localizer.message("login"),
            clear: localizer.message("cancel"),
            blocked: localizer.message("blocked"),
            invalid_user: localizer.message("invalid-user"),
        }
    }
}

/// Everything the QML tree renders, resolved from configuration and catalogs.
#[derive(Debug, Clone)]
pub struct View {
    /// Design tokens of the active theme.
    pub theme: Theme,
    /// Localised strings.
    pub strings: Strings,
}

impl View {
    /// Project `theme` and the strings of `localizer` for the QML preview.
    pub fn new(theme: Theme, localizer: &Localizer) -> Self {
        Self {
            theme,
            strings: Strings::localize(localizer),
        }
    }

    /// Make `view` the projection the QML bridge reads.
    ///
    /// Returns `false` when a projection was already installed: the binary
    /// installs exactly one, and a second call is a programming error rather
    /// than something to overwrite silently.
    pub fn install(view: Self) -> bool {
        INSTALLED.set(view).is_ok()
    }

    /// The installed projection, or the built-in fallback.
    ///
    /// The bridge object is constructed by the QML engine, which can happen
    /// before (or without) the binary installing a projection — for example in
    /// a unit test that instantiates the Rust backing struct directly. Falling
    /// back to the dark theme keeps that path total instead of panicking in a
    /// constructor, which would abort the process.
    pub fn installed() -> Self {
        INSTALLED.get().cloned().unwrap_or_else(Self::fallback)
    }

    /// The projection used when the binary has not installed one.
    pub fn fallback() -> Self {
        Self {
            theme: Theme::builtin_dark(),
            strings: Strings::default(),
        }
    }

    /// Font size in logical pixels, after the user's font scale.
    ///
    /// A nonsensical configured scale must not make text disappear: the result
    /// saturates (as Rust's float-to-int casts do) and is floored at one pixel.
    pub fn font_px(&self) -> i32 {
        let scaled = BASE_FONT_PX * self.theme.typography.scale;
        if scaled.is_finite() {
            (scaled.round() as i32).max(1)
        } else {
            BASE_FONT_PX as i32
        }
    }

    /// The small spacing token, in logical pixels.
    pub fn spacing_sm(&self) -> i32 {
        i32::from(self.theme.spacing.sm)
    }

    /// The medium spacing token, in logical pixels.
    pub fn spacing_md(&self) -> i32 {
        i32::from(self.theme.spacing.md)
    }

    /// The large spacing token, in logical pixels.
    pub fn spacing_lg(&self) -> i32 {
        i32::from(self.theme.spacing.lg)
    }

    /// The medium corner-radius token, in logical pixels.
    pub fn radius_md(&self) -> i32 {
        i32::from(self.theme.radii.md)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_common::i18n::Locale;
    use inox_config::Appearance;

    fn localizer(tag: &str) -> Localizer {
        let locale = Locale::parse(tag).expect("locale");
        crate::i18n::load(locale).expect("catalog")
    }

    #[test]
    fn strings_follow_the_negotiated_language() {
        let en = View::new(Theme::builtin_dark(), &localizer("en-US"));
        let zh = View::new(Theme::builtin_dark(), &localizer("zh-CN"));
        assert_eq!(en.strings.title, "Inox login preview");
        assert_eq!(zh.strings.title, "Inox 登录预览");
        assert_eq!(en.strings.clear, "Clear");
        assert_eq!(zh.strings.clear, "清空");
        for value in [
            &en.strings.title,
            &en.strings.preview,
            &en.strings.user,
            &en.strings.login,
            &en.strings.clear,
            &en.strings.blocked,
            &en.strings.invalid_user,
        ] {
            assert!(!value.is_empty(), "a message id leaked instead of text");
        }
    }

    #[test]
    fn tokens_are_projected_with_the_users_font_scale() {
        let mut theme = Theme::builtin_dark();
        theme.typography.scale = 1.0;
        let view = View::new(theme.clone(), &localizer("en-US"));
        assert_eq!(view.font_px(), 16);
        assert_eq!(view.spacing_sm(), i32::from(theme.spacing.sm));
        assert_eq!(view.spacing_md(), i32::from(theme.spacing.md));
        assert_eq!(view.spacing_lg(), i32::from(theme.spacing.lg));
        assert_eq!(view.radius_md(), i32::from(theme.radii.md));

        theme.typography.scale = 2.0;
        let doubled = View::new(theme, &localizer("en-US"));
        assert_eq!(doubled.font_px(), 32);
    }

    #[test]
    fn a_nonsensical_font_scale_still_yields_visible_text() {
        let mut theme = Theme::builtin_dark();
        for scale in [f32::NAN, f32::INFINITY, -4.0, 0.0] {
            theme.typography.scale = scale;
            let view = View::new(theme.clone(), &localizer("en-US"));
            assert!(
                view.font_px() >= 1,
                "scale {scale} produced {}",
                view.font_px()
            );
        }
    }

    #[test]
    fn the_light_theme_keeps_its_own_palette() {
        let dark = View::new(Theme::builtin_dark(), &localizer("en-US"));
        let light = View::new(Theme::builtin_light(), &localizer("en-US"));
        assert_ne!(dark.theme.colors.background, light.theme.colors.background);
        assert_eq!(
            light.theme.appearance,
            Appearance::Light,
            "the light theme must stay light"
        );
    }

    #[test]
    fn the_fallback_projection_is_usable_without_configuration() {
        let fallback = View::fallback();
        assert_eq!(fallback.theme.name, inox_config::BUILTIN_DARK);
        assert_eq!(fallback.font_px(), 16);
        assert!(fallback.strings.title.is_empty());
    }
}
