//! The Inox display manager: the login lifecycle, and the interface that draws it.
//!
//! `inox-greet` is the optional L4 component PRD §3.1 lists as the display
//! manager, and [ADR 0021] is its specification. Three things about it are
//! constraints rather than features, and they shape every module here.
//!
//! - **It does not authenticate anyone yet.** PAM or any other mechanism, the
//!   helper's privileges and the handling of a credential are items ADR 0015
//!   requires a human to sign off before they land, and a test stub does not
//!   count as authentication. The binary therefore refuses a real login and exits
//!   non-zero; `--preview` opens the interface, which is what this crate's
//!   interface work can be verified against.
//! - **The model never holds a password.** [`state::Greeter`] has no field for
//!   one, and the boundary's vocabulary ([`state::AuthReply`]) is the shape an
//!   answer will arrive in, not a place a credential passes through.
//! - **A preview is not a secure login surface.** It is an ordinary window. The
//!   greeter host, the AT-SPI bridge and the waydriver evidence ADR 0021 lists as
//!   unverified are still unverified, and nothing here claims them.
//!
//! # How the crate is organised
//!
//! | module | what lives there |
//! |---|---|
//! | [`state`] | the login lifecycle and its correlation rules — pure, fully tested |
//! | [`view_model`] | the projection from state to what the interface shows |
//! | [`app`] | `State`, `Message`, `update` (Elm), `view`, `subscription` |
//! | [`view`] | iced widgets, built from the view model and the tokens |
//! | [`theme`] | design tokens turned into iced's theme and styles |
//! | [`i18n`] | the `en-US` and `zh-CN` catalogs this component ships |
//!
//! [ADR 0021]: https://github.com/Inox-desktop/inox/blob/main/docs/adr/0021-l4-session-services.md

#![forbid(unsafe_code)]

pub mod app;
pub mod i18n;
pub mod state;
pub mod theme;
pub mod view;
pub mod view_model;

pub use crate::app::{Message, State};
pub use crate::state::{AttemptId, AuthReply, Command, Event, Greeter, Phase};
pub use crate::view_model::{Action, Face, FocusTarget, GreeterViewModel, Identity};
