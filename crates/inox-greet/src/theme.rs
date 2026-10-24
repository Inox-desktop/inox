//! Design tokens, turned into iced's theme and style closures.
//!
//! Root AGENTS.md §4 forbids a component from hard-coding a colour, a size, a
//! radius or a duration: every one of them comes from [`inox_config::Theme`].
//! iced, meanwhile, expects a [`iced::Theme`] and a handful of `Style` structs
//! built from it. This module is the join between the two, and the only place in
//! the crate where a colour becomes a widget's. `docs/quality/ui-guidelines.md`
//! is the contract this file implements: §1 token discipline, §2 the four type
//! roles, §3 the spacing rhythm, §4 the control and card geometry, §5 the five
//! states a control is drawn in.
//!
//! The login screen has one job for its colours on top of that: a person must be
//! able to see which control the keyboard is on (acceptance Q01). Every control
//! therefore draws its focus ring in the one token that is legible against its own
//! fill — the accent on a plain control, and the on-accent colour on the filled
//! log-in button — and nothing else on the screen uses the accent.
//!
//! ADR 0018 explains why there is a copy of this file in every component instead
//! of one shared crate: the duplication is the price of not adding a same-layer
//! Cargo dependency that would need its own ADR.

use iced::widget::{button, container, text_input};
use iced::{Border, Color, Padding, Theme};
use inox_config::{Rgba, Theme as Tokens};

/// A token colour, as iced wants it.
pub fn color(value: Rgba) -> Color {
    Color::from_rgba8(value.r, value.g, value.b, f32::from(value.a) / 255.0)
}

/// The iced theme the login screen runs under.
pub fn from_tokens(tokens: &Tokens) -> Theme {
    Theme::custom(tokens.name.clone(), palette(tokens))
}

/// The palette iced's own widgets fall back to.
pub fn palette(tokens: &Tokens) -> iced::theme::Palette {
    iced::theme::Palette {
        background: color(tokens.colors.background),
        text: color(tokens.colors.text),
        primary: color(tokens.colors.accent),
        success: color(tokens.colors.success),
        danger: color(tokens.colors.danger),
        warning: color(tokens.colors.warning),
    }
}

// ---------------------------------------------------------------------------
// The type scale (guidelines §2).
//
// Four roles and their multiples are fixed: a window does not get a fifth size,
// and a subsection does not get to be drawn at the size of the page title. What
// they multiply is the base, which is `spacing.lg × typography.scale` — a token
// times the user's scale, so 200 % text is 200 % of the theme's own body size and
// this file names no pixel size of its own (acceptance Q02).
// ---------------------------------------------------------------------------

/// Ratio of the page or window title.
const TITLE_RATIO: f32 = 1.5;

/// Ratio of a card or section heading.
const HEADING_RATIO: f32 = 1.25;

/// Ratio of body text.
const BODY_RATIO: f32 = 1.0;

/// Ratio of captions and secondary lines.
const CAPTION_RATIO: f32 = 0.875;

/// The size every role is a multiple of.
pub fn base_size(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.lg) * tokens.typography.scale
}

/// The size of the page title: the one main heading a login surface has.
pub fn title_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * TITLE_RATIO
}

/// The size of a card or section heading.
pub fn heading_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * HEADING_RATIO
}

/// The size of body text: field values, state lines and control labels.
pub fn body_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * BODY_RATIO
}

/// The size of a caption: field labels, the banner, and every explanation.
pub fn caption_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * CAPTION_RATIO
}

// ---------------------------------------------------------------------------
// Geometry (guidelines §1, §4).
// ---------------------------------------------------------------------------

/// The width of an ordinary outline: half the smallest spacing step.
pub fn hairline(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.xs) / 2.0
}

/// The width of a focus ring, which is one `spacing.xs`.
pub fn focus_ring_width(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.xs)
}

/// The corner radius of a control: a field or a button.
pub fn control_radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.sm).into()
}

/// The corner radius of a card or other boundary of a group.
pub fn card_radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.md).into()
}

/// The padding inside a control.
///
/// Guidelines §3 names `spacing.sm` above and below a control and `md` beside it,
/// and §4 floors an interactive target at `spacing.xl`. With the default 16 px
/// body text a `sm` above and below yields a 27 px target, under the floor, and
/// the tokens carry no second spacing step between `sm` and `md`. The floor is the
/// accessibility rule and the rhythm row is a nominal value, so the vertical
/// padding is `md` here: the target is 32 px at the default scale and grows with
/// `typography.scale` and with the body size, because nothing pins a height.
pub fn control_padding(tokens: &Tokens) -> Padding {
    let step = f32::from(tokens.spacing.md);

    Padding {
        top: step,
        bottom: step,
        left: step,
        right: step,
    }
}

/// Every text colour the login screen uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emphasis {
    /// Body text and control labels.
    None,
    /// Secondary information: what a line explains.
    Muted,
    /// A refusal or a failure.
    Danger,
    /// Something to notice, such as a spent allowance.
    Warning,
    /// Something that went the right way.
    Success,
}

impl Emphasis {
    /// The colour this emphasis is drawn in.
    pub fn colour(self, tokens: &Tokens) -> Color {
        match self {
            Self::None => text(tokens),
            Self::Muted => muted_text(tokens),
            Self::Danger => danger_text(tokens),
            Self::Warning => warning_text(tokens),
            Self::Success => success_text(tokens),
        }
    }
}

/// Default text colour.
pub fn text(tokens: &Tokens) -> Color {
    color(tokens.colors.text)
}

/// Colour of secondary information.
pub fn muted_text(tokens: &Tokens) -> Color {
    color(tokens.colors.text_muted)
}

/// Colour of a refusal.
pub fn danger_text(tokens: &Tokens) -> Color {
    color(tokens.colors.danger)
}

/// Colour of something the user should notice but which is not their fault.
pub fn warning_text(tokens: &Tokens) -> Color {
    color(tokens.colors.warning)
}

/// Colour of a state that went the right way.
pub fn success_text(tokens: &Tokens) -> Color {
    color(tokens.colors.success)
}

/// A border of `colour` at `width`, rounded like a control.
fn control_border(tokens: &Tokens, colour: Rgba, width: f32) -> Border {
    Border {
        color: color(colour),
        width,
        radius: control_radius(tokens),
    }
}

/// A control with no outline of its own.
fn no_border(tokens: &Tokens) -> Border {
    Border {
        color: Color::TRANSPARENT,
        width: 0.0,
        radius: control_radius(tokens),
    }
}

/// The focus ring, in the one token that is legible on `fill`.
pub fn focus_ring(tokens: &Tokens, fill: Rgba) -> Border {
    let ring = if fill == tokens.colors.accent {
        tokens.colors.on_accent
    } else {
        tokens.colors.accent
    };

    control_border(tokens, ring, focus_ring_width(tokens))
}

// ---------------------------------------------------------------------------
// The five states (guidelines §5).
// ---------------------------------------------------------------------------

/// How a control is being used, in the one vocabulary the guidelines define.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    /// Nothing is happening to the control.
    Resting,
    /// The pointer is over it.
    Hovered,
    /// It is being pressed.
    Pressed,
    /// The keyboard is on it.
    Focused,
    /// It cannot be used, and the screen says why beside it.
    Disabled,
}

impl ControlState {
    /// The state iced's own interaction status describes.
    pub fn from_status(status: button::Status) -> Self {
        match status {
            button::Status::Active => Self::Resting,
            button::Status::Hovered => Self::Hovered,
            button::Status::Pressed => Self::Pressed,
            button::Status::Disabled => Self::Disabled,
        }
    }
}

/// The style of a control that is *not* the screen's one primary action.
///
/// Guidelines §5, in order: resting is text on a hairline outline, hovered takes
/// the alternate surface, pressed fills with the accent, the focus ring is the
/// accent at one `spacing.xs`, and disabled keeps only the muted text. A control
/// that is both hovered (or pressed) and focused shows both: the fill says what
/// the pointer is doing and the ring says where the keyboard is, which is the one
/// thing a keyboard user cannot afford to lose.
pub fn action_style(tokens: &Tokens, state: ControlState, focused: bool) -> button::Style {
    match state {
        ControlState::Disabled => button::Style {
            background: None,
            text_color: muted_text(tokens),
            border: no_border(tokens),
            ..button::Style::default()
        },
        ControlState::Hovered => button::Style {
            background: Some(color(tokens.colors.surface_variant).into()),
            text_color: text(tokens),
            border: if focused {
                focus_ring(tokens, tokens.colors.surface_variant)
            } else {
                control_border(tokens, tokens.colors.border, hairline(tokens))
            },
            ..button::Style::default()
        },
        ControlState::Pressed => button::Style {
            background: Some(color(tokens.colors.accent).into()),
            text_color: color(tokens.colors.on_accent),
            border: if focused {
                focus_ring(tokens, tokens.colors.accent)
            } else {
                no_border(tokens)
            },
            ..button::Style::default()
        },
        ControlState::Resting | ControlState::Focused => button::Style {
            background: None,
            text_color: text(tokens),
            border: if focused || state == ControlState::Focused {
                focus_ring(tokens, tokens.colors.surface)
            } else {
                control_border(tokens, tokens.colors.border, hairline(tokens))
            },
            ..button::Style::default()
        },
    }
}

/// The style of the screen's one primary action.
///
/// Filled with the accent while it can be used, and reduced to muted text with no
/// background when it cannot (guidelines §5; acceptance D05 draws the reason
/// beside it). The *fill* does not follow the pointer: the tokens carry no hover
/// variant of the accent, and the one token that means "hover" would make the
/// primary action look disabled. The hover is answered by an `on_accent` hairline
/// instead — a cue that costs no new shade — so the button a user is about to
/// press still says so before they press it. A press is answered as well, and by
/// an exchange of the two colours that are defined to contrast with each other,
/// so it is visible the instant it happens.
pub fn submit_style(tokens: &Tokens, state: ControlState, focused: bool) -> button::Style {
    if state == ControlState::Disabled {
        return button::Style {
            background: None,
            text_color: muted_text(tokens),
            border: no_border(tokens),
            ..button::Style::default()
        };
    }

    if state == ControlState::Pressed {
        return button::Style {
            background: Some(color(tokens.colors.on_accent).into()),
            text_color: color(tokens.colors.accent),
            border: if focused {
                focus_ring(tokens, tokens.colors.on_accent)
            } else {
                no_border(tokens)
            },
            ..button::Style::default()
        };
    }

    if state == ControlState::Hovered && !focused {
        return button::Style {
            background: Some(color(tokens.colors.accent).into()),
            text_color: color(tokens.colors.on_accent),
            border: control_border(tokens, tokens.colors.on_accent, hairline(tokens)),
            ..button::Style::default()
        };
    }

    button::Style {
        background: Some(color(tokens.colors.accent).into()),
        text_color: color(tokens.colors.on_accent),
        border: if focused {
            focus_ring(tokens, tokens.colors.accent)
        } else {
            no_border(tokens)
        },
        ..button::Style::default()
    }
}

// ---------------------------------------------------------------------------
// The surface, the card, and the fields.
// ---------------------------------------------------------------------------

/// The whole surface of the login window.
pub fn screen(tokens: &Tokens) -> impl Fn(&Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(color(tokens.colors.background).into()),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The card a group of the form sits on: `surface`, a hairline outline and `md`
/// corners.
pub fn card(tokens: &Tokens) -> impl Fn(&Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(color(tokens.colors.surface).into()),
        border: Border {
            color: color(tokens.colors.border),
            width: hairline(tokens),
            radius: card_radius(tokens),
        },
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

// ---------------------------------------------------------------------------
// The account medallion and the account list.
// ---------------------------------------------------------------------------

/// Ratio of the head medallion's diameter to the base the type scale is built on.
///
/// A medallion is a shape rather than a type role, so it is not one of the four
/// multiples in guidelines §2; deriving it from the same base is what makes it
/// grow with the person's font scale like everything else (acceptance Q02).
const MEDALLION_RATIO: f32 = 3.0;

/// Ratio of the medallion on an account row: smaller, because a column of avatars
/// as tall as the head one is a column of avatars with no room for names.
const ROW_MEDALLION_RATIO: f32 = 2.0;

/// The diameter of the account medallion at the head of the login screen.
pub fn medallion_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * MEDALLION_RATIO
}

/// The diameter of the medallion on one row of the account list.
pub fn row_medallion_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * ROW_MEDALLION_RATIO
}

/// The medallion: an account's initial on the alternate surface, round.
///
/// The diameter is what makes it a circle, so it is taken rather than derived
/// here: the head medallion and the one on a row are the same shape at two sizes.
pub fn medallion(tokens: &Tokens, diameter: f32) -> impl Fn(&Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(color(tokens.colors.surface_variant).into()),
        border: Border {
            color: color(tokens.colors.border),
            width: hairline(tokens),
            radius: (diameter / 2.0).into(),
        },
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The style of one row of the account list, in iced's own shape.
pub fn account(
    tokens: &Tokens,
    selected: bool,
    focused: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + '_ {
    move |_, status| account_style(tokens, ControlState::from_status(status), selected, focused)
}

/// The style of one row of the account list.
///
/// The account a login would use is the one filled with the accent — the same
/// fact the launcher's selected row states the same way. An account that is not
/// selected is an ordinary control from guidelines §5: no background at rest, the
/// alternate surface under the pointer, the accent while it is being pressed. The
/// focus is the ring, drawn in `on_accent` on the filled row so that it cannot
/// disappear into the fill it is on.
pub fn account_style(
    tokens: &Tokens,
    state: ControlState,
    selected: bool,
    focused: bool,
) -> button::Style {
    if state == ControlState::Disabled {
        return button::Style {
            background: None,
            text_color: muted_text(tokens),
            border: no_border(tokens),
            ..button::Style::default()
        };
    }

    if selected || state == ControlState::Pressed {
        return button::Style {
            background: Some(color(tokens.colors.accent).into()),
            text_color: color(tokens.colors.on_accent),
            border: if focused {
                focus_ring(tokens, tokens.colors.accent)
            } else if state == ControlState::Hovered {
                control_border(tokens, tokens.colors.on_accent, hairline(tokens))
            } else {
                no_border(tokens)
            },
            ..button::Style::default()
        };
    }

    button::Style {
        background: (state == ControlState::Hovered)
            .then(|| color(tokens.colors.surface_variant).into()),
        text_color: text(tokens),
        border: if focused {
            focus_ring(tokens, tokens.colors.surface)
        } else {
            control_border(tokens, tokens.colors.border, hairline(tokens))
        },
        ..button::Style::default()
    }
}

/// The frame of a field that can be typed into.
///
/// Focused it carries the focus ring, in error it carries the danger colour with
/// the error text beside it, and otherwise it is an ordinary `border` outline on
/// the `background` token. The three are exclusive, so "focused" and "wrong" are
/// never drawn as the same thing.
pub fn field(
    tokens: &Tokens,
    focused: bool,
    error: bool,
) -> impl Fn(&Theme) -> container::Style + '_ {
    move |_| field_style(tokens, focused, error, true)
}

/// The style of a field, in iced's own shape.
///
/// `editable` is the fourth state a field has, and it is not one of the four
/// above: guidelines §5 says disabled is the muted text with no background — and
/// no frame either, because a box drawn around a field says "type here" in the one
/// language a form has. A login window that cannot log anyone in must not hold out
/// a box to type a password into; what the person needs instead is the sentence
/// beside it saying why.
pub fn field_style(
    tokens: &Tokens,
    focused: bool,
    error: bool,
    editable: bool,
) -> container::Style {
    if !editable {
        return container::Style {
            background: None,
            border: Border::default(),
            text_color: Some(muted_text(tokens)),
            ..container::Style::default()
        };
    }

    container::Style {
        background: Some(color(tokens.colors.background).into()),
        border: if focused {
            focus_ring(tokens, tokens.colors.background)
        } else if error {
            control_border(tokens, tokens.colors.danger, hairline(tokens))
        } else {
            control_border(tokens, tokens.colors.border, hairline(tokens))
        },
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The log-in button, in iced's own style shape.
///
/// A control that cannot be pressed is a button with no `on_press`, which is how
/// iced reports it as disabled — so "the control is disabled" and "the control
/// sends nothing" are the same fact rather than two that could disagree.
pub fn submit(
    tokens: &Tokens,
    enabled: bool,
    focused: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + '_ {
    move |_, status| {
        let state = ControlState::from_status(status);

        if !enabled || state == ControlState::Disabled {
            submit_style(tokens, ControlState::Disabled, focused)
        } else {
            submit_style(tokens, state, focused)
        }
    }
}

/// The clear button.
pub fn action(
    tokens: &Tokens,
    focused: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + '_ {
    move |_, status| action_style(tokens, ControlState::from_status(status), focused)
}

/// The identity field's editor, drawn with no frame of its own.
///
/// The editor lives inside the field's container, which draws the one frame on
/// screen (see [`field`]); an editor with a second frame would draw two rectangles
/// around one field. Guidelines §1 keeps the colour-to-style join here rather than
/// in the view, which is why this is a style and not a widget.
pub fn bare(tokens: &Tokens) -> impl Fn(&Theme, text_input::Status) -> text_input::Style + '_ {
    move |_, _| text_input::Style {
        background: Color::TRANSPARENT.into(),
        border: Border::default(),
        icon: muted_text(tokens),
        placeholder: muted_text(tokens),
        value: text(tokens),
        selection: color(tokens.colors.accent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::theme::Base;
    use inox_config::{BUILTIN_DARK, BUILTIN_LIGHT};

    fn tokens() -> Tokens {
        Tokens::builtin_dark()
    }

    #[test]
    fn the_iced_theme_is_named_after_the_tokens_it_came_from() {
        assert_eq!(from_tokens(&Tokens::builtin_light()).name(), BUILTIN_LIGHT);
        assert_eq!(from_tokens(&Tokens::builtin_dark()).name(), BUILTIN_DARK);
    }

    #[test]
    fn the_screen_is_fully_opaque() {
        let tokens = tokens();
        let style = screen(&tokens)(&from_tokens(&tokens));

        assert_eq!(
            color(tokens.colors.background).a,
            1.0,
            "a login screen that is not opaque shows whatever is behind it"
        );
        assert_eq!(
            style.background,
            Some(color(tokens.colors.background).into())
        );
    }

    #[test]
    fn the_focus_ring_is_visible_on_every_control_it_is_drawn_on() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        for (control, fill) in [
            ("field", tokens.colors.background),
            ("clear", tokens.colors.surface),
            ("log in", tokens.colors.accent),
        ] {
            let ring = focus_ring(&tokens, fill);

            assert_eq!(
                ring.width,
                focus_ring_width(&tokens),
                "{control} has no ring width"
            );
            assert_ne!(
                ring.color,
                Color::TRANSPARENT,
                "{control}'s focus ring is invisible"
            );
            assert_ne!(
                ring.color,
                color(fill),
                "{control}'s focus ring is the same colour as the control"
            );
            assert!(ring.width > 0.0);
        }

        // The two fills really do resolve to two different rings, so "the ring is
        // legible against the fill" is a rule and not a coincidence.
        assert_ne!(
            focus_ring(&tokens, tokens.colors.background).color,
            focus_ring(&tokens, tokens.colors.accent).color
        );

        let style = field(&tokens, true, false)(&theme);
        assert_eq!(style.border, focus_ring(&tokens, tokens.colors.background));
    }

    #[test]
    fn a_focused_field_is_not_drawn_as_a_wrong_one() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let focused = field(&tokens, true, false)(&theme);
        let wrong = field(&tokens, false, true)(&theme);
        let plain = field(&tokens, false, false)(&theme);

        assert_ne!(focused.border.color, wrong.border.color);
        assert_ne!(focused.border.color, plain.border.color);
        assert_ne!(wrong.border.color, plain.border.color);
        assert_eq!(wrong.border.color, color(tokens.colors.danger));
    }

    #[test]
    fn a_field_that_cannot_be_typed_into_is_not_drawn_as_a_box() {
        let tokens = tokens();
        let style = field_style(&tokens, false, false, false);

        assert_eq!(
            style.background, None,
            "guidelines §5: a disabled control carries no background"
        );
        assert_eq!(
            style.border,
            Border::default(),
            "and no frame, because a frame around a field says `type here`"
        );
        assert_eq!(style.text_color, Some(muted_text(&tokens)));
        assert_ne!(
            style,
            field_style(&tokens, false, false, true),
            "a field that cannot be typed into is not drawn like one that can"
        );
    }

    #[test]
    fn a_disabled_submit_is_muted_rather_than_emphasized() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let enabled = submit(&tokens, true, false)(&theme, button::Status::Active);
        let disabled = submit(&tokens, false, false)(&theme, button::Status::Active);

        assert_eq!(enabled.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(enabled.text_color, color(tokens.colors.on_accent));
        assert_eq!(disabled.background, None);
        assert_eq!(disabled.text_color, muted_text(&tokens));
    }

    #[test]
    fn a_disabled_control_has_no_ring_to_promise_it_can_be_used() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        // Focus implies enabled for every button in this crate; the ring a caller
        // could otherwise draw on a disabled control is deliberately absent.
        let style = submit(&tokens, false, true)(&theme, button::Status::Active);

        assert_eq!(style.border.width, 0.0);
        assert_eq!(style.background, None);
    }

    #[test]
    fn the_log_in_button_keeps_its_fill_and_shows_the_hover_as_an_outline() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let idle = submit(&tokens, true, false)(&theme, button::Status::Active);
        let hovered = submit(&tokens, true, false)(&theme, button::Status::Hovered);

        assert_eq!(
            idle.background, hovered.background,
            "the fill does not follow the pointer: the primary action stays the accent"
        );
        assert_eq!(idle.text_color, hovered.text_color);
        assert_eq!(
            hovered.border.color,
            color(tokens.colors.on_accent),
            "guidelines §5: the hover is drawn as an on_accent hairline"
        );
        assert_ne!(
            (idle.border.width, idle.border.color),
            (hovered.border.width, hovered.border.color),
            "a hover the user cannot see is a hover the screen did not answer"
        );
    }

    #[test]
    fn a_press_is_answered_before_the_pointer_leaves_the_control() {
        let tokens = tokens();

        let resting = submit_style(&tokens, ControlState::Resting, false);
        let pressed = submit_style(&tokens, ControlState::Pressed, false);

        assert_ne!(
            resting, pressed,
            "guidelines §5: the feedback happens on the press, not on the release"
        );
        assert_eq!(
            pressed.background,
            Some(color(tokens.colors.on_accent).into())
        );
        assert_eq!(pressed.text_color, color(tokens.colors.accent));
    }

    #[test]
    fn the_five_control_states_are_pairwise_distinguishable() {
        let tokens = tokens();
        let states = [
            ControlState::Resting,
            ControlState::Hovered,
            ControlState::Pressed,
            ControlState::Focused,
            ControlState::Disabled,
        ];

        for (index, one) in states.iter().enumerate() {
            for other in &states[index + 1..] {
                assert_ne!(
                    action_style(&tokens, *one, false),
                    action_style(&tokens, *other, false),
                    "{one:?} and {other:?} are drawn as the same control"
                );
            }
        }

        assert_eq!(
            action_style(&tokens, ControlState::Resting, false).background,
            None,
            "guidelines §5: a control at rest carries no background"
        );
        assert_eq!(
            action_style(&tokens, ControlState::Hovered, false).background,
            Some(color(tokens.colors.surface_variant).into())
        );
        assert_eq!(
            action_style(&tokens, ControlState::Pressed, false).background,
            Some(color(tokens.colors.accent).into())
        );
        assert_eq!(
            action_style(&tokens, ControlState::Focused, false)
                .border
                .color,
            color(tokens.colors.accent)
        );
        assert_eq!(
            action_style(&tokens, ControlState::Disabled, false).text_color,
            muted_text(&tokens)
        );
    }

    #[test]
    fn a_focused_control_shows_what_the_pointer_is_doing_as_well() {
        // Guidelines §5: the two states have to be able to exist together and both
        // be seen. The fill says the pointer is over the control, the ring says the
        // keyboard is on it, and neither hides the other.
        let tokens = tokens();

        let hovered = action_style(&tokens, ControlState::Hovered, true);

        assert_eq!(
            hovered.background,
            Some(color(tokens.colors.surface_variant).into())
        );
        assert_eq!(hovered.border.width, focus_ring_width(&tokens));
        assert_ne!(hovered.border.color, color(tokens.colors.surface_variant));
        assert_ne!(hovered.border.color, Color::TRANSPARENT);

        let pressed = action_style(&tokens, ControlState::Pressed, true);

        assert_eq!(pressed.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(pressed.border.color, color(tokens.colors.on_accent));
        assert_ne!(pressed.border.width, 0.0);
    }

    #[test]
    fn a_control_keeps_the_token_radius() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        assert_eq!(
            submit(&tokens, true, false)(&theme, button::Status::Active)
                .border
                .radius,
            control_radius(&tokens)
        );
        assert_eq!(
            action(&tokens, false)(&theme, button::Status::Active)
                .border
                .radius,
            control_radius(&tokens)
        );
        assert_eq!(card(&tokens)(&theme).border.radius, card_radius(&tokens));
    }

    #[test]
    fn a_card_is_rounder_than_the_controls_on_it() {
        let tokens = tokens();

        assert_ne!(card_radius(&tokens), control_radius(&tokens));
        assert_eq!(
            control_radius(&tokens),
            iced::border::Radius::from(f32::from(tokens.radii.sm))
        );
        assert_eq!(
            card_radius(&tokens),
            iced::border::Radius::from(f32::from(tokens.radii.md))
        );
    }

    #[test]
    fn the_editor_inside_a_field_draws_no_second_frame() {
        let tokens = tokens();
        let style = bare(&tokens)(&from_tokens(&tokens), text_input::Status::Active);

        assert_eq!(
            style.border,
            Border::default(),
            "the field's container draws the one frame on screen"
        );
        assert_eq!(style.background, Color::TRANSPARENT.into());
        assert_eq!(style.placeholder, muted_text(&tokens));
        assert_eq!(style.value, text(&tokens));
        assert_ne!(style.selection, style.value);
    }

    #[test]
    fn every_distance_is_derived_from_a_token() {
        let mut tokens = tokens();
        tokens.spacing.xs = 4;
        tokens.spacing.md = 10;
        tokens.radii.sm = 1;

        assert_eq!(hairline(&tokens), 2.0, "half of the smallest spacing step");
        assert_eq!(focus_ring_width(&tokens), 4.0);

        let padding = control_padding(&tokens);

        assert_eq!(padding.top, 10.0);
        assert_eq!(padding.bottom, 10.0);
        assert_eq!(padding.left, 10.0);
        assert_eq!(padding.right, 10.0);
        assert_eq!(control_radius(&tokens).top_left, 1.0);
    }

    #[test]
    fn the_control_target_clears_the_guidelines_floor() {
        // Guidelines §4 floors an interactive target at `spacing.xl`, and the tokens
        // carry no height, so the floor has to fall out of the padding and the text
        // together.
        for tokens in [Tokens::builtin_light(), Tokens::builtin_dark()] {
            let target = body_size(&tokens) + control_padding(&tokens).top * 2.0;

            assert!(
                target >= f32::from(tokens.spacing.xl),
                "a control is {target} px tall, under the {} px floor",
                tokens.spacing.xl
            );
        }
    }

    #[test]
    fn text_sizes_follow_the_token_scale() {
        let tokens = tokens();
        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;

        assert!(body_size(&tokens) > 0.0, "a zero text size draws nothing");
        assert_eq!(body_size(&tokens), f32::from(tokens.spacing.lg));
        assert_eq!(body_size(&doubled), body_size(&tokens) * 2.0);
        assert_eq!(title_size(&doubled), title_size(&tokens) * 2.0);
        assert_eq!(heading_size(&doubled), heading_size(&tokens) * 2.0);
        assert_eq!(caption_size(&doubled), caption_size(&tokens) * 2.0);
    }

    #[test]
    fn the_four_roles_are_ordered_and_none_of_them_is_too_small() {
        let tokens = tokens();

        assert!(title_size(&tokens) > heading_size(&tokens));
        assert!(heading_size(&tokens) > body_size(&tokens));
        assert!(body_size(&tokens) > caption_size(&tokens));
        assert!(
            caption_size(&tokens) >= body_size(&tokens) * 0.75,
            "guidelines §2: a caption is never smaller than 0.75 of the body"
        );
        assert_eq!(title_size(&tokens), body_size(&tokens) * 1.5);
        assert_eq!(heading_size(&tokens), body_size(&tokens) * 1.25);
        assert_eq!(caption_size(&tokens), body_size(&tokens) * 0.875);
    }

    #[test]
    fn every_emphasis_is_a_different_colour() {
        let tokens = tokens();
        let colours = [
            Emphasis::None.colour(&tokens),
            Emphasis::Muted.colour(&tokens),
            Emphasis::Danger.colour(&tokens),
            Emphasis::Warning.colour(&tokens),
            Emphasis::Success.colour(&tokens),
        ];

        for (index, one) in colours.iter().enumerate() {
            for other in &colours[index + 1..] {
                assert_ne!(one, other, "two emphases share a colour");
            }
        }
    }

    #[test]
    fn a_token_colour_keeps_its_channels_and_its_alpha() {
        let rgba = Rgba {
            r: 0x23,
            g: 0x26,
            b: 0x2e,
            a: 0x80,
        };
        let colour = color(rgba);

        assert!((colour.r - 0x23 as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.g - 0x26 as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.b - 0x2e as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.a - 128.0 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_two_builtin_themes_produce_different_palettes() {
        let light = from_tokens(&Tokens::builtin_light());
        let dark = from_tokens(&Tokens::builtin_dark());

        assert_ne!(light.palette().background, dark.palette().background);
        assert_ne!(light.palette().text, dark.palette().text);
    }

    #[test]
    fn a_medallion_is_a_circle_that_grows_with_the_text() {
        let tokens = tokens();
        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;

        assert_eq!(
            medallion_size(&tokens),
            body_size(&tokens) * MEDALLION_RATIO
        );
        assert_eq!(
            row_medallion_size(&tokens),
            body_size(&tokens) * ROW_MEDALLION_RATIO
        );
        assert!(
            row_medallion_size(&tokens) < medallion_size(&tokens),
            "a row leaves room for the name beside it"
        );

        for (one, other) in [
            (medallion_size(&tokens), medallion_size(&doubled)),
            (row_medallion_size(&tokens), row_medallion_size(&doubled)),
        ] {
            assert_eq!(
                other,
                one * 2.0,
                "a medallion that did not grow with the text would be a picture \
                 of a size nobody asked for"
            );
        }

        // Half the diameter is what makes it a circle, and it is drawn from the
        // tokens it was handed.
        let diameter = medallion_size(&tokens);
        let style = medallion(&tokens, diameter)(&from_tokens(&tokens));

        assert_eq!(style.border.radius.top_left, diameter / 2.0);
        assert_eq!(style.border.width, hairline(&tokens));
    }

    #[test]
    fn the_medallion_does_not_spend_the_accent() {
        let tokens = tokens();
        let style = medallion(&tokens, medallion_size(&tokens))(&from_tokens(&tokens));

        assert_eq!(
            style.background,
            Some(color(tokens.colors.surface_variant).into())
        );
        assert_ne!(
            style.background,
            Some(color(tokens.colors.accent).into()),
            "the accent is the screen's one call to action, and an avatar is not one"
        );
        assert_eq!(style.text_color, Some(text(&tokens)));
    }

    #[test]
    fn an_account_that_is_not_selected_behaves_like_any_other_control() {
        let tokens = tokens();
        let resting = account_style(&tokens, ControlState::Resting, false, false);
        let hovered = account_style(&tokens, ControlState::Hovered, false, false);
        let pressed = account_style(&tokens, ControlState::Pressed, false, false);
        let disabled = account_style(&tokens, ControlState::Disabled, false, false);

        assert_eq!(resting.background, None);
        assert_eq!(
            hovered.background,
            Some(color(tokens.colors.surface_variant).into())
        );
        assert_eq!(pressed.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(pressed.text_color, color(tokens.colors.on_accent));
        assert_eq!(disabled.text_color, muted_text(&tokens));
        assert_eq!(disabled.background, None);
    }

    #[test]
    fn every_state_of_an_account_that_is_not_selected_is_distinguishable() {
        let tokens = tokens();
        let states = [
            ControlState::Resting,
            ControlState::Hovered,
            ControlState::Pressed,
            ControlState::Disabled,
        ];

        // What the person sees: whether there is a fill, in what colour the text
        // is drawn, and whether there is an outline.
        let shape = |style: button::Style| {
            (
                style.background.is_some(),
                style.text_color,
                style.border.width,
            )
        };

        for (index, one) in states.iter().enumerate() {
            for other in &states[index + 1..] {
                assert_ne!(
                    shape(account_style(&tokens, *one, false, false)),
                    shape(account_style(&tokens, *other, false, false)),
                    "{one:?} and {other:?} are drawn as the same row"
                );
            }
        }
    }

    #[test]
    fn the_account_a_login_would_use_is_the_filled_one_and_keeps_its_ring() {
        let tokens = tokens();
        let selected = account_style(&tokens, ControlState::Resting, true, false);
        let unselected = account_style(&tokens, ControlState::Resting, false, false);

        assert_eq!(
            selected.background,
            Some(color(tokens.colors.accent).into()),
            "the account a login would use says so by being the filled row"
        );
        assert_eq!(selected.text_color, color(tokens.colors.on_accent));
        assert_ne!(selected.background, unselected.background);

        // The focus ring cannot be the accent on an accent fill: it would be a
        // ring nobody can see (guidelines §5).
        let focused = account_style(&tokens, ControlState::Resting, true, true);
        assert_eq!(focused.border.width, focus_ring_width(&tokens));
        assert_eq!(focused.border.color, color(tokens.colors.on_accent));

        // Hovering the row a login would already use is answered without changing
        // the fill, the same way the primary action answers a hover.
        let hovered = account_style(&tokens, ControlState::Hovered, true, false);
        assert_eq!(hovered.background, selected.background);
        assert_ne!(
            hovered.border, selected.border,
            "a hover the user cannot see is a hover the screen did not answer"
        );
    }
}
