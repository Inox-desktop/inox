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
/// The interface that can hold a display change provisionally and take it back
/// (ADR 0028). The compositor serves it at the same object path as the v1
/// interface, which stays as it was for every other consumer.
const INTERFACE_TRIAL: &str = "org.inox.Configuration2";
/// The consumer that owns the displays, and therefore the only one asked for a
/// display trial.
const COMPOSITOR: &str = "org.inox.Compositor";
/// How long a call may take before the window treats it as unanswered.
const CALL_TIMEOUT: Duration = Duration::from_secs(6);

pub(super) fn destinations(component: Component, fields: &BTreeSet<Field>) -> Vec<&'static str> {
    match component {
        Component::Compositor => vec![COMPOSITOR],
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
    trial: bool,
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
    // A display change is asked for as a trial, and only the compositor is asked
    // for one: it is the consumer that owns the displays (ADR 0028).
    let provisional = trial && component == Component::Compositor;
    let method = if provisional { "Trial" } else { "Reload" };
    let interface = if provisional {
        INTERFACE_TRIAL
    } else {
        INTERFACE
    };
    let connection = match zbus::block_on(
        zbus::connection::Builder::session()
            .method_timeout(CALL_TIMEOUT)
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
            let proxy = zbus::Proxy::new(&connection, destination, PATH, interface).await?;
            proxy.call(method, &(generation.get(), &snapshot)).await
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
    if provisional {
        return Message::DisplayTrialArmed {
            generation,
            config: Box::new(config),
        };
    }
    Message::LiveConfigured {
        generation,
        component,
        config: Box::new(config),
    }
}

/// Asks the compositor to keep the display change it is holding provisionally.
///
/// A call that fails leaves the trial open: the compositor reverts it on its own
/// when the countdown runs out, so the window reports the failure and keeps
/// counting rather than pretending the change was kept (ADR 0028).
pub(super) fn confirm_display(generation: Generation) -> Message {
    match display_call("Confirm", generation, true) {
        Ok(()) => Message::DisplayConfirmed { generation },
        Err(reason) => Message::DisplayConfirmFailed { generation, reason },
    }
}

/// Asks the compositor to put the previous display settings back.
///
/// A failure here is not a failure of the change itself: the files are restored
/// either way, so the window says which of the two happened.
pub(super) fn rollback_display(generation: Generation) -> Message {
    match display_call("Rollback", generation, false) {
        Ok(()) => Message::DisplayReverted { generation },
        Err(reason) => Message::DisplayRevertFailed { generation, reason },
    }
}

/// One call of the trial interface that carries only a generation.
///
/// `same_generation` says whether the reply has to echo the generation it was
/// asked about: a confirmation of a stale trial must not be taken as an answer
/// about the current one, while a rollback is idempotent by contract and reports
/// whatever generation was asked for.
fn display_call(method: &str, generation: Generation, same_generation: bool) -> Result<(), String> {
    let connection = zbus::block_on(
        zbus::connection::Builder::session()
            .method_timeout(CALL_TIMEOUT)
            .build(),
    )
    .map_err(|error| error.to_string())?;

    let result: Result<(u64, String), zbus::Error> = zbus::block_on(async {
        let proxy = zbus::Proxy::new(&connection, COMPOSITOR, PATH, INTERFACE_TRIAL).await?;
        proxy.call(method, &(generation.get(),)).await
    });

    match result {
        Ok((reply_generation, _applied))
            if !same_generation || reply_generation == generation.get() =>
        {
            Ok(())
        }
        Ok(_) => Err(format!("{COMPOSITOR}: generation mismatch for {method}")),
        Err(error) => Err(format!("{COMPOSITOR}: {error}")),
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
