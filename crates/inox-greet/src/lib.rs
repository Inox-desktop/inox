//! The Inox display manager: the login lifecycle, and the interface that draws it.
//!
//! `inox-greet` is the optional L4 component PRD §3.1 lists as the display
//! manager, and [ADR 0021] is its specification. Three things about it are
//! constraints rather than features, and they shape every module here.
//!
//! - **It authenticates nobody itself.** The boundary is `inox-auth` (ADR 0029):
//!   this crate speaks its `auth-v1` contract and carries a credential one way, and
//!   it links no PAM, starts no session and can invent no answer. What it *can* say
//!   is that there is no boundary to talk to, which is what the default invocation
//!   refuses to open a window over.
//! - **The credential exists here only while it is being typed.** It is a
//!   [`inox_common::secret::Secret`] — zeroed on clear and on drop, unprintable —
//!   and [`state::Event::Submit`] moves it into the command that carries it to the
//!   boundary. Nothing keeps a copy for a retry, and the projection that the view
//!   and the tests print is handed a *count* of characters, never the value.
//! - **A preview is not a secure login surface.** It is an ordinary window. The
//!   greeter host, the AT-SPI bridge and the waydriver evidence ADR 0021 lists as
//!   unverified are still unverified, and nothing here claims them.
//!
//! # How the crate is organised
//!
//! | module | what lives there |
//! |---|---|
//! | [`state`] | the login lifecycle and its correlation rules — pure, fully tested |
//! | [`accounts`] | the accounts the system offers, and the one to offer first |
//! | [`view_model`] | the projection from state to what the interface shows |
//! | [`boundary`] | the `auth-v1` client: one connection, one request, one answer |
//! | [`app`] | `State`, `Message`, `update` (Elm), `view`, `subscription` |
//! | [`view`] | iced widgets, built from the view model and the tokens |
//! | [`theme`] | design tokens turned into iced's theme and styles |
//! | [`i18n`] | the `en-US` and `zh-CN` catalogs this component ships |
//!
//! [ADR 0021]: https://github.com/Inox-desktop/inox/blob/main/docs/adr/0021-l4-session-services.md

#![forbid(unsafe_code)]

pub mod accounts;
pub mod app;
pub mod boundary;
pub mod i18n;
pub mod state;
pub mod theme;
pub mod view;
pub mod view_model;

pub use crate::accounts::{Account, Accounts};
pub use crate::app::{Message, State};
pub use crate::state::{AttemptId, AuthReply, Command, Event, Greeter, Phase};
pub use crate::view_model::{AccountView, Action, Face, FocusTarget, GreeterViewModel};
