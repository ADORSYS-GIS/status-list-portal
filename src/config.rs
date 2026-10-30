//! Configuration, read once at startup.
//!
//! Every setting comes from an environment variable named `APP_` followed by the setting's
//! path in upper case, with `__` between nested keys: `server.bind_address` is read from
//! `APP_SERVER__BIND_ADDRESS`. An empty variable counts as unset. `.env.template` documents
//! every setting.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroUsize;
use std::path::PathBuf;

use ::config::{ConfigError, Environment};
use axum_extra::extract::cookie::Key;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, de};
use url::{Host, Url};

use crate::Env;
use crate::admin_api::SigningAlgorithm;
use crate::util::logging::LogFormat;

const DEFAULT_BIND_ADDRESS: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);
const DEFAULT_DATABASE_POOL_SIZE: NonZeroUsize = NonZeroUsize::new(10).unwrap();
/// Bytes in the session key: half signs the cookies, half encrypts them.
const SESSION_KEY_LEN: usize = 64;

#[derive(Debug, Deserialize)]
pub struct Config {
    /// Where the server runs (`APP_ENV`).
    #[serde(default)]
    pub env: Env,
    /// How log lines are written (`APP_LOG_FORMAT`). `RUST_LOG` filters them.
    #[serde(default)]
    pub log_format: LogFormat,
    pub server: Server,
    pub database: Database,
    pub session: Session,
    pub github: GitHub,
    pub admin_api: AdminApi,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    /// Address the HTTP listener binds to (`APP_SERVER__BIND_ADDRESS`).
    #[serde(default = "default_bind_address")]
    pub bind_address: SocketAddr,
    /// Where browsers reach the portal (`APP_SERVER__ORIGIN`).
    pub origin: PublicOrigin,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    /// Secret the session cookies are encrypted with (`APP_SESSION__KEY`).
    pub key: SessionKey,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHub {
    /// Client ID of the GitHub OAuth app that signs developers in (`APP_GITHUB__CLIENT_ID`).
    pub client_id: String,
    /// The OAuth app's client secret (`APP_GITHUB__CLIENT_SECRET`).
    pub client_secret: SecretString,
    /// Where GitHub serves its sign-in pages. Only tests change it.
    #[serde(skip, default = "default_github_web_url")]
    pub web_url: Url,
    /// Where GitHub serves its REST API. Only tests change it.
    #[serde(skip, default = "default_github_api_url")]
    pub api_url: Url,
}

/// The Status List Server's admin API. The server's registry file must list the portal as an
/// admin principal with the `issuers:read` and `issuers:write` scopes and the public half of
/// the signing key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminApi {
    /// Where the server's admin listener is reached (`APP_ADMIN_API__URL`), such as
    /// `http://status-list-server:8002`.
    #[serde(deserialize_with = "http_url")]
    pub url: Url,
    /// The portal's admin principal `id` in the registry file (`APP_ADMIN_API__PRINCIPAL`).
    pub principal: String,
    /// One of the server's `admin_auth.audiences` (`APP_ADMIN_API__AUDIENCE`).
    pub audience: String,
    /// PEM file with the private key that signs admin tokens
    /// (`APP_ADMIN_API__SIGNING_KEY_FILE`).
    pub signing_key_file: PathBuf,
    /// The key's `kid` in the registry file (`APP_ADMIN_API__KEY_ID`).
    pub key_id: String,
    /// The key's `alg` in the registry file (`APP_ADMIN_API__ALGORITHM`).
    #[serde(default = "default_admin_api_algorithm")]
    pub algorithm: SigningAlgorithm,
}

/// The scheme, host and port browsers use to reach the portal, such as
/// `https://portal.example.com`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOrigin(String);

impl PublicOrigin {
    pub fn parse(input: &str) -> Result<Self, String> {
        let url = Url::parse(input).map_err(|error| format!("is not a URL: {error}"))?;
        let is_bare = matches!(url.scheme(), "http" | "https")
            && url.has_host()
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none();
        if !is_bare {
            return Err(
                "must be a scheme, host and optional port, such as https://portal.example.com"
                    .to_string(),
            );
        }
        // The cookies are `Secure`, which browsers only accept over https or on loopback.
        if url.scheme() == "http" && !is_loopback(&url) {
            return Err(
                "must use https, unless the host is localhost or a loopback address".to_string(),
            );
        }
        Ok(Self(url.origin().ascii_serialization()))
    }

    /// The origin as browsers write it in the `Origin` header.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The absolute URL of `path`, which starts with `/`, on the portal.
    pub fn url_for(&self, path: &str) -> String {
        format!("{}{path}", self.0)
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain == "localhost",
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

impl<'de> Deserialize<'de> for PublicOrigin {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let input = String::deserialize(deserializer)?;
        Self::parse(&input).map_err(de::Error::custom)
    }
}

/// Key that encrypts the session cookies. Its `Debug` output hides the key.
#[derive(Debug, Clone)]
pub struct SessionKey(Key);

impl SessionKey {
    /// A random key, for tests.
    pub fn generate() -> Self {
        Self(Key::generate())
    }

    pub fn key(&self) -> &Key {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SessionKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Random bytes in hex, rather than any long string, so that a guessable passphrase
        // cannot become the key.
        let secret = SecretString::deserialize(deserializer)?;
        let bytes = hex::decode(secret.expose_secret())
            .ok()
            .filter(|bytes| bytes.len() == SESSION_KEY_LEN)
            .ok_or_else(|| {
                de::Error::custom(format!(
                    "must be {SESSION_KEY_LEN} random bytes in hex, \
                     such as the output of `openssl rand -hex {SESSION_KEY_LEN}`"
                ))
            })?;
        Key::try_from(bytes.as_slice())
            .map(Self)
            .map_err(de::Error::custom)
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

fn default_bind_address() -> SocketAddr {
    DEFAULT_BIND_ADDRESS
}

fn default_database_pool_size() -> NonZeroUsize {
    DEFAULT_DATABASE_POOL_SIZE
}

fn default_github_web_url() -> Url {
    Url::parse("https://github.com/").expect("the URL is valid")
}

fn default_github_api_url() -> Url {
    Url::parse("https://api.github.com/").expect("the URL is valid")
}

fn default_admin_api_algorithm() -> SigningAlgorithm {
    SigningAlgorithm::Es256
}

fn http_url<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Url, D::Error> {
    let input = String::deserialize(deserializer)?;
    Url::parse(&input)
        .ok()
        .filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.has_host()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
        .ok_or_else(|| {
            de::Error::custom("must be an http:// or https:// URL without credentials or query")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATABASE_URL: &str = "postgres://portal:s3cr3t-db@localhost:5432/portal";
    const SESSION_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\
                               0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    const REQUIRED: &[(&str, &str)] = &[
        ("APP_SERVER__ORIGIN", "https://portal.example.com"),
        ("APP_DATABASE__URL", DATABASE_URL),
        ("APP_SESSION__KEY", SESSION_KEY),
        ("APP_GITHUB__CLIENT_ID", "client-id"),
        ("APP_GITHUB__CLIENT_SECRET", "s3cr3t-client"),
        ("APP_ADMIN_API__URL", "http://status-list-server:8002"),
        ("APP_ADMIN_API__PRINCIPAL", "portal"),
        ("APP_ADMIN_API__AUDIENCE", "status-list-admin"),
        (
            "APP_ADMIN_API__SIGNING_KEY_FILE",
            "/run/secrets/admin-key.pem",
        ),
        ("APP_ADMIN_API__KEY_ID", "portal-1"),
    ];

    fn load_exactly<'a>(
        vars: impl IntoIterator<Item = &'a (&'a str, &'a str)>,
    ) -> Result<Config, ConfigError> {
        let vars = vars
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        Config::from_source(environment().source(Some(vars)))
    }

    /// Loads the required settings, then these variables, which can override them.
    fn load(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        load_exactly(REQUIRED.iter().chain(vars))
    }

    fn load_without(key: &str) -> Result<Config, ConfigError> {
        load_exactly(REQUIRED.iter().filter(|(name, _)| *name != key))
    }

    #[test]
    fn optional_settings_have_defaults() {
        let config = load(&[]).unwrap();

        assert_eq!(config.env, Env::Production);
        assert_eq!(config.log_format, LogFormat::Text);
        assert_eq!(config.server.bind_address, DEFAULT_BIND_ADDRESS);
        assert_eq!(config.database.pool_size, DEFAULT_DATABASE_POOL_SIZE);
        assert_eq!(config.github.web_url.as_str(), "https://github.com/");
        assert_eq!(config.github.api_url.as_str(), "https://api.github.com/");
        assert_eq!(config.admin_api.algorithm, SigningAlgorithm::Es256);
    }

    #[test]
    fn reads_prefixed_and_nested_variables() {
        let config = load(&[
            ("APP_ENV", "development"),
            ("APP_LOG_FORMAT", "json"),
            ("APP_SERVER__BIND_ADDRESS", "0.0.0.0:9000"),
            ("APP_DATABASE__POOL_SIZE", "3"),
            ("APP_ADMIN_API__ALGORITHM", "EdDSA"),
        ])
        .unwrap();

        assert_eq!(config.env, Env::Development);
        assert_eq!(config.log_format, LogFormat::Json);
        assert_eq!(config.server.bind_address, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(config.server.origin.as_str(), "https://portal.example.com");
        assert_eq!(config.database.url.expose_secret(), DATABASE_URL);
        assert_eq!(config.database.pool_size.get(), 3);
        assert_eq!(config.github.client_id, "client-id");
        assert_eq!(
            config.admin_api.url.as_str(),
            "http://status-list-server:8002/"
        );
        assert_eq!(config.admin_api.principal, "portal");
        assert_eq!(config.admin_api.audience, "status-list-admin");
        assert_eq!(
            config.admin_api.signing_key_file,
            PathBuf::from("/run/secrets/admin-key.pem")
        );
        assert_eq!(config.admin_api.key_id, "portal-1");
        assert_eq!(config.admin_api.algorithm, SigningAlgorithm::EdDsa);
    }

    #[test]
    fn missing_required_settings_are_named() {
        for (variable, setting) in [
            ("APP_SERVER__ORIGIN", "server"),
            ("APP_DATABASE__URL", "database"),
            ("APP_SESSION__KEY", "session"),
            ("APP_GITHUB__CLIENT_SECRET", "client_secret"),
            ("APP_ADMIN_API__SIGNING_KEY_FILE", "signing_key_file"),
        ] {
            let error = load_without(variable).unwrap_err();

            assert!(error.to_string().contains(setting), "{variable}: {error}");
        }
    }

    #[test]
    fn debug_output_hides_secrets() {
        let config = load(&[]).unwrap();

        let debug = format!("{config:?}");

        assert!(!debug.contains("s3cr3t"), "{debug}");
    }

    #[test]
    fn origins_are_normalized_like_the_origin_header() {
        let origin = PublicOrigin::parse("HTTPS://Portal.Example.com:443/").unwrap();

        assert_eq!(origin.as_str(), "https://portal.example.com");
        assert_eq!(
            origin.url_for("/api/v1/me"),
            "https://portal.example.com/api/v1/me"
        );
    }

    #[test]
    fn origins_have_no_path_query_or_credentials() {
        for input in [
            "https://portal.example.com/portal",
            "https://portal.example.com/?a=b",
            "https://user@portal.example.com",
            "ftp://portal.example.com",
            "portal.example.com",
        ] {
            assert!(PublicOrigin::parse(input).is_err(), "{input}");
        }
    }

    #[test]
    fn http_origins_are_only_for_loopback() {
        for input in [
            "http://localhost:8080",
            "http://127.0.0.1:8080",
            "http://[::1]:8080",
        ] {
            assert!(PublicOrigin::parse(input).is_ok(), "{input}");
        }
        let error = PublicOrigin::parse("http://portal.internal:8080").unwrap_err();
        assert!(error.contains("https"), "{error}");
    }

    #[test]
    fn session_keys_are_64_bytes_in_hex() {
        let passphrase = "correct horse battery staple ".repeat(5);
        let too_short = "ab".repeat(SESSION_KEY_LEN - 1);
        let too_long = "ab".repeat(SESSION_KEY_LEN + 1);

        for key in [passphrase.as_str(), &too_short, &too_long] {
            let error = load(&[("APP_SESSION__KEY", key)]).unwrap_err();

            assert!(error.to_string().contains("session.key"), "{error}");
            assert!(!error.to_string().contains(key), "{error}");
        }
    }

    #[test]
    fn debug_output_hides_the_session_key() {
        let config = load(&[]).unwrap();

        assert!(!format!("{config:?}").contains("0123456789abcdef"));
    }

    #[test]
    fn the_pool_needs_at_least_one_connection() {
        let error = load(&[("APP_DATABASE__POOL_SIZE", "0")]).unwrap_err();

        assert!(error.to_string().contains("database.pool_size"), "{error}");
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
        let vars = REQUIRED
            .iter()
            .filter(|(name, _)| *name != "APP_DATABASE__URL")
            .chain(&[("APP_DATABASE", DATABASE_URL)]);

        let error = load_exactly(vars).unwrap_err();

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

    #[test]
    fn the_admin_api_is_reached_over_http() {
        for url in [
            "status-list-server:8002",
            "ftp://status-list-server",
            "nowhere",
            "http://portal:s3cr3t@status-list-server:8002",
            "http://status-list-server:8002/?debug=1",
            "http://status-list-server:8002/#admin",
        ] {
            let error = load(&[("APP_ADMIN_API__URL", url)]).unwrap_err();

            assert!(
                error.to_string().contains("admin_api.url"),
                "{url}: {error}"
            );
        }
    }

    #[test]
    fn admin_keys_use_an_algorithm_the_server_accepts() {
        let error = load(&[("APP_ADMIN_API__ALGORITHM", "HS256")]).unwrap_err();

        assert!(error.to_string().contains("admin_api.algorithm"), "{error}");
    }
}
