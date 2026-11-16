use std::{
    fs,
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    disable_sponsored_content: bool,
}

pub struct PreferenceStore {
    file: PathBuf,
    disabled: bool,
    error: Option<&'static str>,
}

impl PreferenceStore {
    pub fn new(file: PathBuf) -> Self {
        let mut store = Self {
            file,
            disabled: false,
            error: None,
        };
        match store.load() {
            Ok(disabled) => store.disabled = disabled,
            Err(_) => store.error = Some("Unable to read Intellicord preferences"),
        }
        store
    }

    fn load(&self) -> Result<bool> {
        let bytes = match fs::read(&self.file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        if bytes.len() > 4096 {
            bail!("Preference file exceeds size limit");
        }
        Ok(serde_json::from_slice::<Preferences>(&bytes)?.disable_sponsored_content)
    }

    fn save(&self, disabled: bool) -> Result<()> {
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
            output.write_all(&serde_json::to_vec(&Preferences {
                disable_sponsored_content: disabled,
            })?)?;
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
        match self.save(disabled) {
            Ok(()) => {
                self.disabled = disabled;
                self.error = None;
            }
            Err(_) => {
                self.error = Some("Unable to save Intellicord preferences; setting unchanged")
            }
        }
    }

    pub fn response(&self) -> serde_json::Value {
        serde_json::json!({ "disabled": self.disabled, "error": self.error })
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
            serde_json::json!({"disabled":false,"error":null})
        );
        store.set(true);
        assert_eq!(
            PreferenceStore::new(file.clone()).response()["disabled"],
            true
        );
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\"disable_sponsored_content\":true}"
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
}
