//! Configuration, read once at startup.
//!
//! Every setting comes from an environment variable named `APP_` followed by the setting's
//! path in upper case, with `__` between nested keys: `server.bind_address` is read from
//! `APP_SERVER__BIND_ADDRESS`. An empty variable counts as unset. `.env.template` documents
//! every setting.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use ::config::{ConfigError, Environment};
use serde::Deserialize;

use crate::Env;
use crate::util::logging::LogFormat;

const DEFAULT_BIND_ADDRESS: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);

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
}

#[derive(Debug, Deserialize)]
#[serde(default)]
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
    }
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

    fn load(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let vars = vars
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        Config::from_source(environment().source(Some(vars)))
    }

    #[test]
    fn every_setting_has_a_default() {
        let config = load(&[]).unwrap();

        assert_eq!(config.env, Env::Development);
        assert_eq!(config.log_format, LogFormat::Text);
        assert_eq!(config.server.bind_address, DEFAULT_BIND_ADDRESS);
    }

    #[test]
    fn reads_prefixed_and_nested_variables() {
        let config = load(&[
            ("APP_ENV", "production"),
            ("APP_LOG_FORMAT", "json"),
            ("APP_SERVER__BIND_ADDRESS", "0.0.0.0:9000"),
        ])
        .unwrap();

        assert_eq!(config.env, Env::Production);
        assert_eq!(config.log_format, LogFormat::Json);
        assert_eq!(config.server.bind_address, "0.0.0.0:9000".parse().unwrap());
    }

    #[test]
    fn empty_variables_count_as_unset() {
        let config = load(&[("APP_ENV", ""), ("APP_SERVER__BIND_ADDRESS", "")]).unwrap();

        assert_eq!(config.env, Env::Development);
        assert_eq!(config.server.bind_address, DEFAULT_BIND_ADDRESS);
    }

    #[test]
    fn the_test_environment_cannot_be_selected() {
        let error = load(&[("APP_ENV", "test")]).unwrap_err();

        assert!(error.to_string().contains("for key `env`"), "{error}");
    }

    #[test]
    fn invalid_values_name_the_setting() {
        let error = load(&[("APP_SERVER__BIND_ADDRESS", "nowhere")]).unwrap_err();

        assert!(error.to_string().contains("server.bind_address"), "{error}");
    }
}
