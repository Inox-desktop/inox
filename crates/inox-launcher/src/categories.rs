//! The categories the launcher can browse by.
//!
//! A desktop entry's `Categories=` is the freedesktop menu taxonomy: a long,
//! overlapping list ("Utility;TextEditor;GTK;") where one application can be in
//! several places and half the values are toolkit bookkeeping. The launcher does
//! not reproduce that tree; it offers the small fixed row of chips a person can
//! actually scan, and maps each real category onto one of them.
//!
//! The set and its order are data ([`CATEGORIES`]), the mapping is a table
//! ([`Category::from_freedesktop`]) and the filter is one predicate
//! ([`Category::matches`]). `crate::view_model` decides which chips exist from
//! the index, `crate::app` decides what the selection filters, and the labels
//! come from Fluent — nothing here reads the environment, the clock or the
//! window.

use serde::Deserialize;

/// One chip in the launcher's category row.
///
/// [`Category::All`] is the unfiltered browse, and [`Category::Other`] is where
/// an application lands when none of its `Categories=` values is one the table
/// below recognises — which is common: a desktop file may declare only `GTK;`
/// or nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Every application.
    All,
    /// Programming tools.
    Development,
    /// Image, vector and photo tools.
    Graphics,
    /// Browsers, mail, chat and file transfer.
    Internet,
    /// Documents, spreadsheets and the rest of an office suite.
    Office,
    /// Players, recorders and the rest of the media stack.
    AudioVideo,
    /// Games and the emulators that run them.
    Game,
    /// Small tools: editors, terminals, archivers, calculators.
    Utility,
    /// Settings, monitoring and package management.
    System,
    /// Teaching and learning.
    Education,
    /// Scientific software.
    Science,
    /// Nothing above recognised the entry.
    Other,
}

/// The chips, in the order they are drawn.
///
/// Fixed and ordered rather than discovered, so the row does not reshuffle when
/// an application is installed: [`Category::All`] is first because it is the way
/// back, and the rest follow the order a person scans a menu in.
pub const CATEGORIES: [Category; 12] = [
    Category::All,
    Category::Development,
    Category::Graphics,
    Category::Internet,
    Category::Office,
    Category::AudioVideo,
    Category::Game,
    Category::Utility,
    Category::System,
    Category::Education,
    Category::Science,
    Category::Other,
];

impl Category {
    /// The stable identifier the projection and the action channel use.
    ///
    /// Deliberately not the Fluent id: `crate::catalog_tests` reads every
    /// `launcher-…` literal in the sources as a message the code asks for, and
    /// an interface identifier is not one.
    pub const fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Development => "development",
            Self::Graphics => "graphics",
            Self::Internet => "internet",
            Self::Office => "office",
            Self::AudioVideo => "audio-video",
            Self::Game => "game",
            Self::Utility => "utility",
            Self::System => "system",
            Self::Education => "education",
            Self::Science => "science",
            Self::Other => "other",
        }
    }

    /// The Fluent id of the chip's label.
    pub const fn label_id(self) -> &'static str {
        match self {
            Self::All => "launcher-category-all",
            Self::Development => "launcher-category-development",
            Self::Graphics => "launcher-category-graphics",
            Self::Internet => "launcher-category-internet",
            Self::Office => "launcher-category-office",
            Self::AudioVideo => "launcher-category-audio-video",
            Self::Game => "launcher-category-game",
            Self::Utility => "launcher-category-utility",
            Self::System => "launcher-category-system",
            Self::Education => "launcher-category-education",
            Self::Science => "launcher-category-science",
            Self::Other => "launcher-category-other",
        }
    }

    /// Whether an entry with these `Categories=` belongs to this chip.
    ///
    /// An entry can be in several chips at once (`Development;IDE;` is
    /// [`Category::Development`] only because both map there), and
    /// [`Category::Other`] means "nothing in the table matched", not "the entry
    /// has no categories": a desktop file that says `GTK;` is in Other.
    pub fn matches(self, categories: &[String]) -> bool {
        match self {
            Self::All => true,
            Self::Other => !categories.iter().any(|value| {
                Self::from_freedesktop(value).is_some_and(|category| category != Self::Other)
            }),
            wanted => categories
                .iter()
                .any(|value| Self::from_freedesktop(value) == Some(wanted)),
        }
    }

    /// The chip a single freedesktop category name maps to.
    ///
    /// `None` for a name the table does not know, and never [`Category::All`] or
    /// [`Category::Other`]: those two are the row's own bookkeeping, not values a
    /// desktop file can name.
    pub fn from_freedesktop(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        // Freedesktop category names are CamelCase ASCII; an entry that spells
        // one in another case is still the same category.
        let mut normalized = value.to_owned();
        normalized.make_ascii_lowercase();

        Some(match normalized.as_str() {
            // Development
            "development" | "ide" | "debugger" | "building" | "guigendesigner" | "guidesigner"
            | "profiling" | "revisioncontrol" | "translation" | "webdevelopment" | "database" => {
                Self::Development
            }

            // Graphics
            "graphics" | "2dgraphics" | "3dgraphics" | "vectorgraphics" | "rastergraphics"
            | "photography" | "publishing" | "scanning" | "viewer" | "imageprocessing" => {
                Self::Graphics
            }

            // Internet
            "network" | "email" | "webbrowser" | "instantmessaging" | "chat" | "filetransfer"
            | "news" | "p2p" | "remoteaccess" | "telephony" | "videoconference" | "feed"
            | "irc" => Self::Internet,

            // Office
            "office" | "spreadsheet" | "wordprocessor" | "presentation" | "calendar"
            | "finance" | "contactmanagement" | "projectmanagement" | "flowchart" => Self::Office,

            // Audio and video
            "audiovideo" | "audio" | "video" | "player" | "recorder" | "midi" | "mixer"
            | "sequencer" | "tuner" | "tv" => Self::AudioVideo,

            // Games
            "game" | "actiongame" | "adventuregame" | "arcadegame" | "boardgame" | "blocksgame"
            | "cardgame" | "kidsgame" | "logicgame" | "roleplaying" | "shooter" | "simulation"
            | "sportsgame" | "strategygame" | "emulator" => Self::Game,

            // Utility
            "utility" | "texteditor" | "texttools" | "calculator" | "archiving" | "compression"
            | "filemanager" | "terminalemulator" | "accessibility" | "clock" | "timeutility"
            | "documentation" | "dictionary" | "spellchecking" | "printing" => Self::Utility,

            // System
            "system" | "settings" | "packagemanager" | "security" | "hardwaresettings"
            | "desktopsettings" | "loginmanager" | "screensaver" | "session" | "filetools"
            | "monitor" | "powermanagement" => Self::System,

            // Education
            "education" | "arts" | "languages" | "mathematics" | "teaching" | "training"
            | "geography" | "history" | "literature" => Self::Education,

            // Science
            "science" | "astronomy" | "biology" | "chemistry" | "computerscience"
            | "datavisualization" | "electricity" | "geology" | "math" | "medicalsoftware"
            | "numericalanalysis" | "physics" | "robotics" => Self::Science,

            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_chip_has_its_own_id_and_label() {
        let mut ids: Vec<&str> = CATEGORIES.iter().map(|category| category.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATEGORIES.len(), "two chips share an id");

        let mut labels: Vec<&str> = CATEGORIES
            .iter()
            .map(|category| category.label_id())
            .collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), CATEGORIES.len(), "two chips share a label id");

        assert_eq!(CATEGORIES[0], Category::All, "all is the way back");
        assert_eq!(CATEGORIES[CATEGORIES.len() - 1], Category::Other);
    }

    #[test]
    fn the_freedesktop_categories_map_onto_the_row() {
        assert_eq!(
            Category::from_freedesktop("Development"),
            Some(Category::Development)
        );
        assert_eq!(
            Category::from_freedesktop("IDE"),
            Some(Category::Development)
        );
        assert_eq!(
            Category::from_freedesktop("TextEditor"),
            Some(Category::Utility)
        );
        assert_eq!(
            Category::from_freedesktop("Network"),
            Some(Category::Internet)
        );
        assert_eq!(
            Category::from_freedesktop("WebBrowser"),
            Some(Category::Internet)
        );
        assert_eq!(
            Category::from_freedesktop("Audio"),
            Some(Category::AudioVideo)
        );
        assert_eq!(Category::from_freedesktop("Game"), Some(Category::Game));
        assert_eq!(
            Category::from_freedesktop("Settings"),
            Some(Category::System)
        );
        assert_eq!(
            Category::from_freedesktop("Science"),
            Some(Category::Science)
        );
        assert_eq!(
            Category::from_freedesktop("Education"),
            Some(Category::Education)
        );

        // Case and spacing are the file's business, not the category's.
        assert_eq!(
            Category::from_freedesktop(" webbrowser "),
            Some(Category::Internet)
        );

        // Toolkit and desktop bookkeeping is not a category.
        for unknown in ["GTK", "Qt", "KDE", "Application", "X-Red-Hat-Base", ""] {
            assert_eq!(Category::from_freedesktop(unknown), None, "{unknown}");
        }
        // A desktop file can never name the row's own bookkeeping.
        for value in ["All", "Other"] {
            assert_eq!(
                Category::from_freedesktop(value),
                None,
                "{value} is a chip, not a desktop category",
            );
        }
    }

    #[test]
    fn an_entry_can_belong_to_several_chips() {
        let categories = vec!["Development".to_owned(), "IDE".to_owned()];
        assert!(Category::Development.matches(&categories));
        assert!(Category::All.matches(&categories));
        assert!(!Category::Game.matches(&categories));
        assert!(!Category::Other.matches(&categories));
    }

    #[test]
    fn other_is_for_entries_no_chip_recognises() {
        let toolkit_only = vec!["GTK".to_owned(), "Application".to_owned()];
        assert!(Category::Other.matches(&toolkit_only));
        assert!(Category::All.matches(&toolkit_only));
        assert!(!Category::Utility.matches(&toolkit_only));

        let none = Vec::new();
        assert!(Category::Other.matches(&none), "no categories is other");

        let recognised = vec!["Utility".to_owned()];
        assert!(!Category::Other.matches(&recognised));
        assert!(Category::Utility.matches(&recognised));
    }

    #[test]
    fn the_action_channel_spells_the_categories_in_kebab_case() {
        let parsed: Category = serde_json::from_str("\"audio-video\"").expect("a category");
        assert_eq!(parsed, Category::AudioVideo);
        let parsed: Category = serde_json::from_str("\"all\"").expect("a category");
        assert_eq!(parsed, Category::All);
        assert!(serde_json::from_str::<Category>("\"AudioVideo\"").is_err());
        assert!(serde_json::from_str::<Category>("\"nonsense\"").is_err());
    }
}
