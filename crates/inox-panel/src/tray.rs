//! The tray model: StatusNotifierItems and their D-Bus menus.
//!
//! A tray application registers a StatusNotifierItem with the session's status
//! notifier watcher under a D-Bus name and an object path, then updates its
//! title, icon, status and menu as it runs, and unregisters when it exits. The
//! panel is the *presentation* half of that protocol ("订阅窗口状态；
//! StatusNotifierItem/DBusMenu ... 呈现"), which is what this module models: the
//! item set, the registration lifecycle, the menu tree, and the reconnect that
//! keeps the panel correct when the watcher restarts.
//!
//! **The D-Bus transport is not part of this module, and not implemented in this
//! milestone.** Nothing here opens a bus, owns a name, emits a signal or decodes
//! a variant: this is the data the transport will feed, so the presentation rules
//! — ordering, deduplication, what an unknown item does, what a restart rebuilds
//! — are testable today, without a bus in CI. `main.rs` says as much at startup
//! rather than pretending the panel serves a tray it cannot reach.
//!
//! Two rules from ADR 0013 shape the API:
//!
//! - **An event for an item that is not registered is rejected, not applied.**
//!   A status update that overtakes a registration, or arrives after an exit, is
//!   [`TrayError::UnknownItem`] — the panel does not invent an entry from a
//!   stray signal.
//! - **A restart resynchronises from a snapshot.** After the watcher restarts,
//!   the panel rebuilds the whole item set from what the new watcher reports, so
//!   the same item can never be listed twice and an item that is gone really is
//!   gone (acceptance D03: "重连后由快照重建，不重复条目").

use std::collections::BTreeMap;
use std::fmt;

/// The registration of one status notifier item.
///
/// The identity is the pair the protocol gives: the D-Bus name the application
/// owns and the object path it serves. Not the title, and not the icon — those
/// the application changes while it runs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrayItemId {
    /// Well-known or unique bus name, such as `:1.42`.
    pub bus_name: String,
    /// Object path of the item, such as `/StatusNotifierItem`.
    pub object_path: String,
}

impl TrayItemId {
    /// The identity of the item `bus_name` serves at `object_path`.
    pub fn new(bus_name: impl Into<String>, object_path: impl Into<String>) -> Self {
        Self {
            bus_name: bus_name.into(),
            object_path: object_path.into(),
        }
    }
}

impl fmt::Display for TrayItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.bus_name, self.object_path)
    }
}

/// The status an item reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum TrayStatus {
    /// The item is not showing anything interesting; a panel may hide it.
    Passive,
    /// The item is active and its icon belongs on the tray.
    #[default]
    Active,
    /// The item wants the user's attention.
    NeedsAttention,
}

/// The identifier the D-Bus menu uses to trigger one of its entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MenuItemId(u32);

impl MenuItemId {
    /// The identifier a menu reported.
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw identifier, to send back when the entry is activated.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// One entry of a D-Bus menu.
///
/// The tree is modelled as it is consumed: a label, whether the entry can be
/// activated at all, separators (which carry nothing), and submenus (which are
/// the same tree one level down).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuItem {
    /// A selectable entry.
    Action {
        /// Identifier to trigger it with.
        id: MenuItemId,
        /// Label the menu shows.
        label: String,
        /// Whether the entry can be selected.
        enabled: bool,
    },
    /// An entry that opens further entries.
    Submenu {
        /// Identifier of the submenu entry itself.
        id: MenuItemId,
        /// Label the menu shows.
        label: String,
        /// Whether the entry can be selected.
        enabled: bool,
        /// The entries it opens.
        children: Vec<MenuItem>,
    },
    /// A non-selectable divider.
    Separator,
}

impl MenuItem {
    /// The identifier of a selectable entry; `None` for a separator.
    pub const fn id(&self) -> Option<MenuItemId> {
        match self {
            Self::Action { id, .. } | Self::Submenu { id, .. } => Some(*id),
            Self::Separator => None,
        }
    }

    /// The label of a selectable entry; `None` for a separator.
    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Action { label, .. } | Self::Submenu { label, .. } => Some(label),
            Self::Separator => None,
        }
    }

    /// Whether the entry can be selected. A separator never can.
    pub const fn is_enabled(&self) -> bool {
        match self {
            Self::Action { enabled, .. } | Self::Submenu { enabled, .. } => *enabled,
            Self::Separator => false,
        }
    }

    /// Whether this entry is a separator.
    pub const fn is_separator(&self) -> bool {
        matches!(self, Self::Separator)
    }

    /// The entry with `id`, searching submenus depth-first.
    ///
    /// This is what an activation has to resolve before it can put an identifier
    /// on the bus, and what makes a disabled entry un-activatable.
    pub fn find(&self, id: MenuItemId) -> Option<&Self> {
        if self.id() == Some(id) {
            return Some(self);
        }

        match self {
            Self::Submenu { children, .. } => children.iter().find_map(|child| child.find(id)),
            Self::Action { .. } | Self::Separator => None,
        }
    }

    /// Whether the entry with `id` exists and can be selected.
    pub fn is_selectable(&self, id: MenuItemId) -> bool {
        self.find(id).map(Self::is_enabled).unwrap_or(false)
    }

    /// Every identifier in the tree, in depth-first order.
    pub fn ids(&self) -> Vec<MenuItemId> {
        let mut ids = Vec::new();
        self.collect_ids(&mut ids);
        ids
    }

    fn collect_ids(&self, ids: &mut Vec<MenuItemId>) {
        if let Some(id) = self.id() {
            ids.push(id);
        }

        if let Self::Submenu { children, .. } = self {
            for child in children {
                child.collect_ids(ids);
            }
        }
    }
}

/// One status notifier item, as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayItem {
    /// The registration this item belongs to.
    pub id: TrayItemId,
    /// Title the item reports; may be empty, and then the panel falls back to a
    /// localized name rather than showing nothing.
    pub title: String,
    /// Icon name, for a future icon-theme lookup. The panel does not draw icons
    /// yet, and an absent icon is a normal state, not an error.
    pub icon: Option<String>,
    /// Status the item reports.
    pub status: TrayStatus,
    /// Whether the item asked for attention. Kept apart from [`TrayItem::status`]
    /// because an item can be active *and* asking for attention.
    pub attention: bool,
    /// Tooltip the item reports, if any.
    pub tooltip: Option<String>,
    /// The item's menu, top level first.
    pub menu: Vec<MenuItem>,
}

impl TrayItem {
    /// An item that has registered but reported nothing yet.
    pub fn new(id: TrayItemId) -> Self {
        Self {
            id,
            title: String::new(),
            icon: None,
            status: TrayStatus::Active,
            attention: false,
            tooltip: None,
            menu: Vec::new(),
        }
    }

    /// Whether the item should be visible on a tray that follows status.
    ///
    /// An item that is passive *and* not asking for attention is one the
    /// application itself says has nothing to show.
    pub const fn is_visible(&self) -> bool {
        self.attention || !matches!(self.status, TrayStatus::Passive)
    }

    /// Whether the item is asking for the user's attention, by either route the
    /// protocol offers.
    pub const fn wants_attention(&self) -> bool {
        self.attention || matches!(self.status, TrayStatus::NeedsAttention)
    }
}

/// A change an item reported.
///
/// One event per field rather than a whole-item replacement: the protocol sends
/// properties individually, and replacing the item wholesale would make a title
/// change look like a menu change to every consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayItemUpdate {
    /// The item's title changed.
    Title(String),
    /// The item's icon changed; `None` means it has none.
    Icon(Option<String>),
    /// The item's status changed.
    Status(TrayStatus),
    /// Whether the item asks for attention changed.
    Attention(bool),
    /// The item's tooltip changed; `None` means it has none.
    Tooltip(Option<String>),
    /// The item's menu was replaced.
    Menu(Vec<MenuItem>),
}

/// Why a tray event could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrayError {
    /// The event names an item that is not registered.
    ///
    /// A registration that never arrived, an update that overtook it, and an
    /// event for an application that already exited all look like this, and the
    /// panel answers all three the same way: it does not invent an entry.
    #[error("`{id}` is not a registered status notifier item")]
    UnknownItem {
        /// The item the event named.
        id: TrayItemId,
    },
    /// The event names a menu entry the item's menu does not contain.
    #[error("`{id}` has no menu entry {entry:?}")]
    UnknownMenuItem {
        /// The item whose menu was searched.
        id: TrayItemId,
        /// The entry that was not found.
        entry: MenuItemId,
    },
}

/// What a snapshot changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraySnapshot {
    /// Items the snapshot introduced.
    pub added: Vec<TrayItemId>,
    /// Items the snapshot removed because they are gone.
    pub removed: Vec<TrayItemId>,
    /// Items the snapshot changed.
    pub updated: Vec<TrayItemId>,
    /// How many items the snapshot restated unchanged.
    pub unchanged: usize,
}

impl TraySnapshot {
    /// Whether the snapshot left the item set exactly as it was.
    ///
    /// A reconnect that reports a clean snapshot is the idempotent case: the
    /// item set was rebuilt from it without adding or dropping anything.
    pub fn is_idempotent(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.updated.is_empty()
    }
}

/// The status notifier items the session currently has.
#[derive(Debug, Clone, Default)]
pub struct Tray {
    items: BTreeMap<TrayItemId, TrayItem>,
}

impl Tray {
    /// An empty tray.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many items are registered.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether no item is registered.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The item registered under `id`.
    pub fn item(&self, id: &TrayItemId) -> Option<&TrayItem> {
        self.items.get(id)
    }

    /// Whether `id` is registered.
    pub fn contains(&self, id: &TrayItemId) -> bool {
        self.items.contains_key(id)
    }

    /// Every item, ordered by registration identity.
    ///
    /// Ordered by bus name and object path, not by arrival: a tray that
    /// reorders itself whenever an application restarts is unusable.
    pub fn items(&self) -> impl Iterator<Item = &TrayItem> {
        self.items.values()
    }

    /// The items a tray that follows status would show.
    pub fn visible(&self) -> impl Iterator<Item = &TrayItem> {
        self.items.values().filter(|item| item.is_visible())
    }

    /// Registers `id`, returning whether it was new.
    ///
    /// Registering an item that is already registered is not an error and does
    /// not reset it: the protocol lets an application re-announce itself, and the
    /// panel must not lose the state it already has (no duplicated entry either
    /// way).
    pub fn register(&mut self, id: &TrayItemId) -> bool {
        if self.items.contains_key(id) {
            return false;
        }

        self.items.insert(id.clone(), TrayItem::new(id.clone()));
        true
    }

    /// Forgets an item that unregistered or exited, returning what was known.
    pub fn unregister(&mut self, id: &TrayItemId) -> Option<TrayItem> {
        self.items.remove(id)
    }

    /// Applies one property change to a registered item.
    ///
    /// # Errors
    ///
    /// Returns [`TrayError::UnknownItem`] when `id` is not registered.
    pub fn update(&mut self, id: &TrayItemId, update: TrayItemUpdate) -> Result<(), TrayError> {
        let item = self
            .items
            .get_mut(id)
            .ok_or_else(|| TrayError::UnknownItem { id: id.clone() })?;

        match update {
            TrayItemUpdate::Title(title) => item.title = title,
            TrayItemUpdate::Icon(icon) => item.icon = icon,
            TrayItemUpdate::Status(status) => item.status = status,
            TrayItemUpdate::Attention(attention) => item.attention = attention,
            TrayItemUpdate::Tooltip(tooltip) => item.tooltip = tooltip,
            TrayItemUpdate::Menu(menu) => item.menu = menu,
        }

        Ok(())
    }

    /// Resolves a menu entry of `id` that the user selected.
    ///
    /// # Errors
    ///
    /// Returns [`TrayError::UnknownItem`] when the item is not registered and
    /// [`TrayError::UnknownMenuItem`] when its menu has no such entry — an
    /// activation for an entry the application has already withdrawn.
    pub fn menu_entry(&self, id: &TrayItemId, entry: MenuItemId) -> Result<&MenuItem, TrayError> {
        let item = self
            .items
            .get(id)
            .ok_or_else(|| TrayError::UnknownItem { id: id.clone() })?;

        item.menu
            .iter()
            .find_map(|root| root.find(entry))
            .ok_or_else(|| TrayError::UnknownMenuItem {
                id: id.clone(),
                entry,
            })
    }

    /// Rebuilds the item set from a snapshot (reconnect).
    ///
    /// The snapshot is authoritative for **which** items exist and what they
    /// say, so an item it does not mention is dropped — that is how an item whose
    /// application exited while the panel was disconnected disappears — and every
    /// item it does mention is stored exactly once, keyed by its registration.
    /// Applying the same snapshot twice changes nothing the second time.
    pub fn apply_snapshot(&mut self, snapshot: impl IntoIterator<Item = TrayItem>) -> TraySnapshot {
        let snapshot: BTreeMap<TrayItemId, TrayItem> = snapshot
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect();

        let mut diff = TraySnapshot::default();

        let removed: Vec<TrayItemId> = self
            .items
            .keys()
            .filter(|id| !snapshot.contains_key(*id))
            .cloned()
            .collect();
        for id in &removed {
            self.items.remove(id);
        }
        diff.removed = removed;

        for (id, item) in snapshot {
            match self.items.get(&id) {
                Some(known) if *known == item => diff.unchanged += 1,
                Some(_) => {
                    diff.updated.push(id.clone());
                    self.items.insert(id, item);
                }
                None => {
                    diff.added.push(id.clone());
                    self.items.insert(id, item);
                }
            }
        }

        diff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> TrayItemId {
        TrayItemId::new(name, "/StatusNotifierItem")
    }

    fn item(name: &str, title: &str) -> TrayItem {
        TrayItem {
            title: title.to_owned(),
            ..TrayItem::new(id(name))
        }
    }

    fn menu() -> Vec<MenuItem> {
        vec![
            MenuItem::Action {
                id: MenuItemId::new(1),
                label: "Open".to_owned(),
                enabled: true,
            },
            MenuItem::Separator,
            MenuItem::Submenu {
                id: MenuItemId::new(2),
                label: "More".to_owned(),
                enabled: true,
                children: vec![
                    MenuItem::Action {
                        id: MenuItemId::new(3),
                        label: "Details".to_owned(),
                        enabled: false,
                    },
                    MenuItem::Action {
                        id: MenuItemId::new(4),
                        label: "Quit".to_owned(),
                        enabled: true,
                    },
                ],
            },
        ]
    }

    #[test]
    fn registering_twice_does_not_duplicate_an_item() {
        let mut tray = Tray::new();

        assert!(tray.register(&id(":1.1")));
        assert!(!tray.register(&id(":1.1")));
        assert_eq!(tray.len(), 1);

        tray.update(&id(":1.1"), TrayItemUpdate::Title("Mail".to_owned()))
            .expect("the item is registered");
        assert!(
            !tray.register(&id(":1.1")),
            "an item that is already registered is not registered again"
        );
        assert_eq!(
            tray.item(&id(":1.1")).map(|item| item.title.as_str()),
            Some("Mail"),
            "a re-announcement must not reset what the item already reported"
        );
    }

    #[test]
    fn an_update_for_an_unknown_item_is_rejected() {
        let mut tray = Tray::new();

        let error = tray
            .update(&id(":1.9"), TrayItemUpdate::Title("Late".to_owned()))
            .expect_err("the item never registered");

        assert_eq!(error, TrayError::UnknownItem { id: id(":1.9") });
        assert!(tray.is_empty(), "a stray event must not invent an entry");
    }

    #[test]
    fn an_update_after_unregistration_is_rejected() {
        let mut tray = Tray::new();
        tray.register(&id(":1.2"));
        tray.unregister(&id(":1.2"));

        assert!(matches!(
            tray.update(&id(":1.2"), TrayItemUpdate::Status(TrayStatus::Passive)),
            Err(TrayError::UnknownItem { .. })
        ));
    }

    #[test]
    fn every_field_of_an_item_can_be_updated() {
        let mut tray = Tray::new();
        let key = id(":1.3");
        tray.register(&key);

        tray.update(&key, TrayItemUpdate::Title("Music".to_owned()))
            .expect("registered");
        tray.update(&key, TrayItemUpdate::Icon(Some("music".to_owned())))
            .expect("registered");
        tray.update(&key, TrayItemUpdate::Status(TrayStatus::NeedsAttention))
            .expect("registered");
        tray.update(&key, TrayItemUpdate::Attention(true))
            .expect("registered");
        tray.update(&key, TrayItemUpdate::Tooltip(Some("Playing".to_owned())))
            .expect("registered");
        tray.update(&key, TrayItemUpdate::Menu(menu()))
            .expect("registered");

        let item = tray.item(&key).expect("registered");
        assert_eq!(item.title, "Music");
        assert_eq!(item.icon.as_deref(), Some("music"));
        assert_eq!(item.status, TrayStatus::NeedsAttention);
        assert!(item.attention);
        assert_eq!(item.tooltip.as_deref(), Some("Playing"));
        assert_eq!(item.menu.len(), 3);
        assert!(item.wants_attention());
        assert!(item.is_visible());
    }

    #[test]
    fn an_exiting_item_leaves_the_tray() {
        let mut tray = Tray::new();
        tray.register(&id(":1.4"));
        tray.register(&id(":1.5"));

        let gone = tray.unregister(&id(":1.4"));

        assert_eq!(gone.map(|item| item.id), Some(id(":1.4")));
        assert_eq!(tray.len(), 1);
        assert_eq!(tray.unregister(&id(":1.4")), None, "exiting twice is fine");
    }

    #[test]
    fn a_passive_item_that_is_not_asking_for_attention_is_hidden() {
        let mut tray = Tray::new();
        let key = id(":1.6");
        tray.register(&key);

        assert_eq!(tray.visible().count(), 1, "active items are shown");

        tray.update(&key, TrayItemUpdate::Status(TrayStatus::Passive))
            .expect("registered");
        assert_eq!(tray.visible().count(), 0);

        tray.update(&key, TrayItemUpdate::Attention(true))
            .expect("registered");
        assert_eq!(tray.visible().count(), 1, "attention overrides passivity");
    }

    #[test]
    fn items_are_ordered_by_registration_not_by_arrival() {
        let mut tray = Tray::new();
        tray.register(&id(":1.9"));
        tray.register(&id(":1.2"));
        tray.register(&id(":1.5"));

        let names: Vec<_> = tray.items().map(|item| item.id.bus_name.as_str()).collect();
        assert_eq!(names, vec![":1.2", ":1.5", ":1.9"]);
    }

    #[test]
    fn a_menu_entry_is_found_through_its_submenu() {
        let mut tray = Tray::new();
        let key = id(":1.7");
        tray.register(&key);
        tray.update(&key, TrayItemUpdate::Menu(menu()))
            .expect("registered");

        assert!(tray.menu_entry(&key, MenuItemId::new(4)).is_ok());
        assert!(
            !tray
                .menu_entry(&key, MenuItemId::new(4))
                .expect("present")
                .is_separator()
        );
        assert_eq!(
            tray.menu_entry(&key, MenuItemId::new(4))
                .expect("present")
                .label(),
            Some("Quit")
        );
        assert!(
            tray.menu_entry(&key, MenuItemId::new(3))
                .map(|entry| !entry.is_enabled())
                .unwrap_or(false),
            "a disabled entry stays disabled"
        );
    }

    #[test]
    fn an_unknown_menu_entry_is_rejected() {
        let mut tray = Tray::new();
        let key = id(":1.8");
        tray.register(&key);
        tray.update(&key, TrayItemUpdate::Menu(menu()))
            .expect("registered");

        assert_eq!(
            tray.menu_entry(&key, MenuItemId::new(99)),
            Err(TrayError::UnknownMenuItem {
                id: key.clone(),
                entry: MenuItemId::new(99),
            })
        );
        assert!(matches!(
            tray.menu_entry(&id(":9.9"), MenuItemId::new(1)),
            Err(TrayError::UnknownItem { .. })
        ));
    }

    #[test]
    fn a_menu_without_entries_yields_no_identifier() {
        assert_eq!(MenuItem::Separator.id(), None);
        assert_eq!(MenuItem::Separator.label(), None);
        assert!(!MenuItem::Separator.is_enabled());
        assert!(MenuItem::Separator.find(MenuItemId::new(1)).is_none());
        assert!(MenuItem::Separator.ids().is_empty());
    }

    #[test]
    fn menu_identifiers_are_listed_depth_first() {
        let tree = MenuItem::Submenu {
            id: MenuItemId::new(1),
            label: "root".to_owned(),
            enabled: true,
            children: menu(),
        };

        let ids: Vec<_> = tree.ids().into_iter().map(MenuItemId::get).collect();
        assert_eq!(ids, vec![1, 1, 2, 3, 4]);
        assert!(tree.is_selectable(MenuItemId::new(1)));
    }

    #[test]
    fn a_snapshot_that_repeats_itself_changes_nothing() {
        let items = vec![item(":1.1", "Mail"), item(":1.2", "Music")];
        let mut tray = Tray::new();

        let first = tray.apply_snapshot(items.clone());
        assert_eq!(first.added, vec![id(":1.1"), id(":1.2")]);
        assert!(first.removed.is_empty());
        assert_eq!(tray.len(), 2);

        let second = tray.apply_snapshot(items);
        assert!(second.is_idempotent(), "{second:?}");
        assert_eq!(second.unchanged, 2);
        assert_eq!(tray.len(), 2, "a reconnect must not duplicate entries");
    }

    #[test]
    fn a_snapshot_drops_items_that_are_gone_and_takes_new_ones() {
        let mut tray = Tray::new();
        tray.apply_snapshot(vec![item(":1.1", "Mail"), item(":1.2", "Music")]);

        let diff = tray.apply_snapshot(vec![item(":1.2", "Music"), item(":1.3", "Chat")]);

        assert_eq!(diff.removed, vec![id(":1.1")]);
        assert_eq!(diff.added, vec![id(":1.3")]);
        assert_eq!(diff.updated, Vec::<TrayItemId>::new());
        assert_eq!(tray.len(), 2);
        assert!(!tray.contains(&id(":1.1")));
    }

    #[test]
    fn a_snapshot_reports_an_item_that_changed() {
        let mut tray = Tray::new();
        tray.apply_snapshot(vec![item(":1.1", "Mail")]);

        let diff = tray.apply_snapshot(vec![item(":1.1", "Mail (3)")]);

        assert_eq!(diff.updated, vec![id(":1.1")]);
        assert!(diff.added.is_empty() && diff.removed.is_empty());
        assert_eq!(
            tray.item(&id(":1.1")).map(|item| item.title.as_str()),
            Some("Mail (3)")
        );
    }

    #[test]
    fn a_restart_of_the_watcher_rebuilds_the_set() {
        // The panel held three items; the watcher restarted and reports two.
        let mut tray = Tray::new();
        tray.apply_snapshot(vec![
            item(":1.1", "Mail"),
            item(":1.2", "Music"),
            item(":1.3", "Chat"),
        ]);
        let before = tray.len();

        let diff = tray.apply_snapshot(vec![item(":2.1", "Mail"), item(":2.2", "Music")]);

        assert_eq!(before, 3);
        assert_eq!(tray.len(), 2, "the old registrations are gone, not kept");
        assert_eq!(diff.removed.len(), 3);
        assert_eq!(diff.added.len(), 2);
        assert_eq!(
            tray.items()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Mail", "Music"]
        );
    }

    #[test]
    fn an_empty_snapshot_empties_the_tray() {
        let mut tray = Tray::new();
        tray.apply_snapshot(vec![item(":1.1", "Mail")]);

        let diff = tray.apply_snapshot(Vec::new());

        assert_eq!(diff.removed, vec![id(":1.1")]);
        assert!(tray.is_empty());
        assert_eq!(tray.items().count(), 0);
    }
}
