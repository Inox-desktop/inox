//! The entry registry: what it resolves, and what it refuses (acceptance C01).
//!
//! The rejection cases matter as much as the happy path: a name that no entry
//! has, an entry listed twice and a configuration path that leaves the
//! configuration directory must all fail at construction, with an error that
//! names the entry and the reason, rather than surfacing later as a component
//! that reads the wrong file.

use inox_config::{ComponentFiles, ConfigPaths};
use inox_shell::contract::ServiceState;
use inox_shell::diagnostic::Diagnostic;
use inox_shell::i18n::{EN_US, id};
use inox_shell::navigation::EntryId;
use inox_shell::registry::{EntryRegistry, EntrySpec, RegistryError};
use std::path::PathBuf;

fn paths() -> ConfigPaths {
    ConfigPaths::new("/tmp/inox-shell-registry-test")
}

fn spec(name: &str, config: &str) -> EntrySpec {
    EntrySpec::new(name, format!("inox-{name}"), config, false)
}

#[test]
fn a_stock_session_resolves_the_four_entries() {
    let paths = paths();
    let registry = EntryRegistry::standard(&paths, &ComponentFiles::default())
        .expect("the default component files are inside the configuration directory");

    assert_eq!(registry.len(), 4);
    assert!(!registry.is_empty());
    assert_eq!(
        registry.iter().map(|entry| entry.id()).collect::<Vec<_>>(),
        EntryId::ALL.to_vec()
    );

    let panel = registry
        .get(EntryId::Panel)
        .expect("the panel is registered");
    assert_eq!(panel.binary(), "inox-panel");
    assert_eq!(
        panel.config(),
        paths
            .file(&PathBuf::from("shell.toml"))
            .expect("inside")
            .as_path()
    );
    assert!(panel.is_required());
    assert_eq!(panel.state(), &ServiceState::Starting);

    let launcher = registry
        .get(EntryId::Launcher)
        .expect("the launcher is registered");
    assert_eq!(launcher.binary(), "inox-launcher");
    assert!(launcher.is_required());

    let notifications = registry
        .get(EntryId::Notifications)
        .expect("the notifications daemon is registered");
    assert_eq!(notifications.binary(), "inox-notifications");
    assert_eq!(
        notifications.config(),
        paths
            .file(&PathBuf::from("notifications.toml"))
            .expect("inside")
            .as_path()
    );
    assert!(!notifications.is_required());

    let settings = registry
        .get(EntryId::Settings)
        .expect("settings is registered");
    assert_eq!(settings.binary(), "inox-settings");
    assert!(!settings.is_required());
}

#[test]
fn the_configuration_files_are_the_ones_config_toml_names() {
    let files = ComponentFiles {
        compositor: PathBuf::from("compositor.toml"),
        shell: PathBuf::from("desktop/shell.toml"),
        notifications: PathBuf::from("desktop/notifications.toml"),
    };

    let registry = EntryRegistry::standard(&paths(), &files).expect("the paths are inside");

    let panel = registry
        .get(EntryId::Panel)
        .expect("the panel is registered");
    assert_eq!(
        panel.config(),
        paths().file(&files.shell).expect("inside").as_path()
    );
    assert_eq!(
        registry
            .get(EntryId::Notifications)
            .expect("registered")
            .config(),
        paths()
            .file(&files.notifications)
            .expect("inside")
            .as_path()
    );
}

#[test]
fn an_unknown_entry_name_is_rejected_with_the_names_that_would_work() {
    let error = EntryRegistry::resolve(&paths(), &[spec("tray", "shell.toml")])
        .expect_err("tray is not an entry");

    match &error {
        RegistryError::UnknownEntry { name, known } => {
            assert_eq!(name, "tray");
            assert!(known.contains("panel"), "{known}");
            assert!(known.contains("settings"), "{known}");
        }
        other => panic!("unexpected error: {other}"),
    }
    assert!(error.to_string().contains("tray"));
    assert_eq!(error.describe().id(), id::REGISTRY_UNKNOWN_ENTRY);
}

#[test]
fn an_entry_registered_twice_is_rejected() {
    let error = EntryRegistry::resolve(
        &paths(),
        &[
            spec("panel", "shell.toml"),
            spec("launcher", "shell.toml"),
            spec("panel", "other.toml"),
        ],
    )
    .expect_err("the panel is listed twice");

    assert!(matches!(
        error,
        RegistryError::DuplicateEntry { id: EntryId::Panel }
    ));
    assert_eq!(error.describe().id(), id::REGISTRY_DUPLICATE_ENTRY);
}

#[test]
fn a_config_path_outside_the_configuration_directory_is_rejected() {
    let escaping = [
        "../shell.toml",
        "/etc/inox/shell.toml",
        "themes/../../shell.toml",
        "",
    ];

    for config in escaping {
        let error = EntryRegistry::resolve(&paths(), &[spec("panel", config)]);

        match error {
            Err(RegistryError::InvalidConfigPath { id, path, .. }) => {
                assert_eq!(id, EntryId::Panel);
                assert_eq!(path, PathBuf::from(config));
            }
            other => panic!("`{config}` was accepted: {other:?}"),
        }
    }
}

#[test]
fn a_valid_nested_path_is_accepted() {
    let registry = EntryRegistry::resolve(&paths(), &[spec("panel", "desktop/panel.toml")])
        .expect("a nested path inside the configuration directory");

    let panel = registry.get(EntryId::Panel).expect("registered");
    assert!(panel.config().ends_with("desktop/panel.toml"));
}

#[test]
fn the_session_state_of_an_entry_can_be_updated() {
    let mut registry = EntryRegistry::standard(&paths(), &ComponentFiles::default())
        .expect("the standard entries");

    registry
        .set_state(EntryId::Launcher, ServiceState::Ready)
        .expect("the launcher is registered");
    assert_eq!(
        registry.get(EntryId::Launcher).expect("registered").state(),
        &ServiceState::Ready
    );

    registry
        .set_state(
            EntryId::Panel,
            ServiceState::Unavailable {
                actionable: Diagnostic::new(id::SERVICE_UNAVAILABLE),
            },
        )
        .expect("the panel is registered");
    assert!(
        !registry
            .get(EntryId::Panel)
            .expect("registered")
            .state()
            .is_ready()
    );
}

#[test]
fn setting_the_state_of_an_entry_that_is_not_registered_is_rejected() {
    let mut registry =
        EntryRegistry::resolve(&paths(), &[spec("panel", "shell.toml")]).expect("one entry");

    let error = registry
        .set_state(EntryId::Notifications, ServiceState::Ready)
        .expect_err("notifications is not in this registry");

    assert!(matches!(
        error,
        RegistryError::NotRegistered {
            id: EntryId::Notifications
        }
    ));
    assert_eq!(error.describe().id(), id::REGISTRY_NOT_REGISTERED);
    assert_eq!(registry.get(EntryId::Notifications), None);
}

#[test]
fn an_empty_registry_is_valid() {
    let registry = EntryRegistry::resolve(&paths(), &[]).expect("no entries is not an error");

    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert_eq!(registry.iter().count(), 0);
}

#[test]
fn every_registry_error_has_a_catalog_message() {
    let errors = [
        RegistryError::UnknownEntry {
            name: "tray".to_owned(),
            known: EntryId::names(),
        },
        RegistryError::DuplicateEntry { id: EntryId::Panel },
        RegistryError::NotRegistered {
            id: EntryId::Settings,
        },
    ];

    let defined = inox_common::i18n::message_ids(&[EN_US]).expect("the catalog parses");

    for error in &errors {
        let diagnostic = error.describe();

        assert!(
            defined.contains(diagnostic.id()),
            "`{}` is missing from the catalog",
            diagnostic.id()
        );
        assert!(id::ALL.contains(&diagnostic.id()));
    }
}
