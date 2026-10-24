//! Design tokens to Iced, and the style closures built from them.
//!
//! Every colour, distance, radius and interaction state the launcher draws with
//! is built here from [`inox_config::Theme`] (root AGENTS.md §4, PRD §9.2): no
//! view and no style function in this crate contains a colour or a pixel size.
//! ADR 0018 explains why this small mapping is duplicated in each L3 component
//! instead of being shared — a shared UI crate would be a new same-layer
//! dependency, which needs an ADR of its own.
//!
//! Three things are derived here rather than read from a token, because the
//! token set has no field for them and the design guidelines' §1 says so:
//!
//! | derived | from | used by |
//! |---------|------|---------|
//! | the four text sizes | [`TextRole`] × the base text size | every string |
//! | the hairline | `spacing.xs / 2`, never below one pixel | outlines |
//! | the focus ring | `spacing.xs` | the search field |
//!
//! The base text size is not a token either: the theme carries a *scale*, and
//! what it scales is Iced's own default text size, so a theme asking for 200 %
//! text gets 200 % of what an unscaled Iced window would draw without this file
//! naming a pixel.
//!
//! Every interactive style here draws the five states the guidelines' §5 asks
//! for, and each state is pairwise distinguishable from the others — see the
//! tests at the bottom of this file, which compare the styles rather than
//! trusting a comment.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Padding};
use inox_config::Theme;
use inox_config::theme::Rgba;

/// The multiplier of the `title` text role (guidelines §2).
const TITLE_RATIO: f32 = 1.5;
/// The multiplier of the `heading` text role.
const HEADING_RATIO: f32 = 1.25;
/// The multiplier of the `body` text role.
const BODY_RATIO: f32 = 1.0;
/// The multiplier of the `caption` text role.
const CAPTION_RATIO: f32 = 0.875;

/// A hairline is half of the smallest spacing step (guidelines §1).
const HAIRLINE_RATIO: f32 = 0.5;

/// A hairline thinner than one pixel is a border nobody can see, so a theme that
/// zeroes `spacing.xs` still gets an outline rather than none.
const MINIMUM_HAIRLINE: f32 = 1.0;

/// The narrowest a group is ever allowed to get, for an output too small to hold
/// even the window's own padding. A width of zero draws nothing at all.
const MINIMUM_CONTENT_WIDTH: f32 = 1.0;

/// The ratio of an average glyph's advance to the font size.
///
/// A proportional sans font's average advance is about half an em. This is a
/// dimensionless ratio, not a size: the size is [`text_size`], and the ratio
/// only decides how many characters a result row's label has room for.
const AVERAGE_ADVANCE_RATIO: f32 = 0.5;

/// The four text roles of the guidelines' §2.
///
/// The roles are the whole type scale: a surface has at most one `title`, groups
/// and sections are headed with `heading`, text and control labels are `body`,
/// and explanations, units and secondary metadata are `caption`. Emphasis is a
/// weight, not a fifth size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    /// The one thing the surface is about; the launcher draws the search field
    /// with it.
    Title,
    /// A group or section heading.
    Heading,
    /// Body text and control labels.
    Body,
    /// Explanations, units and secondary metadata.
    Caption,
}

impl TextRole {
    /// The multiplier the role applies to the base text size (guidelines §2).
    pub const fn ratio(self) -> f32 {
        match self {
            Self::Title => TITLE_RATIO,
            Self::Heading => HEADING_RATIO,
            Self::Body => BODY_RATIO,
            Self::Caption => CAPTION_RATIO,
        }
    }
}

/// The theme's distances, in the form the widget tree needs them.
///
/// The token table has three granularities of the same numbers — a step
/// ([`Theme::spacing`]), the derived widths (hairline, focus ring) and the
/// paddings the guidelines' §3 and §4 spell out. Building them once per frame
/// keeps every `Padding` field and every `spacing` argument in the view a named
/// value rather than arithmetic (guidelines §1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// `spacing.xs`.
    pub space_xs: f32,
    /// `spacing.sm`.
    pub space_sm: f32,
    /// `spacing.md`.
    pub space_md: f32,
    /// `spacing.lg`.
    pub space_lg: f32,
    /// `spacing.xl`.
    pub space_xl: f32,
    /// `radii.sm`, for controls.
    pub radius_sm: f32,
    /// `radii.md`, for cards, groups and popovers.
    pub radius_md: f32,
    /// `radii.lg`, for windows and full-screen surfaces.
    pub radius_lg: f32,
    /// The border width of an outline: `spacing.xs / 2`, at least one pixel.
    pub hairline: f32,
    /// The border width of the focus ring: `spacing.xs`, at least a hairline.
    pub focus_ring: f32,
}

impl Metrics {
    /// The distances a theme implies.
    pub fn from_tokens(tokens: &Theme) -> Self {
        let hairline = (f32::from(tokens.spacing.xs) * HAIRLINE_RATIO).max(MINIMUM_HAIRLINE);

        Self {
            space_xs: f32::from(tokens.spacing.xs),
            space_sm: f32::from(tokens.spacing.sm),
            space_md: f32::from(tokens.spacing.md),
            space_lg: f32::from(tokens.spacing.lg),
            space_xl: f32::from(tokens.spacing.xl),
            radius_sm: f32::from(tokens.radii.sm),
            radius_md: f32::from(tokens.radii.md),
            radius_lg: f32::from(tokens.radii.lg),
            hairline,
            focus_ring: f32::from(tokens.spacing.xs).max(hairline),
        }
    }

    /// The padding of a control: `spacing.md` at the sides, `spacing.sm` above
    /// and below (guidelines §3).
    pub fn control_padding(&self) -> Padding {
        Padding {
            top: self.space_sm,
            right: self.space_md,
            bottom: self.space_sm,
            left: self.space_md,
        }
    }

    /// The padding inside a group card: `spacing.lg` on every side (§4).
    pub fn card_padding(&self) -> Padding {
        Padding::new(self.space_lg)
    }

    /// The width the groups get inside a surface of `surface` logical pixels.
    ///
    /// The window's own padding is spent on both sides of the groups, which is
    /// what keeps a field from being drawn edge to edge on a screen much wider
    /// than the surface the host maps (§4: a width follows what a control is
    /// for, not how much room there happens to be).
    pub fn content_width(&self, surface: u32) -> f32 {
        (surface as f32 - self.space_xl * 2.0).max(MINIMUM_CONTENT_WIDTH)
    }
}

/// The Iced theme a whole window is drawn with.
///
/// The seven token colours Iced's palette is built from are the ones with a
/// counterpart in the toolkit; the rest (`surface_variant`, `text_muted`,
/// `on_accent`) reach the screen through the style closures below.
pub fn iced_theme(tokens: &Theme) -> iced::Theme {
    iced::Theme::custom(
        tokens.name.clone(),
        iced::theme::Palette {
            background: colour(tokens.colors.background),
            text: colour(tokens.colors.text),
            primary: colour(tokens.colors.accent),
            success: colour(tokens.colors.success),
            danger: colour(tokens.colors.danger),
            warning: colour(tokens.colors.warning),
        },
    )
}

/// A token colour as an Iced colour.
pub fn colour(rgba: Rgba) -> Color {
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, f32::from(rgba.a) / 255.0)
}

/// The text size the `body` role is drawn at.
///
/// The token is the *scale*; what it scales is Iced's own default text size, the
/// one the rest of the toolkit uses. That keeps the multiplication honest — a
/// theme asking for 200 % text gets 200 % of what an unscaled Iced window would
/// draw — without this file naming a pixel size of its own.
pub fn text_size(tokens: &Theme) -> f32 {
    role_size(tokens, TextRole::Body)
}

/// The size one of the four text roles is drawn at.
pub fn role_size(tokens: &Theme, role: TextRole) -> f32 {
    iced::Settings::default().default_text_size.0 * tokens.typography.scale * role.ratio()
}

/// The height one line of a text role takes, as Iced lays it out.
///
/// Iced's own default line height, applied to the role's size rather than a
/// factor written here: a surface that has to be sized before any widget exists
/// (see [`crate::placement`]) measures its text the same way the renderer will.
pub fn line_height(tokens: &Theme, role: TextRole) -> f32 {
    iced::advanced::text::LineHeight::default()
        .to_absolute(role_size(tokens, role).into())
        .0
}

/// The width one character of the theme's text takes, on average.
pub fn average_advance(tokens: &Theme) -> f32 {
    (text_size(tokens) * AVERAGE_ADVANCE_RATIO).max(1.0)
}

/// The window's own background.
///
/// The guideline's surface order is `background` (the window) → `surface`
/// (cards, panels, popovers) → `surface_variant` (hover, selection, secondary
/// blocks); the launcher's groups are the `surface` cards on top of this.
pub fn window(tokens: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(colour(tokens.colors.background).into()),
        text_color: Some(colour(tokens.colors.text)),
        ..Default::default()
    }
}

/// A group card: the `surface` token, a hairline border, the `md` radius.
pub fn card(tokens: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    let metrics = Metrics::from_tokens(tokens);

    move |_| container::Style {
        background: Some(colour(tokens.colors.surface).into()),
        border: Border {
            color: colour(tokens.colors.border),
            width: metrics.hairline,
            radius: metrics.radius_md.into(),
        },
        text_color: Some(colour(tokens.colors.text)),
        ..Default::default()
    }
}

/// The search field.
///
/// Resting it is a `surface` control with a hairline outline, hovered it takes
/// the alternate surface, and focused it wears the `accent` focus ring at
/// `spacing.xs` — which is the one focus indication on the launcher's screen,
/// because the caret lives here (guidelines §5).
pub fn field(
    tokens: &Theme,
) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style + '_ {
    let metrics = Metrics::from_tokens(tokens);

    move |_, status| {
        let (background, outline, width, value) = match status {
            text_input::Status::Hovered => (
                tokens.colors.surface_variant,
                tokens.colors.border,
                metrics.hairline,
                tokens.colors.text,
            ),
            text_input::Status::Focused { .. } => (
                tokens.colors.surface,
                tokens.colors.accent,
                metrics.focus_ring,
                tokens.colors.text,
            ),
            text_input::Status::Disabled => (
                tokens.colors.background,
                tokens.colors.border,
                metrics.hairline,
                tokens.colors.text_muted,
            ),
            text_input::Status::Active => (
                tokens.colors.surface,
                tokens.colors.border,
                metrics.hairline,
                tokens.colors.text,
            ),
        };

        text_input::Style {
            background: colour(background).into(),
            border: Border {
                color: colour(outline),
                width,
                radius: metrics.radius_sm.into(),
            },
            icon: colour(tokens.colors.text_muted),
            placeholder: colour(tokens.colors.text_muted),
            value: colour(value),
            selection: colour(tokens.colors.accent),
        }
    }
}

/// The icon column's backdrop.
///
/// The icon theme is not loaded yet, so a row whose entry names an icon is drawn
/// with the letter of its name on the accent, and a row whose entry names none is
/// drawn muted with the placeholder. Keeping the two apart is what makes a
/// missing icon visible rather than silent (acceptance D05).
pub fn icon_background(tokens: &Theme, placeholder: bool) -> Background {
    colour(badge_tint(tokens, placeholder)).into()
}

/// The colour the icon column's glyph is drawn in.
pub fn badge_glyph_colour(tokens: &Theme, placeholder: bool) -> Color {
    colour(if placeholder {
        tokens.colors.text_muted
    } else {
        tokens.colors.on_accent
    })
}

/// The icon column as a square badge: a control-radius tile, never wider than
/// the icon it stands in for.
pub fn badge(tokens: &Theme, placeholder: bool) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    let metrics = Metrics::from_tokens(tokens);
    let background = icon_background(tokens, placeholder);
    let glyph = badge_glyph_colour(tokens, placeholder);

    move |_| container::Style {
        background: Some(background),
        border: Border {
            radius: metrics.radius_sm.into(),
            ..Border::default()
        },
        text_color: Some(glyph),
        ..Default::default()
    }
}

/// A result row.
///
/// The five states of the guidelines' §5, pairwise distinguishable:
///
/// | state | drawn as |
/// |-------|----------|
/// | resting | no background, `text` |
/// | hovered | `surface_variant` background |
/// | pressed | `accent` background, `on_accent` text |
/// | selected | `accent` background, `on_accent` text — the background *is* the selection |
/// | disabled | no background, `text_muted`, with the reason next to it |
///
/// A row that cannot be launched is muted rather than coloured as an error,
/// because it is not one. A row that is both selected and unusable keeps the
/// muted text and no background: the selection can be moved onto it (so that
/// Enter can explain why it will not run), but it must not look launchable.
pub fn row(
    tokens: &Theme,
    selected: bool,
    enabled: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    let metrics = Metrics::from_tokens(tokens);

    move |_, status| {
        let (background, text) = match (selected, enabled, status) {
            (_, false, _) => (None, tokens.colors.text_muted),
            (true, true, _) => (Some(tokens.colors.accent), tokens.colors.on_accent),
            (false, true, button::Status::Pressed) => {
                (Some(tokens.colors.accent), tokens.colors.on_accent)
            }
            (false, true, button::Status::Hovered) => {
                (Some(tokens.colors.surface_variant), tokens.colors.text)
            }
            (false, true, button::Status::Active | button::Status::Disabled) => {
                (None, tokens.colors.text)
            }
        };

        button::Style {
            background: background.map(|tint| colour(tint).into()),
            text_color: colour(text),
            border: Border {
                radius: metrics.radius_sm.into(),
                ..Border::default()
            },
            ..Default::default()
        }
    }
}

/// The colour a row's own text is drawn in.
///
/// The button style carries a text colour, but a widget tree does not inherit
/// it: the labels inside the row are told explicitly, so a selected row's name
/// stays readable on the accent instead of keeping the window's text colour.
pub fn row_text_colour(tokens: &Theme, selected: bool, enabled: bool) -> Color {
    colour(if !enabled {
        tokens.colors.text_muted
    } else if selected {
        tokens.colors.on_accent
    } else {
        tokens.colors.text
    })
}

/// The colour a row's secondary line (its generic name) is drawn in.
///
/// Muted on a resting row, `on_accent` on a selected one: `text_muted` is not
/// readable on the accent, and the size difference already marks it as secondary.
pub fn row_detail_colour(tokens: &Theme, selected: bool, enabled: bool) -> Color {
    colour(if !enabled {
        tokens.colors.text_muted
    } else if selected {
        tokens.colors.on_accent
    } else {
        tokens.colors.text_muted
    })
}

/// A secondary action, such as a retry or a clear.
///
/// The five states of §5 verbatim: resting it is an outline, hovered it takes
/// the alternate surface, pressed it becomes the accent, and a disabled one is
/// muted and carries the reason it is disabled next to it.
pub fn action(
    tokens: &Theme,
    enabled: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    let metrics = Metrics::from_tokens(tokens);

    move |_, status| {
        let (background, text, outline) = match (enabled, status) {
            (false, _) | (true, button::Status::Disabled) => {
                (None, tokens.colors.text_muted, tokens.colors.border)
            }
            (true, button::Status::Pressed) => (
                Some(tokens.colors.accent),
                tokens.colors.on_accent,
                tokens.colors.accent,
            ),
            (true, button::Status::Hovered) => (
                Some(tokens.colors.surface_variant),
                tokens.colors.text,
                tokens.colors.border,
            ),
            (true, button::Status::Active) => (None, tokens.colors.text, tokens.colors.border),
        };

        button::Style {
            background: background.map(|tint| colour(tint).into()),
            text_color: colour(text),
            border: Border {
                color: colour(outline),
                width: metrics.hairline,
                radius: metrics.radius_sm.into(),
            },
            ..Default::default()
        }
    }
}

/// The tint behind the icon column.
fn badge_tint(tokens: &Theme, placeholder: bool) -> Rgba {
    if placeholder {
        tokens.colors.surface_variant
    } else {
        tokens.colors.accent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::Theme as Tokens;

    fn tokens() -> Tokens {
        Tokens::builtin_dark()
    }

    /// How one state of a button is drawn, as one comparable value.
    fn button_look(style: button::Style) -> (Option<Background>, Color, Border) {
        (style.background, style.text_color, style.border)
    }

    /// How one state of the search field is drawn, in the same shape.
    ///
    /// The second slot carries the outline colour rather than a text colour,
    /// because that is the visible difference between the field's states; the
    /// comparison only ever asks whether two states look the same.
    fn field_look(style: text_input::Style) -> (Option<Background>, Color, Border) {
        (Some(style.background), style.border.color, style.border)
    }

    /// Whether no two of `states` are drawn the same way.
    fn all_distinct<T: PartialEq + std::fmt::Debug>(states: &[T]) -> bool {
        states
            .iter()
            .enumerate()
            .all(|(index, state)| states.iter().skip(index + 1).all(|other| state != other))
    }

    #[test]
    fn the_theme_carries_the_named_palette_tokens() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);

        assert_eq!(theme.palette().background, colour(tokens.colors.background));
        assert_eq!(theme.palette().text, colour(tokens.colors.text));
        assert_eq!(theme.palette().primary, colour(tokens.colors.accent));
        assert_eq!(theme.palette().success, colour(tokens.colors.success));
        assert_eq!(theme.palette().danger, colour(tokens.colors.danger));
        assert_eq!(theme.palette().warning, colour(tokens.colors.warning));
    }

    #[test]
    fn the_two_builtin_themes_produce_different_palettes() {
        let dark = iced_theme(&Tokens::builtin_dark());
        let light = iced_theme(&Tokens::builtin_light());
        assert_ne!(dark.palette().background, light.palette().background);
        assert_ne!(dark.palette().text, light.palette().text);
    }

    #[test]
    fn a_token_colour_keeps_its_channels_and_its_alpha() {
        let rgba = tokens().colors.accent;
        let colour = colour(rgba);
        assert_eq!(
            colour,
            Color::from_rgba8(rgba.r, rgba.g, rgba.b, f32::from(rgba.a) / 255.0)
        );
    }

    #[test]
    fn the_four_text_roles_follow_the_guideline_ratios() {
        let tokens = tokens();
        let base = text_size(&tokens);

        assert_eq!(role_size(&tokens, TextRole::Body), base);
        assert_eq!(role_size(&tokens, TextRole::Title), base * TITLE_RATIO);
        assert_eq!(role_size(&tokens, TextRole::Heading), base * HEADING_RATIO);
        assert_eq!(role_size(&tokens, TextRole::Caption), base * CAPTION_RATIO);

        let title = role_size(&tokens, TextRole::Title);
        let heading = role_size(&tokens, TextRole::Heading);
        let caption = role_size(&tokens, TextRole::Caption);
        assert!(title > heading && heading > base && base > caption);
        assert!(
            caption >= base * 0.75,
            "a caption may not be smaller than three quarters of the body size",
        );

        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;
        for role in [
            TextRole::Title,
            TextRole::Heading,
            TextRole::Body,
            TextRole::Caption,
        ] {
            assert_eq!(
                role_size(&doubled, role),
                role_size(&tokens, role) * 2.0,
                "every role follows the font scale, so 200 % text stays uncropped",
            );
        }
    }

    #[test]
    fn the_metrics_are_derived_from_the_tokens() {
        let tokens = tokens();
        let metrics = Metrics::from_tokens(&tokens);

        assert_eq!(metrics.space_xs, f32::from(tokens.spacing.xs));
        assert_eq!(metrics.space_sm, f32::from(tokens.spacing.sm));
        assert_eq!(metrics.space_md, f32::from(tokens.spacing.md));
        assert_eq!(metrics.space_lg, f32::from(tokens.spacing.lg));
        assert_eq!(metrics.space_xl, f32::from(tokens.spacing.xl));
        assert_eq!(metrics.radius_sm, f32::from(tokens.radii.sm));
        assert_eq!(metrics.radius_md, f32::from(tokens.radii.md));
        assert_eq!(metrics.radius_lg, f32::from(tokens.radii.lg));
        assert_eq!(metrics.hairline, f32::from(tokens.spacing.xs) / 2.0);
        assert_eq!(metrics.focus_ring, f32::from(tokens.spacing.xs));
    }

    #[test]
    fn a_theme_that_zeroes_its_smallest_step_still_draws_an_outline() {
        let mut tokens = tokens();
        tokens.spacing.xs = 0;
        let metrics = Metrics::from_tokens(&tokens);

        assert_eq!(metrics.hairline, MINIMUM_HAIRLINE);
        assert!(metrics.focus_ring >= metrics.hairline);
    }

    #[test]
    fn the_paddings_follow_the_spacing_rhythm() {
        let tokens = tokens();
        let metrics = Metrics::from_tokens(&tokens);
        let control = metrics.control_padding();
        let card = metrics.card_padding();

        assert_eq!(control.top, metrics.space_sm);
        assert_eq!(control.bottom, metrics.space_sm);
        assert_eq!(control.left, metrics.space_md);
        assert_eq!(control.right, metrics.space_md);
        assert_eq!(card.top, metrics.space_lg);
        assert_eq!(card.right, metrics.space_lg);
        assert_eq!(card.bottom, metrics.space_lg);
        assert_eq!(card.left, metrics.space_lg);
    }

    #[test]
    fn the_content_width_spends_the_window_padding_on_both_sides() {
        let tokens = tokens();
        let metrics = Metrics::from_tokens(&tokens);
        let surface = 512;

        assert_eq!(
            metrics.content_width(surface),
            surface as f32 - metrics.space_xl * 2.0,
            "the cards get what is left of the surface after its own padding",
        );
        assert_eq!(
            metrics.content_width(0),
            MINIMUM_CONTENT_WIDTH,
            "a column of zero width draws nothing at all",
        );
    }

    #[test]
    fn the_window_and_the_cards_come_from_different_tokens() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);

        let window = window(&tokens)(&theme);
        assert_eq!(
            window.background,
            Some(colour(tokens.colors.background).into())
        );
        assert_eq!(window.text_color, Some(colour(tokens.colors.text)));

        let card = card(&tokens)(&theme);
        assert_eq!(card.background, Some(colour(tokens.colors.surface).into()));
        assert_eq!(card.text_color, Some(colour(tokens.colors.text)));
        assert_eq!(card.border.color, colour(tokens.colors.border));
        assert_eq!(card.border.width, Metrics::from_tokens(&tokens).hairline);
        assert_eq!(card.border.radius, (f32::from(tokens.radii.md)).into());

        assert_ne!(
            window.background, card.background,
            "a card that is the same colour as the window has no boundary",
        );
    }

    #[test]
    fn the_selected_row_is_drawn_in_the_accent_colour() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);

        let selected = row(&tokens, true, true)(&theme, button::Status::Active);
        assert_eq!(
            selected.background,
            Some(colour(tokens.colors.accent).into())
        );
        assert_eq!(selected.text_color, colour(tokens.colors.on_accent));

        let idle = row(&tokens, false, true)(&theme, button::Status::Active);
        assert_eq!(idle.background, None);
        assert_eq!(idle.text_color, colour(tokens.colors.text));
    }

    #[test]
    fn the_five_states_of_the_surface_are_pairwise_distinguishable() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);

        // The five states of the guidelines' §5 as the launcher draws them: a
        // row at rest, hovered, pressed and unusable, and the focused search
        // field, which is where this surface keeps its focus.
        let resting = button_look(row(&tokens, false, true)(&theme, button::Status::Active));
        let hovered = button_look(row(&tokens, false, true)(&theme, button::Status::Hovered));
        let pressed = button_look(row(&tokens, false, true)(&theme, button::Status::Pressed));
        let disabled = button_look(row(&tokens, false, false)(&theme, button::Status::Disabled));
        let focused = field_look(field(&tokens)(
            &theme,
            text_input::Status::Focused { is_hovered: false },
        ));

        let states = [resting, hovered, pressed, disabled, focused];
        assert!(
            all_distinct(&states),
            "two of the surface's five states look the same: {states:?}",
        );

        assert_ne!(hovered.0, resting.0, "hover is a background, not a nothing");
        assert_eq!(pressed.1, colour(tokens.colors.on_accent));
        assert_eq!(
            disabled.1,
            colour(tokens.colors.text_muted),
            "a row that cannot be launched must not look like the selection",
        );
        assert_eq!(disabled.0, None);
        assert_eq!(focused.1, colour(tokens.colors.accent));
    }

    #[test]
    fn the_selected_row_is_drawn_like_the_pressed_one() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);

        // Pressing a row is what selects it (`app::update`), so the two states
        // are one look: the row the user is aiming at, filled with the accent.
        // Selection is a *background* (guidelines §5), which is what the accent
        // fill is; the launcher's one focus ring belongs to the search field.
        assert_eq!(
            button_look(row(&tokens, true, true)(&theme, button::Status::Active)),
            button_look(row(&tokens, false, true)(&theme, button::Status::Pressed)),
        );
    }

    #[test]
    fn a_row_that_cannot_be_launched_is_muted_rather_than_highlighted() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);

        let disabled = row(&tokens, true, false)(&theme, button::Status::Active);
        assert_eq!(
            disabled.text_color,
            colour(tokens.colors.text_muted),
            "a row that cannot be launched must not look like the selection",
        );
        assert_eq!(disabled.background, None);
    }

    #[test]
    fn the_five_action_states_are_pairwise_distinguishable() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);

        let resting = button_look(action(&tokens, true)(&theme, button::Status::Active));
        let hovered = button_look(action(&tokens, true)(&theme, button::Status::Hovered));
        let pressed = button_look(action(&tokens, true)(&theme, button::Status::Pressed));
        let disabled = button_look(action(&tokens, false)(&theme, button::Status::Disabled));

        let states = [resting, hovered, pressed, disabled];
        assert!(
            all_distinct(&states),
            "two action states look the same: {states:?}",
        );
        assert_eq!(resting.0, None, "a resting control has no background");
        assert_eq!(
            hovered.0,
            Some(colour(tokens.colors.surface_variant).into())
        );
        assert_eq!(pressed.0, Some(colour(tokens.colors.accent).into()));
        assert_eq!(pressed.1, colour(tokens.colors.on_accent));
        assert_eq!(disabled.1, colour(tokens.colors.text_muted));
    }

    #[test]
    fn the_focused_search_field_wears_the_focus_ring() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);
        let metrics = Metrics::from_tokens(&tokens);

        let resting = field(&tokens)(&theme, text_input::Status::Active);
        let hovered = field(&tokens)(&theme, text_input::Status::Hovered);
        let focused = field(&tokens)(&theme, text_input::Status::Focused { is_hovered: false });
        let disabled = field(&tokens)(&theme, text_input::Status::Disabled);

        assert_eq!(focused.border.color, colour(tokens.colors.accent));
        assert_eq!(focused.border.width, metrics.focus_ring);
        assert!(
            focused.border.width > resting.border.width,
            "focus has to be wider than the resting outline to be seen",
        );
        assert_eq!(resting.border.color, colour(tokens.colors.border));
        assert_eq!(resting.border.width, metrics.hairline);

        let looks: Vec<_> = [&resting, &hovered, &focused, &disabled]
            .into_iter()
            .map(|style| {
                (
                    style.background,
                    style.border.color,
                    style.border.width,
                    style.value,
                )
            })
            .collect();
        assert!(all_distinct(&looks), "two input states look the same");
    }

    #[test]
    fn the_icon_placeholder_distinguishes_the_two_icon_states() {
        let tokens = tokens();
        assert_ne!(
            icon_background(&tokens, true),
            icon_background(&tokens, false)
        );
        assert_ne!(
            badge_glyph_colour(&tokens, true),
            badge_glyph_colour(&tokens, false)
        );

        let theme = iced_theme(&tokens);
        let named = badge(&tokens, false)(&theme);
        let placeholder = badge(&tokens, true)(&theme);
        assert_eq!(named.background, Some(icon_background(&tokens, false)));
        assert_eq!(placeholder.background, Some(icon_background(&tokens, true)));
        assert_eq!(named.border.radius, (f32::from(tokens.radii.sm)).into());
        assert_eq!(
            named.border.width, 0.0,
            "the icon tile has no outline of its own",
        );
    }

    #[test]
    fn the_text_size_is_the_token_scale_applied_to_iceds_default() {
        let tokens = tokens();
        assert!(text_size(&tokens) > 0.0, "a zero text size draws nothing");
        assert_eq!(
            text_size(&tokens),
            iced::Settings::default().default_text_size.0 * tokens.typography.scale
        );

        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;
        assert_eq!(text_size(&doubled), text_size(&tokens) * 2.0);
    }

    #[test]
    fn the_average_advance_follows_the_font_scale() {
        let tokens = tokens();
        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;

        assert!(average_advance(&tokens) >= 1.0);
        assert_eq!(average_advance(&doubled), average_advance(&tokens) * 2.0);
    }

    #[test]
    fn the_input_style_uses_the_theme_tokens() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);
        let style = field(&tokens)(&theme, text_input::Status::Active);
        assert_eq!(style.placeholder, colour(tokens.colors.text_muted));
        assert_eq!(style.value, colour(tokens.colors.text));
        assert_eq!(style.selection, colour(tokens.colors.accent));
    }

    #[test]
    fn a_row_labels_are_readable_on_the_colour_they_are_drawn_on() {
        let tokens = tokens();

        assert_eq!(
            row_text_colour(&tokens, true, true),
            colour(tokens.colors.on_accent)
        );
        assert_eq!(
            row_text_colour(&tokens, false, true),
            colour(tokens.colors.text)
        );
        assert_eq!(
            row_text_colour(&tokens, false, false),
            colour(tokens.colors.text_muted)
        );
        assert_eq!(
            row_detail_colour(&tokens, true, true),
            colour(tokens.colors.on_accent),
            "a muted caption is not readable on the accent",
        );
        assert_eq!(
            row_detail_colour(&tokens, false, true),
            colour(tokens.colors.text_muted)
        );
    }
}
