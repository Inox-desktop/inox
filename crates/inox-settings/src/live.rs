//! Confirmed per-consumer application; all calls run on Iced's worker pool.

use inox_config::{Appearance, ConfigPaths, InoxConfig};
use inox_settings::app::Message;
use inox_settings::apply::{Component, Generation};
use inox_settings::document::{Documents, SettingFile};
use inox_settings::draft::{Field, Settings};
use std::collections::BTreeSet;
use std::time::Duration;

const PATH: &str = "/org/inox/Configuration";
const INTERFACE: &str = "org.inox.Configuration1";

pub(super) fn destinations(component: Component, fields: &BTreeSet<Field>) -> Vec<&'static str> {
    match component {
        Component::Compositor => vec!["org.inox.Compositor"],
        Component::Notifications => vec!["org.inox.Notifications"],
        Component::Shell => {
            let all = fields.is_empty() || fields.iter().any(|f| f.file() == SettingFile::Root);
            [
                ("panel.", "org.inox.Panel"),
                ("launcher.", "org.inox.Launcher"),
                ("wallpaper.", "org.inox.Wallpaper"),
            ]
            .into_iter()
            .filter(|(prefix, _)| all || fields.iter().any(|f| f.path().starts_with(prefix)))
            .map(|(_, name)| name)
            .collect()
        }
    }
}

fn candidate(
    paths: &ConfigPaths,
    documents: &Documents,
    expected: &Settings,
) -> Result<InoxConfig, String> {
    let config = InoxConfig::load(paths, Appearance::System).map_err(|e| e.to_string())?;
    let disk = Documents::open(&config, paths).map_err(|e| e.to_string())?;
    for file in SettingFile::ALL {
        let before = documents
            .get(file)
            .ok_or_else(|| format!("{} was not loaded", file.name()))?;
        let after = disk
            .get(file)
            .ok_or_else(|| format!("{} was not loaded", file.name()))?;
        if before.text() != after.text() || before.path() != after.path() {
            return Err(format!(
                "{} changed after saving; read the files again",
                file.name()
            ));
        }
    }
    if Settings::from_config(&config) != *expected {
        return Err(
            "configuration differs from the saved preferences; read the files again".into(),
        );
    }
    Ok(config)
}

pub(super) fn apply(
    paths: &ConfigPaths,
    documents: &Documents,
    expected: &Settings,
    fields: &BTreeSet<Field>,
    generation: Generation,
    component: Component,
) -> Message {
    let config = match candidate(paths, documents, expected) {
        Ok(config) => config,
        Err(reason) => {
            return Message::ComponentRejected {
                component,
                generation,
                reason,
                fields: vec![],
            };
        }
    };
    let snapshot = match config.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Message::ComponentRejected {
                component,
                generation,
                reason: error.to_string(),
                fields: vec![],
            };
        }
    };
    let connection = match zbus::block_on(
        zbus::connection::Builder::session()
            .method_timeout(Duration::from_secs(6))
            .build(),
    ) {
        Ok(connection) => connection,
        Err(error) => {
            return Message::ComponentUnreachable {
                component,
                generation,
                reason: error.to_string(),
            };
        }
    };
    let mut unavailable = Vec::new();
    let mut rejected = Vec::new();
    for destination in destinations(component, fields) {
        let result: Result<(u64, String), zbus::Error> = zbus::block_on(async {
            let proxy = zbus::Proxy::new(&connection, destination, PATH, INTERFACE).await?;
            proxy.call("Reload", &(generation.get(), &snapshot)).await
        });
        match result {
            Ok((reply_generation, applied))
                if reply_generation == generation.get() && applied == snapshot => {}
            Ok(_) => rejected.push(format!(
                "{destination}: configuration readback or generation mismatch"
            )),
            Err(zbus::Error::FDO(error))
                if matches!(error.as_ref(), zbus::fdo::Error::Failed(_)) =>
            {
                rejected.push(format!("{destination}: {error}"));
            }
            Err(error) => unavailable.push(format!("{destination}: {error}")),
        }
    }
    if !rejected.is_empty() {
        rejected.extend(unavailable);
        return Message::ComponentRejected {
            component,
            generation,
            reason: rejected.join("; "),
            fields: vec![],
        };
    }
    if !unavailable.is_empty() {
        return Message::ComponentUnreachable {
            component,
            generation,
            reason: unavailable.join("; "),
        };
    }
    Message::LiveConfigured {
        generation,
        component,
        config: Box::new(config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_consumers_of_changed_fields_are_contacted() {
        assert_eq!(
            destinations(Component::Shell, &BTreeSet::from([Field::PanelHeight])),
            ["org.inox.Panel"]
        );
        assert_eq!(
            destinations(Component::Shell, &BTreeSet::from([Field::WallpaperImage])),
            ["org.inox.Wallpaper"]
        );
        assert_eq!(
            destinations(Component::Shell, &BTreeSet::from([Field::LocaleLanguage])).len(),
            3
        );
    }

    #[test]
    fn an_external_edit_after_saving_cannot_be_confirmed_as_the_users_candidate() {
        let dir = tempfile::tempdir().expect("directory");
        let paths = ConfigPaths::new(dir.path());
        let config = InoxConfig::load(&paths, Appearance::System).expect("defaults");
        let documents = Documents::open(&config, &paths).expect("documents");
        let expected = Settings::from_config(&config);
        assert!(candidate(&paths, &documents, &expected).is_ok());
        std::fs::write(dir.path().join("shell.toml"), "[panel]\nheight = 48\n")
            .expect("external edit");
        assert!(
            candidate(&paths, &documents, &expected)
                .expect_err("conflict")
                .contains("shell.toml")
        );
    }
}
