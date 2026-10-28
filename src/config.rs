//! Configuration, read once at startup.
//!
//! Every setting comes from an environment variable named `APP_` followed by the setting's
//! path in upper case, with `__` between nested keys: `server.bind_address` is read from
//! `APP_SERVER__BIND_ADDRESS`. An empty variable counts as unset. `.env.template` documents
//! every setting.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroUsize;

use ::config::{ConfigError, Environment};
use secrecy::SecretString;
use serde::Deserialize;

use crate::Env;
use crate::util::logging::LogFormat;

const DEFAULT_BIND_ADDRESS: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);
const DEFAULT_DATABASE_POOL_SIZE: NonZeroUsize = NonZeroUsize::new(10).unwrap();

#[derive(Debug, Deserialize)]
pub struct Config {
    /// Where the server runs (`APP_ENV`).
    #[serde(default)]
    pub env: Env,
    /// How log lines are written (`APP_LOG_FORMAT`). `RUST_LOG` filters them.
    #[serde(default)]
    pub log_format: LogFormat,
    #[serde(default)]
    pub server: Server,
    pub database: Database,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Database {
    /// Connection URL (`APP_DATABASE__URL`), for example
    /// `postgres://portal:password@localhost:5432/portal`.
    pub url: SecretString,
    /// Most connections the pool keeps open (`APP_DATABASE__POOL_SIZE`).
    #[serde(default = "default_database_pool_size")]
    pub pool_size: NonZeroUsize,
}

fn default_database_pool_size() -> NonZeroUsize {
    DEFAULT_DATABASE_POOL_SIZE
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Server {
    /// Address the HTTP listener binds to (`APP_SERVER__BIND_ADDRESS`).
    pub bind_address: SocketAddr,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            bind_address: DEFAULT_BIND_ADDRESS,
        }
    }
}

impl Config {
    /// Reads the configuration from the process environment.
    pub fn from_environment() -> Result<Self, ConfigError> {
        Self::from_source(environment())
    }

    fn from_source(environment: Environment) -> Result<Self, ConfigError> {
        ::config::Config::builder()
            .add_source(environment)
            .build()?
            .try_deserialize()
            .map_err(without_value)
    }
}

/// Drops the value from type and value errors: a variable set at the wrong level, such as
/// `APP_DATABASE` instead of `APP_DATABASE__URL`, can hold a secret.
fn without_value(error: ConfigError) -> ConfigError {
    match error {
        ConfigError::Type { key, expected, .. } => {
            let key = key
                .map(|key| format!(" for key `{key}`"))
                .unwrap_or_default();
            ConfigError::Message(format!("invalid type: expected {expected}{key}"))
        }
        ConfigError::Message(message) => ConfigError::Message(message_without_value(message)),
        ConfigError::At { error, origin, key } => ConfigError::At {
            error: Box::new(without_value(*error)),
            origin,
            key,
        },
        error => error,
    }
}

/// Rewrites serde's `invalid type: <value>, expected <what>` (and `invalid value: ...`)
/// without the value.
fn message_without_value(message: String) -> String {
    for prefix in ["invalid type: ", "invalid value: "] {
        let expected = message
            .strip_prefix(prefix)
            .and_then(|rest| rest.rsplit_once(", expected "));
        if let Some((_, expected)) = expected {
            return format!("{prefix}expected {expected}");
        }
    }
    message
}

fn environment() -> Environment {
    Environment::with_prefix("APP")
        .prefix_separator("_")
        .separator("__")
        .ignore_empty(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    const DATABASE_URL: &str = "postgres://portal:s3cr3t@localhost:5432/portal";

    fn load_exactly(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let vars = vars
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        Config::from_source(environment().source(Some(vars)))
    }

    /// Loads these variables plus the required ones.
    fn load(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        load_exactly(&[&[("APP_DATABASE__URL", DATABASE_URL)], vars].concat())
    }

    #[test]
    fn optional_settings_have_defaults() {
        let config = load(&[]).unwrap();

        assert_eq!(config.env, Env::Production);
        assert_eq!(config.log_format, LogFormat::Text);
        assert_eq!(config.server.bind_address, DEFAULT_BIND_ADDRESS);
        assert_eq!(config.database.url.expose_secret(), DATABASE_URL);
        assert_eq!(config.database.pool_size, DEFAULT_DATABASE_POOL_SIZE);
    }

    #[test]
    fn reads_prefixed_and_nested_variables() {
        let config = load(&[
            ("APP_ENV", "development"),
            ("APP_LOG_FORMAT", "json"),
            ("APP_SERVER__BIND_ADDRESS", "0.0.0.0:9000"),
            ("APP_DATABASE__POOL_SIZE", "3"),
        ])
        .unwrap();

        assert_eq!(config.env, Env::Development);
        assert_eq!(config.log_format, LogFormat::Json);
        assert_eq!(config.server.bind_address, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(config.database.pool_size.get(), 3);
    }

    #[test]
    fn the_pool_needs_at_least_one_connection() {
        let error = load(&[("APP_DATABASE__POOL_SIZE", "0")]).unwrap_err();

        assert!(error.to_string().contains("database.pool_size"), "{error}");
    }

    #[test]
    fn the_database_url_is_required() {
        let error = load_exactly(&[]).unwrap_err();

        assert!(error.to_string().contains("database"), "{error}");
    }

    #[test]
    fn debug_output_hides_the_database_url() {
        let config = load(&[]).unwrap();

        assert!(!format!("{config:?}").contains("s3cr3t"));
    }

    #[test]
    fn empty_variables_count_as_unset() {
        let config = load(&[("APP_ENV", ""), ("APP_SERVER__BIND_ADDRESS", "")]).unwrap();

        assert_eq!(config.env, Env::Production);
        assert_eq!(config.server.bind_address, DEFAULT_BIND_ADDRESS);
    }

    #[test]
    fn the_test_environment_cannot_be_selected() {
        let error = load(&[("APP_ENV", "test")]).unwrap_err();

        assert!(error.to_string().contains("for key `env`"), "{error}");
    }

    #[test]
    fn errors_do_not_repeat_the_value() {
        let error = load_exactly(&[("APP_DATABASE", DATABASE_URL)]).unwrap_err();

        assert!(!error.to_string().contains("s3cr3t"), "{error}");
        assert!(error.to_string().contains("database"), "{error}");
    }

    #[test]
    fn misspelled_settings_are_rejected() {
        let error = load(&[("APP_SERVER__BIND_ADDR", "0.0.0.0:8080")]).unwrap_err();

        assert!(
            error.to_string().contains("unknown field `bind_addr`"),
            "{error}"
        );
    }

    #[test]
    fn invalid_values_name_the_setting() {
        let error = load(&[("APP_SERVER__BIND_ADDRESS", "nowhere")]).unwrap_err();

        assert!(error.to_string().contains("server.bind_address"), "{error}");
    }
}
