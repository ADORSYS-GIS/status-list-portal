//! Secrets read from files, as Docker and Kubernetes mount them: a secret setting's
//! variable followed by `_FILE`, such as `APP_SESSION__KEY_FILE`, names the file that holds
//! its value. The value then stays out of the process environment, where child processes,
//! `ps` and crash reports can see it.

use std::collections::HashMap;

use ::config::ConfigError;

/// The settings whose values may come from a file, by path.
pub(super) const SECRET_SETTINGS: [&str; 3] =
    ["database.url", "session.key", "github.client_secret"];

/// The values that the `_FILE` variables of `settings` in `vars` point to, by setting path. A
/// file's final line break is dropped, since `echo` and most editors add one.
pub(super) fn read(
    vars: &HashMap<String, String>,
    settings: &[&'static str],
) -> Result<Vec<(&'static str, String)>, ConfigError> {
    let set = |name: &str| vars.get(name).filter(|value| !value.is_empty());
    settings
        .iter()
        .copied()
        .filter_map(|setting| {
            let variable = variable(setting);
            let file_variable = format!("{variable}_FILE");
            let path = set(&file_variable)?;
            let value = if set(&variable).is_some() {
                Err(format!("set {variable} or {file_variable}, not both"))
            } else {
                read_secret(&file_variable, path)
            };
            Some(
                value
                    .map(|value| (setting, value))
                    .map_err(ConfigError::Message),
            )
        })
        .collect()
}

/// Whether `name` is the `_FILE` variable of one of `settings`, such as
/// `APP_SESSION__KEY_FILE`.
pub(super) fn is_file_variable(name: &str, settings: &[&str]) -> bool {
    settings
        .iter()
        .any(|setting| name.strip_suffix("_FILE") == Some(variable(setting).as_str()))
}

/// The variable that holds `setting`: `database.url` is in `APP_DATABASE__URL`.
fn variable(setting: &str) -> String {
    format!("APP_{}", setting.to_uppercase().replace('.', "__"))
}

fn read_secret(file_variable: &str, path: &str) -> Result<String, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|error| format!("{file_variable}: cannot read {path}: {error}"))?;
    let value = contents.strip_suffix('\n').unwrap_or(&contents);
    let value = value.strip_suffix('\r').unwrap_or(value);
    if value.is_empty() {
        return Err(format!("{file_variable}: {path} is empty"));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    /// A file holding `contents`, removed when the test ends.
    struct SecretFile(PathBuf);

    impl SecretFile {
        fn new(contents: &str) -> Self {
            let path = std::env::temp_dir().join(format!("portal-secret-{}", Uuid::new_v4()));
            std::fs::write(&path, contents).unwrap();
            Self(path)
        }

        fn path(&self) -> String {
            self.0.to_str().unwrap().to_owned()
        }
    }

    impl Drop for SecretFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn file_variables_name_the_secret_settings() {
        assert_eq!(variable("database.url"), "APP_DATABASE__URL");
        assert_eq!(
            variable("github.client_secret"),
            "APP_GITHUB__CLIENT_SECRET"
        );
    }

    #[test]
    fn secrets_are_read_without_the_final_line_break() {
        let key = SecretFile::new("0123abcd\n");
        let secret = SecretFile::new("client-secret\r\n");
        let url = SecretFile::new("postgres://portal@db/portal");
        let with_files = vars(&[
            ("APP_SESSION__KEY_FILE", &key.path()),
            ("APP_GITHUB__CLIENT_SECRET_FILE", &secret.path()),
            ("APP_DATABASE__URL_FILE", &url.path()),
        ]);

        let values = read(&with_files, &SECRET_SETTINGS).unwrap();

        assert_eq!(
            values,
            [
                ("database.url", "postgres://portal@db/portal".to_string()),
                ("session.key", "0123abcd".to_string()),
                ("github.client_secret", "client-secret".to_string()),
            ]
        );
    }

    #[test]
    fn nothing_is_read_without_file_variables() {
        let without_files = vars(&[
            ("APP_SESSION__KEY", "0123abcd"),
            ("APP_SESSION__KEY_FILE", ""),
        ]);
        assert_eq!(read(&without_files, &SECRET_SETTINGS).unwrap(), []);
    }

    #[test]
    fn a_secret_comes_from_one_place() {
        let key = SecretFile::new("0123abcd");
        let both = vars(&[
            ("APP_SESSION__KEY", "0123abcd"),
            ("APP_SESSION__KEY_FILE", &key.path()),
        ]);

        let error = read(&both, &SECRET_SETTINGS).unwrap_err();

        assert_eq!(
            error.to_string(),
            "set APP_SESSION__KEY or APP_SESSION__KEY_FILE, not both"
        );
    }

    #[test]
    fn missing_and_empty_files_are_refused() {
        let empty = SecretFile::new("\n");
        let empty_file = vars(&[("APP_DATABASE__URL_FILE", &empty.path())]);
        let error = read(&empty_file, &SECRET_SETTINGS).unwrap_err().to_string();
        assert!(error.ends_with("is empty"), "{error}");

        let missing_file = vars(&[("APP_DATABASE__URL_FILE", "/nonexistent/secret")]);
        let error = read(&missing_file, &SECRET_SETTINGS)
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with("APP_DATABASE__URL_FILE: cannot read /nonexistent/secret"),
            "{error}"
        );
    }

    #[test]
    fn only_secret_settings_have_file_variables() {
        assert!(is_file_variable("APP_SESSION__KEY_FILE", &SECRET_SETTINGS));
        assert!(is_file_variable("APP_DATABASE__URL_FILE", &SECRET_SETTINGS));
        assert!(!is_file_variable("APP_SESSION__KEY", &SECRET_SETTINGS));
        assert!(!is_file_variable(
            "APP_SERVER__ORIGIN_FILE",
            &SECRET_SETTINGS
        ));
    }
}
