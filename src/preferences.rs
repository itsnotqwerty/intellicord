use std::{
    fs,
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    disable_sponsored_content: bool,
    #[serde(default)]
    promotions: bool,
    #[serde(default)]
    shop: bool,
    #[serde(default)]
    decorations: bool,
}

pub struct PreferenceStore {
    file: PathBuf,
    preferences: Preferences,
    error: Option<&'static str>,
}

impl PreferenceStore {
    pub fn new(file: PathBuf) -> Self {
        let mut store = Self {
            file,
            preferences: Preferences::default(),
            error: None,
        };
        match store.load() {
            Ok(preferences) => store.preferences = preferences,
            Err(_) => store.error = Some("Unable to read Intellicord preferences"),
        }
        store
    }

    fn load(&self) -> Result<Preferences> {
        let bytes = match fs::read(&self.file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Preferences::default());
            }
            Err(error) => return Err(error.into()),
        };
        if bytes.len() > 4096 {
            bail!("Preference file exceeds size limit");
        }
        Ok(serde_json::from_slice::<Preferences>(&bytes)?)
    }

    fn save(&self, preferences: &Preferences) -> Result<()> {
        let directory = self
            .file
            .parent()
            .context("Preference path has no parent")?;
        fs::create_dir_all(directory)?;
        #[cfg(unix)]
        fs::set_permissions(
            directory,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )?;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temporary = directory.join(format!(".preferences-{}-{nonce}.tmp", std::process::id()));
        let result = (|| -> Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut output = options.open(&temporary)?;
            output.write_all(&serde_json::to_vec(preferences)?)?;
            output.sync_all()?;
            fs::rename(&temporary, &self.file)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    pub fn set(&mut self, disabled: bool) {
        let _ = self.set_surface("sponsored", disabled);
    }

    pub fn set_surface(&mut self, key: &str, disabled: bool) -> Result<()> {
        let mut preferences = self.preferences.clone();
        match key {
            "sponsored" => preferences.disable_sponsored_content = disabled,
            "promotions" => preferences.promotions = disabled,
            "shop" => preferences.shop = disabled,
            "decorations" => preferences.decorations = disabled,
            _ => {
                self.error = Some("Unsupported Intellicord preference surface; setting unchanged");
                bail!("Unsupported Intellicord preference surface");
            }
        }
        match self.save(&preferences) {
            Ok(()) => {
                self.preferences = preferences;
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.error = Some("Unable to save Intellicord preferences; setting unchanged");
                Err(error)
            }
        }
    }

    pub fn response(&self) -> serde_json::Value {
        serde_json::json!({
            "disabled": self.preferences.disable_sponsored_content,
            "promotions": self.preferences.promotions,
            "shop": self.preferences.shop,
            "decorations": self.preferences.decorations,
            "error": self.error
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "intellicord-preferences-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
            .join("preferences.json")
    }

    #[test]
    fn backend_preferences_round_trip_without_identity_or_history() {
        let file = path();
        let mut store = PreferenceStore::new(file.clone());
        assert_eq!(
            store.response(),
            serde_json::json!({"disabled":false,"promotions":false,"shop":false,"decorations":false,"error":null})
        );
        store.set(true);
        assert_eq!(
            PreferenceStore::new(file.clone()).response()["disabled"],
            true
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&file).unwrap()).unwrap(),
            serde_json::json!({"disable_sponsored_content":true,"promotions":false,"shop":false,"decorations":false})
        );
        store.set(false);
        assert_eq!(
            PreferenceStore::new(file.clone()).response()["disabled"],
            false
        );
        fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn malformed_files_and_failed_writes_leave_filtering_unchanged() {
        let file = path();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "{\"disable_sponsored_content\":\"true\"}").unwrap();
        let mut store = PreferenceStore::new(file.clone());
        assert_eq!(store.response()["disabled"], false);
        assert!(store.response()["error"].is_string());
        fs::remove_file(&file).unwrap();
        fs::create_dir(&file).unwrap();
        store.set(true);
        assert_eq!(store.response()["disabled"], false);
        assert!(store.response()["error"].is_string());
        fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn preferences_old_files_migrate_and_surfaces_persist_independently() {
        let file = path();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "{\"disable_sponsored_content\":true}").unwrap();
        let mut store = PreferenceStore::new(file.clone());
        assert_eq!(store.response()["promotions"], false);
        assert_eq!(store.response()["shop"], false);
        assert_eq!(store.response()["decorations"], false);
        for key in ["promotions", "shop", "decorations"] {
            store.set_surface(key, true).unwrap();
            assert_eq!(PreferenceStore::new(file.clone()).response()[key], true);
        }
        store.set_surface("shop", false).unwrap();
        store.set(false);
        let expected = serde_json::json!({"disabled":false,"promotions":true,"shop":false,"decorations":true,"error":null});
        assert_eq!(PreferenceStore::new(file.clone()).response(), expected);
        assert!(store.set_surface("unknown", true).is_err());
        assert_eq!(PreferenceStore::new(file.clone()).response(), expected);
        fs::remove_file(&file).unwrap();
        fs::create_dir(&file).unwrap();
        assert!(store.set_surface("promotions", false).is_err());
        assert_eq!(store.response()["promotions"], true);
        assert_eq!(store.response()["decorations"], true);
        assert!(store.response()["error"].is_string());
        fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }
}
