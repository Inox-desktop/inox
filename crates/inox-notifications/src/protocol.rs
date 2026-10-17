//! The freedesktop desktop-notification contract, as data.
//!
//! Everything in this module is *the wire*: the bus name, the object path, the
//! interface, the four methods with their exact argument lists, the two signals,
//! the capabilities Inox claims, and the enumerations the specification defines
//! (close reasons, urgency values, hint keys). Nothing here talks to D-Bus —
//! there is no transport in this milestone (PRD §13 puts the service in M5) —
//! but the M5 service and today's tests read the same constants, so the contract
//! is written down once and cannot drift between the two.
//!
//! Spelling out the signatures instead of only naming the methods is deliberate:
//! the argument list of `Notify` is long, positional and easy to get subtly
//! wrong, and [`Method::signature_is_consistent`] makes the parts and the whole
//! check each other.
//!
//! # What Inox claims, and what it does not
//!
//! [`CAPABILITIES`] is the list this daemon can back up today. A capability is
//! claimed only when a popup can actually present it, so `icon-static` and
//! `body-images` are **not** claimed: decoding an icon or a body image needs a
//! file or theme lookup off the UI thread, which this milestone does not have
//! (`<img>` is still parsed and its alt text is shown, and a popup with an icon
//! reference draws the space the icon will occupy — see [`crate::view`]).
//! `sound` is not claimed either: the `sound-name` and `suppress-sound` hints are
//! recorded, but Inox plays no sounds yet. `persistence` is not claimed, because
//! the promise it makes is the `resident` hint's: a notification would be kept
//! after one of its actions is invoked, and this daemon closes it instead (see
//! [`crate::app`]). The hint is read and recorded either way, so the list may
//! grow a `persistence` the day the daemon keeps one.

/// The bus name the notification service owns.
pub const BUS_NAME: &str = "org.freedesktop.Notifications";

/// The object path the notification service is exported at.
pub const OBJECT_PATH: &str = "/org/freedesktop/Notifications";

/// The interface the four methods and two signals belong to.
pub const INTERFACE: &str = "org.freedesktop.Notifications";

/// The version of the desktop-notification specification this crate implements.
pub const SPEC_VERSION: &str = "1.2";

/// The action key the specification reserves for "the user activated the
/// notification itself", which clients receive as an ordinary action.
pub const DEFAULT_ACTION_KEY: &str = "default";

/// `expire_timeout` meaning "let the server decide".
pub const EXPIRETIMEOUT_SERVER_DEFAULT: i32 = -1;

/// `expire_timeout` meaning "never expire on its own".
pub const EXPIRETIMEOUT_NEVER: i32 = 0;

/// What `GetServerInformation` reports about the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerInformation {
    /// The name of the server, shown by `notify-send --debug` and the like.
    pub name: &'static str,
    /// Who publishes it.
    pub vendor: &'static str,
    /// The version of the daemon, which is the workspace version.
    pub version: &'static str,
    /// The version of the specification implemented.
    pub spec_version: &'static str,
}

/// What Inox answers `GetServerInformation` with.
pub const SERVER_INFORMATION: ServerInformation = ServerInformation {
    name: "Inox",
    vendor: "Inox Desktop",
    version: env!("CARGO_PKG_VERSION"),
    spec_version: SPEC_VERSION,
};

/// One positional argument of a method or signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argument {
    /// The name the specification gives the argument.
    pub name: &'static str,
    /// Its D-Bus type signature (`s`, `u`, `i`, `as`, `a{sv}`, …).
    pub signature: &'static str,
}

/// A method of the interface, with its arguments and its return value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Method {
    /// The method name.
    pub name: &'static str,
    /// The arguments, in the order the wire carries them.
    pub arguments: &'static [Argument],
    /// What the call returns; empty for a method with no return value.
    pub returns: &'static [Argument],
    /// Every argument's signature followed by every return's.
    pub signature: &'static str,
}

impl Method {
    /// The number of arguments the method takes.
    pub const fn arity(&self) -> usize {
        self.arguments.len()
    }

    /// The argument called `name`, if the method has one.
    pub fn argument(&self, name: &str) -> Option<&'static Argument> {
        self.arguments.iter().find(|argument| argument.name == name)
    }

    /// Whether [`Method::signature`] is what the parts spell out.
    ///
    /// The single check every method pays for itself: a mistyped argument
    /// signature, a missing return value or a reordered list all make the whole
    /// disagree with the parts.
    pub fn signature_is_consistent(&self) -> bool {
        let mut spelled = String::new();
        for argument in self.arguments.iter().chain(self.returns) {
            spelled.push_str(argument.signature);
        }

        spelled == self.signature
    }
}

/// A signal the service emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signal {
    /// The signal name.
    pub name: &'static str,
    /// The arguments it carries.
    pub arguments: &'static [Argument],
    /// Every argument's signature.
    pub signature: &'static str,
}

impl Signal {
    /// Whether [`Signal::signature`] is what the arguments spell out.
    pub fn signature_is_consistent(&self) -> bool {
        let spelled: String = self
            .arguments
            .iter()
            .map(|argument| argument.signature)
            .collect();

        spelled == self.signature
    }
}

/// `Notify`, the method an application sends a notification with.
///
/// `replaces_id` is `0` for a new notification; any other value names the
/// notification this one replaces, which keeps its identifier.
pub const NOTIFY: Method = Method {
    name: "Notify",
    arguments: &[
        Argument {
            name: "app_name",
            signature: "s",
        },
        Argument {
            name: "replaces_id",
            signature: "u",
        },
        Argument {
            name: "app_icon",
            signature: "s",
        },
        Argument {
            name: "summary",
            signature: "s",
        },
        Argument {
            name: "body",
            signature: "s",
        },
        Argument {
            name: "actions",
            signature: "as",
        },
        Argument {
            name: "hints",
            signature: "a{sv}",
        },
        Argument {
            name: "expire_timeout",
            signature: "i",
        },
    ],
    returns: &[Argument {
        name: "id",
        signature: "u",
    }],
    signature: "susssasa{sv}iu",
};

/// `CloseNotification`, which asks the daemon to close one notification.
pub const CLOSE_NOTIFICATION: Method = Method {
    name: "CloseNotification",
    arguments: &[Argument {
        name: "id",
        signature: "u",
    }],
    returns: &[],
    signature: "u",
};

/// `GetCapabilities`, which answers with the list of [`CAPABILITIES`].
pub const GET_CAPABILITIES: Method = Method {
    name: "GetCapabilities",
    arguments: &[],
    returns: &[Argument {
        name: "capabilities",
        signature: "as",
    }],
    signature: "as",
};

/// `GetServerInformation`, which answers with [`SERVER_INFORMATION`].
pub const GET_SERVER_INFORMATION: Method = Method {
    name: "GetServerInformation",
    arguments: &[],
    returns: &[
        Argument {
            name: "name",
            signature: "s",
        },
        Argument {
            name: "vendor",
            signature: "s",
        },
        Argument {
            name: "version",
            signature: "s",
        },
        Argument {
            name: "spec_version",
            signature: "s",
        },
    ],
    signature: "ssss",
};

/// Every method of the interface, in the order the specification lists them.
pub const METHODS: &[Method] = &[
    NOTIFY,
    CLOSE_NOTIFICATION,
    GET_CAPABILITIES,
    GET_SERVER_INFORMATION,
];

/// The signal that tells a client its notification is gone, and why.
pub const NOTIFICATION_CLOSED: Signal = Signal {
    name: "NotificationClosed",
    arguments: &[
        Argument {
            name: "id",
            signature: "u",
        },
        Argument {
            name: "reason",
            signature: "u",
        },
    ],
    signature: "uu",
};

/// The signal that tells a client one of its actions was invoked.
pub const ACTION_INVOKED: Signal = Signal {
    name: "ActionInvoked",
    arguments: &[
        Argument {
            name: "id",
            signature: "u",
        },
        Argument {
            name: "action_key",
            signature: "s",
        },
    ],
    signature: "us",
};

/// Every signal of the interface.
pub const SIGNALS: &[Signal] = &[NOTIFICATION_CLOSED, ACTION_INVOKED];

/// The capabilities Inox claims, sorted, as `GetCapabilities` returns them.
///
/// See the module documentation for the rule this list is written to, and for
/// the capabilities that are deliberately absent.
pub const CAPABILITIES: &[&str] = &["actions", "body", "body-hyperlinks", "body-markup"];

/// The hint key carrying the notification's urgency.
pub const HINT_URGENCY: &str = "urgency";

/// The hint key carrying the notification's category.
pub const HINT_CATEGORY: &str = "category";

/// The hint key naming the application the notification came from.
pub const HINT_DESKTOP_ENTRY: &str = "desktop-entry";

/// The hint key carrying an image the notification refers to.
pub const HINT_IMAGE_PATH: &str = "image-path";

/// The spelling older clients still use for [`HINT_IMAGE_PATH`].
pub const HINT_IMAGE_PATH_DEPRECATED: &str = "image_path";

/// The hint key asking the daemon to keep the notification after an action.
pub const HINT_RESIDENT: &str = "resident";

/// The hint key marking a notification that is about something happening now.
pub const HINT_TRANSIENT: &str = "transient";

/// The hint key naming a sound to play.
pub const HINT_SOUND_NAME: &str = "sound-name";

/// The hint key asking the daemon not to play a sound.
pub const HINT_SUPPRESS_SOUND: &str = "suppress-sound";

/// The standard hint keys, sorted.
pub const HINT_KEYS: &[&str] = &[
    HINT_CATEGORY,
    HINT_DESKTOP_ENTRY,
    HINT_IMAGE_PATH,
    HINT_RESIDENT,
    HINT_SOUND_NAME,
    HINT_SUPPRESS_SOUND,
    HINT_TRANSIENT,
    HINT_URGENCY,
];

/// Why a notification is gone, as `NotificationClosed` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloseReason {
    /// It reached the end of its lifetime.
    Expired,
    /// The user dismissed it.
    Dismissed,
    /// A client called `CloseNotification`.
    ClosedByCall,
    /// Something else: the specification defines no reason for it.
    ///
    /// Inox uses this for a notification a full queue evicted or coalesced
    /// away; the queue's own counters say which of the two it was.
    Undefined,
}

impl CloseReason {
    /// The `reason` value the wire carries.
    pub const fn code(self) -> u32 {
        match self {
            Self::Expired => 1,
            Self::Dismissed => 2,
            Self::ClosedByCall => 3,
            Self::Undefined => 4,
        }
    }

    /// The reason a wire value names.
    ///
    /// A value the specification does not define reads as
    /// [`CloseReason::Undefined`] rather than being rejected: a daemon that
    /// refused to answer an unknown code would leave the client waiting for a
    /// signal it will never get.
    pub const fn from_code(code: u32) -> Self {
        match code {
            1 => Self::Expired,
            2 => Self::Dismissed,
            3 => Self::ClosedByCall,
            _ => Self::Undefined,
        }
    }
}

/// How urgent a notification says it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Urgency {
    /// The `urgency` hint value `0`: information the user can read later.
    Low,
    /// The `urgency` hint value `1`, and the value a notification without the
    /// hint has.
    #[default]
    Normal,
    /// The `urgency` hint value `2`: something the user has to see, which never
    /// expires on its own (see [`crate::queue`]).
    Critical,
}

impl Urgency {
    /// The `urgency` hint value.
    pub const fn raw(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Normal => 1,
            Self::Critical => 2,
        }
    }

    /// The urgency a hint value names.
    ///
    /// Values above `2` read as [`Urgency::Normal`]: the hint is a hint, and a
    /// daemon that rejected a notification for carrying an unknown urgency would
    /// lose the notification entirely.
    pub const fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Low,
            2 => Self::Critical,
            _ => Self::Normal,
        }
    }
}

/// What a client's `expire_timeout` asked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Expiry {
    /// `-1`: the daemon's configured lifetime for the urgency applies.
    #[default]
    ServerDefault,
    /// `0`: the notification never expires on its own.
    Never,
    /// A positive number of milliseconds.
    After(u32),
}

impl Expiry {
    /// What the wire value asks for.
    ///
    /// Every negative value other than `-1` reads as
    /// [`Expiry::ServerDefault`]: only `-1` and `0` mean something to the
    /// specification, and "let the server decide" is the meaning that cannot
    /// leave a notification on screen forever by accident.
    pub const fn from_wire(expire_timeout: i32) -> Self {
        match expire_timeout {
            EXPIRETIMEOUT_NEVER => Self::Never,
            value if value > 0 => Self::After(value as u32),
            _ => Self::ServerDefault,
        }
    }

    /// The wire value this reads back as.
    pub const fn to_wire(self) -> i32 {
        match self {
            Self::ServerDefault => EXPIRETIMEOUT_SERVER_DEFAULT,
            Self::Never => EXPIRETIMEOUT_NEVER,
            Self::After(milliseconds) => milliseconds as i32,
        }
    }
}

/// A hint value, in the encodings this daemon understands.
///
/// D-Bus hints are a variant table, so a client may send a boolean as a boolean
/// (`b`) or, as several real clients do, as an integer. The accessors below
/// accept the alternates rather than dropping a hint that a client spelled
/// differently from the specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintValue {
    /// A byte (`y`).
    Byte(u8),
    /// A boolean (`b`).
    Boolean(bool),
    /// A signed 32-bit integer (`i`).
    Int(i32),
    /// An unsigned 32-bit integer (`u`).
    Unsigned(u32),
    /// A string (`s`).
    Text(String),
}

impl HintValue {
    /// The value as a boolean, in whichever encoding it arrived.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            Self::Byte(0) | Self::Int(0) | Self::Unsigned(0) => Some(false),
            Self::Byte(1) | Self::Int(1) | Self::Unsigned(1) => Some(true),
            Self::Byte(_) | Self::Int(_) | Self::Unsigned(_) | Self::Text(_) => None,
        }
    }

    /// The value as an urgency byte, in whichever encoding it arrived.
    pub fn as_byte(&self) -> Option<u8> {
        match self {
            Self::Byte(value) => Some(*value),
            Self::Int(value) => u8::try_from(*value).ok(),
            Self::Unsigned(value) => u8::try_from(*value).ok(),
            Self::Boolean(_) | Self::Text(_) => None,
        }
    }

    /// The value as text.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            Self::Byte(_) | Self::Boolean(_) | Self::Int(_) | Self::Unsigned(_) => None,
        }
    }
}

/// The hints of one `Notify` call, as far as Inox reads them.
///
/// Only the standard keys of [`HINT_KEYS`] (plus the deprecated
/// [`HINT_IMAGE_PATH_DEPRECATED`] spelling) are read; everything else is
/// ignored, as the specification requires, and counted in [`Hints::ignored`] so
/// that "ignored" is a number a test can assert rather than a claim.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hints {
    /// How urgent the notification says it is.
    pub urgency: Urgency,
    /// The notification's category, such as `device` or `im.received`.
    pub category: Option<String>,
    /// The `.desktop` file the application is identified by.
    pub desktop_entry: Option<String>,
    /// An image the notification refers to: a path, or a file-like URI.
    pub image_path: Option<String>,
    /// Whether the daemon should keep the notification after an action.
    ///
    /// Recorded, not acted on: this daemon closes a notification when one of its
    /// actions is invoked (see [`crate::actions`]), which is what a resident
    /// notification asks it not to do — so `persistence` is not in
    /// [`CAPABILITIES`].
    pub resident: bool,
    /// Whether the notification is about something happening right now.
    ///
    /// Recorded, not acted on: there is no history to keep it out of yet.
    pub transient: bool,
    /// The sound the client would like played.
    pub sound_name: Option<String>,
    /// Whether the client asked for silence.
    pub suppress_sound: bool,
    /// How many pairs were not a standard hint key.
    pub ignored: usize,
}

impl Hints {
    /// Reads the standard hints out of the pairs a `Notify` call carried.
    pub fn from_pairs(pairs: &[(String, HintValue)]) -> Self {
        let mut hints = Self::default();

        for (key, value) in pairs {
            match key.as_str() {
                HINT_URGENCY => {
                    if let Some(raw) = value.as_byte() {
                        hints.urgency = Urgency::from_raw(raw);
                    }
                }
                HINT_CATEGORY => hints.category = value.as_text().map(str::to_owned),
                HINT_DESKTOP_ENTRY => hints.desktop_entry = value.as_text().map(str::to_owned),
                HINT_IMAGE_PATH | HINT_IMAGE_PATH_DEPRECATED => {
                    if let Some(path) = value.as_text() {
                        hints.image_path = Some(path.to_owned());
                    }
                }
                HINT_RESIDENT => hints.resident = value.as_bool().unwrap_or(false),
                HINT_TRANSIENT => hints.transient = value.as_bool().unwrap_or(false),
                HINT_SOUND_NAME => hints.sound_name = value.as_text().map(str::to_owned),
                HINT_SUPPRESS_SOUND => hints.suppress_sound = value.as_bool().unwrap_or(false),
                _ => hints.ignored += 1,
            }
        }

        hints
    }

    /// The image the notification refers to, preferring `image-path` and
    /// falling back to the deprecated `image_path` spelling.
    pub fn icon_reference(&self) -> Option<&str> {
        self.image_path.as_deref()
    }
}

/// Whether `name` is a capability this daemon claims.
pub fn claims(name: &str) -> bool {
    CAPABILITIES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bus_name_object_path_and_interface_are_the_ones_clients_use() {
        assert_eq!(BUS_NAME, "org.freedesktop.Notifications");
        assert_eq!(OBJECT_PATH, "/org/freedesktop/Notifications");
        assert_eq!(INTERFACE, "org.freedesktop.Notifications");
    }

    #[test]
    fn every_method_spells_its_own_signature_out() {
        for method in METHODS {
            assert!(
                method.signature_is_consistent(),
                "`{}` declares `{}`, which its arguments do not spell",
                method.name,
                method.signature
            );
        }

        for signal in SIGNALS {
            assert!(
                signal.signature_is_consistent(),
                "`{}` declares `{}`, which its arguments do not spell",
                signal.name,
                signal.signature
            );
        }
    }

    #[test]
    fn notify_carries_the_eight_arguments_of_the_specification_in_order() {
        let names: Vec<&str> = NOTIFY
            .arguments
            .iter()
            .map(|argument| argument.name)
            .collect();

        assert_eq!(
            names,
            [
                "app_name",
                "replaces_id",
                "app_icon",
                "summary",
                "body",
                "actions",
                "hints",
                "expire_timeout",
            ]
        );
        assert_eq!(NOTIFY.signature, "susssasa{sv}iu");
        assert_eq!(NOTIFY.arity(), 8);
        assert_eq!(NOTIFY.returns.len(), 1, "Notify answers with the id");
        assert_eq!(NOTIFY.returns[0].name, "id");
        assert_eq!(NOTIFY.returns[0].signature, "u");
        assert_eq!(
            NOTIFY.argument("replaces_id").map(|a| a.signature),
            Some("u")
        );
        assert!(NOTIFY.argument("unknown").is_none());
    }

    #[test]
    fn close_notification_takes_one_id_and_returns_nothing() {
        assert_eq!(CLOSE_NOTIFICATION.signature, "u");
        assert_eq!(CLOSE_NOTIFICATION.arity(), 1);
        assert!(CLOSE_NOTIFICATION.returns.is_empty());
    }

    #[test]
    fn capabilities_are_answered_as_a_string_array() {
        assert_eq!(GET_CAPABILITIES.signature, "as");
        assert_eq!(GET_CAPABILITIES.arity(), 0);
        assert_eq!(GET_CAPABILITIES.returns[0].signature, "as");
    }

    #[test]
    fn server_information_answers_with_four_strings() {
        assert_eq!(GET_SERVER_INFORMATION.signature, "ssss");
        let names: Vec<&str> = GET_SERVER_INFORMATION
            .returns
            .iter()
            .map(|argument| argument.name)
            .collect();

        assert_eq!(names, ["name", "vendor", "version", "spec_version"]);
    }

    #[test]
    fn the_signals_carry_the_id_and_what_changed() {
        assert_eq!(NOTIFICATION_CLOSED.signature, "uu");
        assert_eq!(ACTION_INVOKED.signature, "us");
        assert_eq!(ACTION_INVOKED.arguments[1].name, "action_key");
        assert!(
            !METHODS
                .iter()
                .any(|method| method.name == ACTION_INVOKED.name),
            "`ActionInvoked` is a signal, never a method a client can call"
        );
        assert!(
            !SIGNALS.iter().any(|signal| signal.name == NOTIFY.name),
            "`Notify` is a method, never a signal"
        );
    }

    #[test]
    fn the_daemon_reports_its_own_version_and_the_specification_it_follows() {
        assert_eq!(SERVER_INFORMATION.name, "Inox");
        assert_eq!(SERVER_INFORMATION.vendor, "Inox Desktop");
        assert_eq!(SERVER_INFORMATION.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(SERVER_INFORMATION.spec_version, "1.2");
    }

    #[test]
    fn only_capabilities_this_daemon_can_present_are_claimed() {
        let mut claimed = CAPABILITIES.to_vec();
        claimed.sort_unstable();
        assert_eq!(claimed, CAPABILITIES, "the list is returned sorted");
        assert!(!CAPABILITIES.contains(&""));

        for capability in CAPABILITIES {
            assert!(claims(capability), "`{capability}` is in the list");
        }

        // The ones this milestone cannot back up, each for a reason the module
        // documentation gives: no image decoding, no sound playback, no inline
        // replies, and no notification kept after an action is invoked.
        for unclaimed in [
            "action-icons",
            "body-images",
            "icon-multi",
            "icon-static",
            "inline-reply",
            "persistence",
            "sound",
        ] {
            assert!(
                !claims(unclaimed),
                "`{unclaimed}` is claimed but not implemented"
            );
        }

        assert!(!claims("nonsense"));
    }

    #[test]
    fn close_reasons_are_the_four_the_specification_defines() {
        assert_eq!(CloseReason::Expired.code(), 1);
        assert_eq!(CloseReason::Dismissed.code(), 2);
        assert_eq!(CloseReason::ClosedByCall.code(), 3);
        assert_eq!(CloseReason::Undefined.code(), 4);

        for reason in [
            CloseReason::Expired,
            CloseReason::Dismissed,
            CloseReason::ClosedByCall,
            CloseReason::Undefined,
        ] {
            assert_eq!(CloseReason::from_code(reason.code()), reason);
        }
    }

    #[test]
    fn an_unknown_close_reason_is_undefined_rather_than_an_error() {
        assert_eq!(CloseReason::from_code(0), CloseReason::Undefined);
        assert_eq!(CloseReason::from_code(5), CloseReason::Undefined);
        assert_eq!(CloseReason::from_code(u32::MAX), CloseReason::Undefined);
    }

    #[test]
    fn urgency_reads_back_the_value_it_was_written_as() {
        assert_eq!(Urgency::Low.raw(), 0);
        assert_eq!(Urgency::Normal.raw(), 1);
        assert_eq!(Urgency::Critical.raw(), 2);

        for urgency in [Urgency::Low, Urgency::Normal, Urgency::Critical] {
            assert_eq!(Urgency::from_raw(urgency.raw()), urgency);
        }
    }

    #[test]
    fn an_unknown_urgency_is_normal_rather_than_a_lost_notification() {
        assert_eq!(Urgency::from_raw(3), Urgency::Normal);
        assert_eq!(Urgency::from_raw(u8::MAX), Urgency::Normal);
        assert_eq!(Urgency::default(), Urgency::Normal);
    }

    #[test]
    fn expire_timeout_reads_the_three_meanings_the_specification_defines() {
        assert_eq!(Expiry::from_wire(-1), Expiry::ServerDefault);
        assert_eq!(Expiry::from_wire(0), Expiry::Never);
        assert_eq!(Expiry::from_wire(5_000), Expiry::After(5_000));
        assert_eq!(Expiry::from_wire(1), Expiry::After(1));

        for expiry in [
            Expiry::ServerDefault,
            Expiry::Never,
            Expiry::After(1),
            Expiry::After(600_000),
        ] {
            assert_eq!(Expiry::from_wire(expiry.to_wire()), expiry);
        }
    }

    #[test]
    fn a_negative_expire_timeout_other_than_minus_one_lets_the_server_decide() {
        assert_eq!(Expiry::from_wire(-2), Expiry::ServerDefault);
        assert_eq!(Expiry::from_wire(i32::MIN), Expiry::ServerDefault);
    }

    #[test]
    fn the_standard_hints_are_read_into_their_fields() {
        let hints = Hints::from_pairs(&[
            (HINT_URGENCY.to_owned(), HintValue::Byte(2)),
            (HINT_CATEGORY.to_owned(), HintValue::Text("im".to_owned())),
            (
                HINT_DESKTOP_ENTRY.to_owned(),
                HintValue::Text("firefox.desktop".to_owned()),
            ),
            (
                HINT_IMAGE_PATH.to_owned(),
                HintValue::Text("/usr/share/icons/x.png".to_owned()),
            ),
            (HINT_RESIDENT.to_owned(), HintValue::Boolean(true)),
            (HINT_TRANSIENT.to_owned(), HintValue::Boolean(true)),
            (
                HINT_SOUND_NAME.to_owned(),
                HintValue::Text("message-new-instant".to_owned()),
            ),
            (HINT_SUPPRESS_SOUND.to_owned(), HintValue::Boolean(true)),
        ]);

        assert_eq!(hints.urgency, Urgency::Critical);
        assert_eq!(hints.category.as_deref(), Some("im"));
        assert_eq!(hints.desktop_entry.as_deref(), Some("firefox.desktop"));
        assert_eq!(hints.icon_reference(), Some("/usr/share/icons/x.png"));
        assert!(hints.resident);
        assert!(hints.transient);
        assert_eq!(hints.sound_name.as_deref(), Some("message-new-instant"));
        assert!(hints.suppress_sound);
        assert_eq!(hints.ignored, 0);
    }

    #[test]
    fn a_notification_without_hints_is_a_normal_one() {
        let hints = Hints::from_pairs(&[]);

        assert_eq!(hints, Hints::default());
        assert_eq!(hints.urgency, Urgency::Normal);
        assert!(hints.icon_reference().is_none());
        assert!(!hints.resident);
        assert!(!hints.suppress_sound);
    }

    #[test]
    fn unknown_hints_are_ignored_and_counted() {
        let hints = Hints::from_pairs(&[
            (
                "x-inox-nonsense".to_owned(),
                HintValue::Text("x".to_owned()),
            ),
            ("another".to_owned(), HintValue::Byte(9)),
        ]);

        assert_eq!(
            hints,
            Hints {
                ignored: 2,
                ..Hints::default()
            }
        );
    }

    #[test]
    fn a_hint_spelled_in_another_encoding_is_still_read() {
        let hints = Hints::from_pairs(&[
            (HINT_RESIDENT.to_owned(), HintValue::Int(1)),
            (HINT_TRANSIENT.to_owned(), HintValue::Unsigned(0)),
            (HINT_URGENCY.to_owned(), HintValue::Int(0)),
            (HINT_SUPPRESS_SOUND.to_owned(), HintValue::Byte(1)),
        ]);

        assert!(hints.resident, "1 as an integer means true");
        assert!(!hints.transient);
        assert_eq!(hints.urgency, Urgency::Low);
        assert!(hints.suppress_sound);
        assert_eq!(hints.ignored, 0);
    }

    #[test]
    fn a_hint_of_the_wrong_type_is_ignored_rather_than_guessed() {
        let hints = Hints::from_pairs(&[
            (HINT_URGENCY.to_owned(), HintValue::Text("high".to_owned())),
            (HINT_RESIDENT.to_owned(), HintValue::Int(7)),
            (HINT_CATEGORY.to_owned(), HintValue::Boolean(true)),
            (HINT_IMAGE_PATH.to_owned(), HintValue::Byte(3)),
        ]);

        assert_eq!(hints.urgency, Urgency::Normal);
        assert!(!hints.resident);
        assert!(hints.category.is_none());
        assert!(hints.icon_reference().is_none());
        assert_eq!(hints.ignored, 0, "they were standard keys, just unusable");
    }

    #[test]
    fn the_deprecated_image_path_spelling_is_accepted() {
        let hints = Hints::from_pairs(&[(
            HINT_IMAGE_PATH_DEPRECATED.to_owned(),
            HintValue::Text("/tmp/a.png".to_owned()),
        )]);

        assert_eq!(hints.icon_reference(), Some("/tmp/a.png"));
    }

    #[test]
    fn the_standard_hint_keys_are_the_eight_the_specification_names() {
        let mut keys = HINT_KEYS.to_vec();
        keys.sort_unstable();
        assert_eq!(keys, HINT_KEYS);

        assert_eq!(
            HINT_KEYS,
            [
                "category",
                "desktop-entry",
                "image-path",
                "resident",
                "sound-name",
                "suppress-sound",
                "transient",
                "urgency",
            ]
        );
        assert!(
            !HINT_KEYS.contains(&HINT_IMAGE_PATH_DEPRECATED),
            "the deprecated spelling is an alias, not a standard key"
        );
    }

    #[test]
    fn the_default_action_key_is_the_reserved_one() {
        assert_eq!(DEFAULT_ACTION_KEY, "default");
    }
}
