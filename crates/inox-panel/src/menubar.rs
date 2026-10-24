//! The menu bar: the panel drawn as a full-width bar along the top edge.
//!
//! **Inox has no application-menu protocol.** A Wayland client cannot publish
//! the "File / Edit / View" menus a macOS menu bar draws, so this bar does not
//! invent any: it shows the focused application's name at its left end (when
//! `[menubar] show_app_name` asks for it) and nothing that pretends to be a
//! menu. Inventing titles would be a claim about a protocol that does not
//! exist, and the panel only draws what it actually knows (ADR 0013).
//!
//! What it does draw is the same data the taskbar draws, arranged for a bar:
//!
//! ```text
//! ┌ highlight ────────────────────────────────────────────────────────────┐
//! │ app name   notice + retry      (flexible gap)      tray items   clock │
//! └───────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! This module is the *pure* half — the geometry and the projection. The
//! widgets are in [`crate::view::menubar`], and this module has no iced widget
//! in it at all, which is what lets its tests run without a renderer.
//!
//! Three decisions are worth naming:
//!
//! - **The bar never takes keyboard focus.** It is a display surface: the retry
//!   action is a click target like the taskbar's, and the placement asks the
//!   compositor for [`crate::placement::KeyboardInteractivity::None`] (D01).
//! - **The two groups are separated by a gap wider than the entries inside
//!   them** (ui-guidelines §3): the app name and the notice sit together on the
//!   left, the tray and the clock on the right, and the flexible space between
//!   them is at least one `spacing.xl` step.
//! - **The clock and the tray are not the bar's own state.** They appear or not
//!   exactly as `[menubar] show_clock` and `show_tray` say; the panel's notice
//!   is always shown when the projection produced one, because a service that
//!   is not available is not something a user should have to switch on (D05).

use iced::widget::tooltip;
use inox_common::i18n::Localizer;
use inox_config::{MenubarConfig, Theme as Tokens};

use crate::clock::ClockLabel;
use crate::theme;
use crate::view_model::{PanelViewModel, TrayItemView, app_label};

/// How many extra-large spacing steps the notice sentence may be wide.
///
/// The sentence about the panel's state is the most important thing on the bar
/// after the application's name, so it is allowed to be wider than a single
/// entry — but it is still bounded, so it can never push the clock off the end
/// of a 1280-wide bar.
const NOTICE_WIDTH_STEPS: f32 = 8.0;

/// The geometry of the menu bar, derived from the tokens and the configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct MenubarGeometry {
    /// Height of the bar, in logical pixels.
    pub height: f32,
    /// Padding at the left and right end of the bar.
    pub padding: f32,
    /// Distance between the bar and the top edge of the output.
    pub margin: f32,
    /// Vertical inset of the bar's content.
    pub inset: f32,
    /// Thickness of the highlight hairline along the top edge.
    pub highlight_height: f32,
    /// Gap between two entries of the same group.
    pub entry_spacing: f32,
    /// Gap between the left group and the right group: wider than the entries
    /// inside either of them (ui-guidelines §3).
    pub group_gap: f32,
    /// Text size of the application's name: the title role.
    pub font_size: f32,
    /// Text size of the tray items and the clock: secondary information.
    pub caption_size: f32,
    /// Widest the notice sentence may be before the layout clips it.
    pub label_max_width: f32,
    /// Which way a tooltip opens. The bar sits on the top edge, so its tooltips
    /// open downwards.
    pub tooltip_position: tooltip::Position,
}

impl MenubarGeometry {
    /// Derives the bar's geometry.
    pub fn new(tokens: &Tokens, config: &MenubarConfig) -> Self {
        let height = f32::from(config.height);
        let inset = f32::from(tokens.spacing.xs);
        let highlight_height = theme::hairline(tokens);
        // Text that is taller than the bar's content area would overflow it, so
        // the scaled sizes are clamped to what is left after the inset and the
        // highlight hairline along the top edge.
        let available = (height - inset * 2.0 - highlight_height).max(1.0);

        Self {
            height,
            padding: f32::from(tokens.spacing.md),
            margin: f32::from(config.margin),
            inset,
            highlight_height,
            entry_spacing: f32::from(tokens.spacing.sm),
            group_gap: f32::from(tokens.spacing.xl),
            font_size: theme::title_size(tokens, config.height).min(available),
            caption_size: theme::caption_size(tokens, config.height).min(available),
            label_max_width: f32::from(tokens.spacing.xl) * NOTICE_WIDTH_STEPS,
            tooltip_position: tooltip::Position::Bottom,
        }
    }

    /// The height the bar's content has, below the highlight hairline and
    /// inside the vertical inset.
    pub fn content_height(&self) -> f32 {
        (self.height - self.inset * 2.0 - self.highlight_height).max(1.0)
    }
}

/// What the bar draws, derived from what the panel knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenubarView {
    /// The focused application's name, when there is one and the configuration
    /// asks for it. `None` is the ordinary state of a session with no focused
    /// window, and the bar then simply starts with its notice or with the clock.
    pub app_name: Option<String>,
    /// The sentence about the panel's own state, when there is one (D05).
    pub notice: Option<String>,
    /// The label of the retry action, when the notice offers one.
    pub retry: Option<String>,
    /// The status notifier items, empty when `show_tray` is off.
    pub tray: Vec<TrayItemView>,
    /// The clock, when the configuration shows one and it could be rendered.
    pub clock: Option<ClockLabel>,
}

impl MenubarView {
    /// Projects the shared panel model onto the bar's own arrangement.
    ///
    /// The task list, the tray and the clock are the same data the taskbar
    /// draws; what changes is what the bar keeps. `focused` is the application
    /// the compositor reports as focused, which is exactly the task button the
    /// projection marked active.
    pub fn project(model: &PanelViewModel, config: &MenubarConfig, localizer: &Localizer) -> Self {
        let app_name = if config.show_app_name {
            model
                .tasks
                .iter()
                .find(|task| task.active)
                .map(|task| app_label(&task.app_id, localizer))
        } else {
            None
        };

        Self {
            app_name,
            notice: model.notice.clone(),
            retry: model.retry.clone(),
            tray: if config.show_tray {
                model.tray.clone()
            } else {
                Vec::new()
            },
            clock: if config.show_clock {
                model.clock.clone()
            } else {
                None
            },
        }
    }

    /// Whether the bar has anything to draw at all.
    pub fn is_empty(&self) -> bool {
        self.app_name.is_none()
            && self.notice.is_none()
            && self.tray.is_empty()
            && self.clock.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_common::i18n::Locale;

    use crate::i18n;
    use crate::tasks::{SessionGeneration, WindowId, WindowKey};
    use crate::view_model::{PanelStatus, TaskButton};

    fn localizer(tag: &str) -> Localizer {
        let locale = Locale::parse(tag).expect("a well-formed tag");

        i18n::load(&locale).expect("the catalogs parse")
    }

    fn english() -> Localizer {
        localizer("en-US")
    }

    fn key(window: u64) -> WindowKey {
        WindowKey::new(WindowId::new(window), SessionGeneration::new(1))
    }

    fn task(app_id: &str, active: bool) -> TaskButton {
        TaskButton {
            key: key(1),
            app_id: app_id.to_owned(),
            windows: 1,
            label: "A window".to_owned(),
            active,
            minimized: false,
            enabled: true,
            disabled_reason: None,
            tooltip: "app: A window".to_owned(),
        }
    }

    fn model(tasks: Vec<TaskButton>, notice: Option<&str>) -> PanelViewModel {
        PanelViewModel {
            status: if notice.is_some() {
                PanelStatus::Starting
            } else {
                PanelStatus::Ready
            },
            tasks,
            tray: Vec::new(),
            clock: Some(ClockLabel {
                text: "09:05".to_owned(),
                tooltip: "Friday".to_owned(),
            }),
            notice: notice.map(str::to_owned),
            retry: None,
            focus_order: Vec::new(),
            focused: None,
        }
    }

    fn geometry(config: &MenubarConfig) -> MenubarGeometry {
        MenubarGeometry::new(&Tokens::builtin_dark(), config)
    }

    #[test]
    fn the_bar_is_as_tall_as_the_configuration_says() {
        let tall = geometry(&MenubarConfig {
            height: 40,
            ..MenubarConfig::default()
        });

        assert!((tall.height - 40.0).abs() < f32::EPSILON);
        assert_eq!(
            geometry(&MenubarConfig::default()).height,
            f32::from(MenubarConfig::default().height),
            "the default is the 28 logical pixels the schema documents"
        );
    }

    #[test]
    fn the_highlight_is_a_hairline_from_the_tokens() {
        let tokens = Tokens::builtin_dark();

        assert!(
            (geometry(&MenubarConfig::default()).highlight_height - theme::hairline(&tokens)).abs()
                < f32::EPSILON
        );
        assert!(geometry(&MenubarConfig::default()).highlight_height > 0.0);
    }

    #[test]
    fn the_groups_are_further_apart_than_the_entries_inside_them() {
        let geometry = geometry(&MenubarConfig::default());

        assert!(
            geometry.group_gap > geometry.entry_spacing,
            "grouping is what tells a user what belongs together (ui-guidelines §3)"
        );
        assert!(geometry.entry_spacing > 0.0);
    }

    #[test]
    fn the_name_is_the_title_role_and_secondary_information_is_smaller() {
        let geometry = geometry(&MenubarConfig::default());

        assert!(geometry.caption_size < geometry.font_size);
        assert!(geometry.font_size <= geometry.content_height());
        assert!(geometry.caption_size <= geometry.content_height());
    }

    #[test]
    fn scaled_text_never_grows_past_the_bar() {
        let mut tokens = Tokens::builtin_dark();
        tokens.typography.scale = 4.0;
        let config = MenubarConfig {
            height: 16,
            ..MenubarConfig::default()
        };

        let geometry = MenubarGeometry::new(&tokens, &config);

        assert!(geometry.font_size <= geometry.content_height());
        assert!(geometry.caption_size <= geometry.content_height());
    }

    #[test]
    fn the_tooltips_open_away_from_the_edge_the_bar_sits_on() {
        assert_eq!(
            geometry(&MenubarConfig::default()).tooltip_position,
            tooltip::Position::Bottom,
            "the bar is on the top edge, so its tooltips open downwards"
        );
    }

    #[test]
    fn a_focused_application_names_the_bar() {
        let view = MenubarView::project(
            &model(vec![task("firefox", false), task("terminal", true)], None),
            &MenubarConfig::default(),
            &english(),
        );

        assert_eq!(view.app_name.as_deref(), Some("terminal"));
        assert!(!view.is_empty());
    }

    #[test]
    fn no_focused_application_means_no_name() {
        let view = MenubarView::project(
            &model(vec![task("firefox", false)], None),
            &MenubarConfig::default(),
            &english(),
        );

        assert_eq!(view.app_name, None);
    }

    #[test]
    fn the_configuration_can_switch_the_name_off() {
        let view = MenubarView::project(
            &model(vec![task("firefox", true)], None),
            &MenubarConfig {
                show_app_name: false,
                ..MenubarConfig::default()
            },
            &english(),
        );

        assert_eq!(view.app_name, None);
    }

    #[test]
    fn an_application_without_a_name_is_labelled_as_unknown() {
        let view = MenubarView::project(
            &model(vec![task("", true)], None),
            &MenubarConfig::default(),
            &english(),
        );

        assert_eq!(view.app_name.as_deref(), Some("Unknown application"));

        let chinese = MenubarView::project(
            &model(vec![task("", true)], None),
            &MenubarConfig::default(),
            &localizer("zh-CN"),
        );

        assert_eq!(chinese.app_name.as_deref(), Some("未知应用"));
    }

    #[test]
    fn the_notice_and_its_retry_come_from_the_shared_projection() {
        let mut model = model(Vec::new(), Some("The window service is not available"));
        model.retry = Some("Retry".to_owned());

        let view = MenubarView::project(&model, &MenubarConfig::default(), &english());

        assert_eq!(
            view.notice.as_deref(),
            Some("The window service is not available")
        );
        assert_eq!(view.retry.as_deref(), Some("Retry"));
        assert!(!view.is_empty());
    }

    #[test]
    fn the_clock_and_the_tray_follow_the_bars_own_configuration() {
        let mut model = model(vec![task("firefox", true)], None);
        model.tray = vec![TrayItemView {
            id: crate::tray::TrayItemId::new(":1.1", "/StatusNotifierItem"),
            label: "Mail".to_owned(),
            status: "active".to_owned(),
            attention: false,
            tooltip: "Mail (active)".to_owned(),
        }];

        let showing = MenubarView::project(&model, &MenubarConfig::default(), &english());
        assert_eq!(showing.tray.len(), 1);
        assert!(showing.clock.is_some());

        let hidden = MenubarView::project(
            &model,
            &MenubarConfig {
                show_tray: false,
                show_clock: false,
                ..MenubarConfig::default()
            },
            &english(),
        );
        assert!(hidden.tray.is_empty());
        assert_eq!(hidden.clock, None);
    }
}
