//! The panel page: the strip along an output's edge.
//!
//! Every field is a field of [`inox_config::PanelConfig`], and every control is
//! built from that type's own shape: the edge is
//! [`inox_config::PanelPosition`], the height is the range `inox-config`
//! enforces while parsing `shell.toml`, and the two switches are booleans. A
//! value the schema rejects cannot be entered here.
//!
use crate::draft::Field;

/// The fields this page edits, in the order the page draws them: where the strip
/// sits, then how it behaves, then its clock, then its task buttons.
/// [`crate::pages::PageId::groups`] splits them into the cards, and the keyboard
/// walks them in the order they are drawn (acceptance Q01).
pub const FIELDS: &[Field] = &[
    Field::PanelPosition,
    Field::PanelHeight,
    Field::PanelMargin,
    Field::PanelExclusive,
    Field::PanelAutohide,
    Field::PanelShowClock,
    Field::PanelClockFormat,
    Field::PanelMaxTaskButtons,
    Field::PanelIconSize,
];
