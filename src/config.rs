//! Configuration, read once at startup.
//!
//! Every setting comes from an environment variable named `APP_` followed by the setting's
//! path in upper case, with `__` between nested keys: `server.bind_address` is read from
//! `APP_SERVER__BIND_ADDRESS`. An empty variable counts as unset. `.env.template` documents
//! every setting. Secrets can also come from files: see [`secret_files`].

mod secret_files;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::{NonZeroU16, NonZeroU32, NonZeroUsize};
use std::path::PathBuf;

use ::config::{ConfigError, Environment};
use axum_extra::extract::cookie::Key;
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, de};
use url::{Host, Url};

use crate::Env;
use crate::admin_api::SigningAlgorithm;
use crate::client_address::TrustedProxies;
use crate::util::logging::LogFormat;

const DEFAULT_BIND_ADDRESS: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);
const DEFAULT_DATABASE_POOL_SIZE: NonZeroUsize = NonZeroUsize::new(10).unwrap();
/// Bytes in the session key: half signs the cookies, half encrypts them.
const SESSION_KEY_LEN: usize = 64;
const DEFAULT_MAX_ISSUERS_PER_ACCOUNT: usize = 2;
const DEFAULT_MAX_REGISTRATIONS_PER_ACCOUNT: usize = 10;
const DEFAULT_MAX_ACTIVE_ISSUERS: usize = 100;
/// The server's longest lifetime for the sandbox tier.
const DEFAULT_SANDBOX_LIFETIME_DAYS: NonZeroU16 = NonZeroU16::new(30).unwrap();

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
    pub management_api: ManagementApi,
    #[serde(default)]
    pub sandbox: Sandbox,
    pub terms: Terms,
    /// How often developers may sign in and change their issuers (`APP_RATE_LIMITS__*`).
    pub rate_limits: RateLimits,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    /// Address the HTTP listener binds to (`APP_SERVER__BIND_ADDRESS`).
    #[serde(default = "default_bind_address")]
    pub bind_address: SocketAddr,
    /// Where browsers reach the portal (`APP_SERVER__ORIGIN`).
    pub origin: PublicOrigin,
    /// The built frontend, `frontend/build` (`APP_SERVER__FRONTEND_DIR`). Unset, the server
    /// answers API requests only, as when the frontend's dev server runs in front of it.
    #[serde(default)]
    pub frontend_dir: Option<PathBuf>,
    /// The proxies in front of the server, whose `X-Forwarded-For` names the client
    /// (`APP_SERVER__TRUSTED_PROXIES`): addresses or CIDR blocks, separated by commas. Behind
    /// a proxy that is not listed, every client shares the proxy's sign-in limit.
    #[serde(default)]
    pub trusted_proxies: TrustedProxies,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Database {
    /// Connection URL (`APP_DATABASE__URL`), for example
    /// `postgres://portal:password@localhost:5432/portal`.
    pub url: SecretString,
    /// Optional PEM bundle of trusted Postgres CAs, replacing system roots
    /// (`APP_DATABASE__TLS_CA_FILE`).
    pub tls_ca_file: Option<PathBuf>,
    /// Most connections the pool keeps open (`APP_DATABASE__POOL_SIZE`).
    #[serde(default = "default_database_pool_size")]
    pub pool_size: NonZeroUsize,
}

/// Settings needed by the standalone migration job. Other application secrets are not needed.
#[derive(Debug, Deserialize)]
pub struct MigrationConfig {
    #[serde(default)]
    pub env: Env,
    #[serde(default)]
    pub log_format: LogFormat,
    pub database: Database,
}

/// The only secret the migration job needs: the other secrets' files need not be mounted.
const MIGRATION_SECRETS: [&str; 1] = ["database.url"];

impl MigrationConfig {
    /// Reads the database settings as [`Config::from_environment`] does.
    pub fn from_environment() -> Result<Self, ConfigError> {
        Self::from_vars(process_vars())
    }

    fn from_vars(vars: HashMap<String, String>) -> Result<Self, ConfigError> {
        load(vars, &MIGRATION_SECRETS)
    }
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
    /// Where GitHub serves its sign-in pages (`APP_GITHUB__WEB_URL`), ending with `/`. Only
    /// tests point it elsewhere, at a stand-in for GitHub; production refuses that.
    #[serde(default = "default_github_web_url", deserialize_with = "http_url")]
    pub web_url: Url,
    /// Where GitHub serves its REST API (`APP_GITHUB__API_URL`), ending with `/`. Only tests
    /// point it elsewhere.
    #[serde(default = "default_github_api_url", deserialize_with = "http_url")]
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

/// The Status List Server's management API, as developers' issuers reach it. The portal
/// shows both settings next to every issuer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementApi {
    /// Public URL of the management API (`APP_MANAGEMENT_API__URL`).
    #[serde(deserialize_with = "http_url")]
    pub url: Url,
    /// The `aud` that management tokens name: one of the server's
    /// `management_auth.audiences` (`APP_MANAGEMENT_API__AUDIENCE`).
    pub audience: String,
}

/// Limits on sandbox issuers. A limit of 0 stops new registrations.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sandbox {
    /// Most active issuers one account may have (`APP_SANDBOX__MAX_ISSUERS_PER_ACCOUNT`).
    #[serde(default = "default_max_issuers_per_account")]
    pub max_issuers_per_account: usize,
    /// Most issuers one account may register, deleted ones included
    /// (`APP_SANDBOX__MAX_REGISTRATIONS_PER_ACCOUNT`). The admin API keeps every issuer, so
    /// this bounds what one account can leave behind.
    #[serde(default = "default_max_registrations_per_account")]
    pub max_registrations_per_account: usize,
    /// Most active sandbox issuers all accounts together may have
    /// (`APP_SANDBOX__MAX_ACTIVE_ISSUERS`). Each registration counts them through the admin
    /// API, 100 to a call, so the cost of a registration grows with this number.
    #[serde(default = "default_max_active_issuers")]
    pub max_active_issuers: usize,
    /// Days a new or renewed issuer stays active (`APP_SANDBOX__LIFETIME_DAYS`). At most
    /// the server's longest lifetime for the sandbox tier.
    #[serde(default = "default_sandbox_lifetime_days")]
    pub lifetime_days: NonZeroU16,
    /// Whether an operator approves every registration (`APP_SANDBOX__MANUAL_APPROVAL`).
    /// Off by default: registering creates the issuer at once.
    #[serde(default)]
    pub manual_approval: bool,
}

impl Default for Sandbox {
    fn default() -> Self {
        Self {
            max_issuers_per_account: default_max_issuers_per_account(),
            max_registrations_per_account: default_max_registrations_per_account(),
            max_active_issuers: default_max_active_issuers(),
            lifetime_days: default_sandbox_lifetime_days(),
            manual_approval: false,
        }
    }
}

/// The terms of use developers accept before they register an issuer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terms {
    /// Version of the current terms (`APP_TERMS__VERSION`). Changing it asks every developer
    /// to accept again.
    pub version: String,
    /// Where the current terms are published (`APP_TERMS__URL`).
    #[serde(deserialize_with = "http_url")]
    pub url: Url,
}

/// How often a client may do something: `burst` times in a row, then once every
/// `refill_seconds` (`APP_RATE_LIMITS__<ACTION>__BURST` and `__REFILL_SECONDS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimit {
    pub burst: NonZeroU32,
    pub refill_seconds: NonZeroU32,
}

impl RateLimit {
    /// Panics if either number is 0.
    pub const fn new(burst: u32, refill_seconds: u32) -> Self {
        Self {
            burst: NonZeroU32::new(burst).expect("the burst is not 0"),
            refill_seconds: NonZeroU32::new(refill_seconds).expect("the interval is not 0"),
        }
    }

    pub fn refill_interval(self) -> time::Duration {
        time::Duration::seconds(self.refill_seconds.get().into())
    }
}

/// Rate limits: per developer for changes to their issuers, and per client network for
/// sign-ins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimits {
    /// Registering an issuer, at once or for approval.
    pub register: RateLimit,
    pub renew: RateLimit,
    /// Adding a signing key.
    pub add_key: RateLimit,
    /// Revoking a signing key, independently of additions.
    pub revoke_key: RateLimit,
    /// Deleting an issuer.
    pub delete: RateLimit,
    /// Finishing a sign-in with GitHub.
    pub sign_in: RateLimit,
}

impl Default for RateLimits {
    fn default() -> Self {
        Self {
            register: RateLimit::new(5, 600),
            renew: RateLimit::new(10, 60),
            add_key: RateLimit::new(10, 60),
            revoke_key: RateLimit::new(20, 30),
            delete: RateLimit::new(20, 30),
            sign_in: RateLimit::new(20, 6),
        }
    }
}

/// The settings that have defaults one variable can override without the others, such as
/// `APP_RATE_LIMITS__REGISTER__BURST` alone.
#[derive(Serialize)]
struct Defaults {
    rate_limits: RateLimits,
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

    /// The scheme browsers use: `https`, or `http` on loopback.
    pub fn scheme(&self) -> &'static str {
        if self.0.starts_with("https:") {
            "https"
        } else {
            "http"
        }
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
    /// Reads the configuration from the process environment, and the secret files it names.
    /// Variables whose names or values are not Unicode are left out.
    pub fn from_environment() -> Result<Self, ConfigError> {
        Self::from_vars(process_vars())
    }

    /// Reads the configuration from `vars`, as if they were the process environment.
    fn from_vars(vars: HashMap<String, String>) -> Result<Self, ConfigError> {
        load::<Self>(vars, &secret_files::SECRET_SETTINGS)?.refuse_stand_ins()
    }

    /// Refuses, in production, a stand-in for GitHub: end-to-end tests sign in through one.
    fn refuse_stand_ins(self) -> Result<Self, ConfigError> {
        let github = &self.github;
        let stand_in = github.web_url != default_github_web_url()
            || github.api_url != default_github_api_url();
        if self.env == Env::Production && stand_in {
            return Err(ConfigError::Message(
                "APP_GITHUB__WEB_URL and APP_GITHUB__API_URL are for tests: in production, the \
                 portal signs developers in with GitHub itself"
                    .to_string(),
            ));
        }
        Ok(self)
    }
}

/// The process environment. Variables whose names or values are not Unicode are left out.
fn process_vars() -> HashMap<String, String> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

/// Reads settings from `vars`, as if they were the process environment, and from the files
/// they name for `secrets`.
fn load<T: DeserializeOwned>(
    vars: HashMap<String, String>,
    secrets: &[&'static str],
) -> Result<T, ConfigError> {
    let defaults = ::config::Config::try_from(&Defaults {
        rate_limits: RateLimits::default(),
    })?;
    let builder = secret_files::read(&vars, secrets)?
        .into_iter()
        .try_fold(::config::Config::builder(), |builder, (setting, value)| {
            builder.set_override(setting, value)
        })?;
    // The `_FILE` variables are read above; as settings, they would be unknown.
    let vars = vars
        .into_iter()
        .filter(|(name, _)| !secret_files::is_file_variable(name, secrets))
        .collect();
    builder
        .add_source(defaults)
        .add_source(environment().source(Some(vars)))
        .build()?
        .try_deserialize()
        .map_err(without_value)
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

fn default_max_issuers_per_account() -> usize {
    DEFAULT_MAX_ISSUERS_PER_ACCOUNT
}

fn default_max_registrations_per_account() -> usize {
    DEFAULT_MAX_REGISTRATIONS_PER_ACCOUNT
}

fn default_max_active_issuers() -> usize {
    DEFAULT_MAX_ACTIVE_ISSUERS
}

fn default_sandbox_lifetime_days() -> NonZeroU16 {
    DEFAULT_SANDBOX_LIFETIME_DAYS
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
        ("APP_MANAGEMENT_API__URL", "https://status.example.com"),
        ("APP_MANAGEMENT_API__AUDIENCE", "https://status.example.com"),
        ("APP_TERMS__VERSION", "2026-10-22"),
        ("APP_TERMS__URL", "https://portal.example.com/terms"),
    ];

    fn load_exactly<'a>(
        vars: impl IntoIterator<Item = &'a (&'a str, &'a str)>,
    ) -> Result<Config, ConfigError> {
        let vars = vars
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        Config::from_vars(vars)
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
        assert_eq!(config.server.frontend_dir, None);
        assert_eq!(config.database.pool_size, DEFAULT_DATABASE_POOL_SIZE);
        assert_eq!(config.github.web_url.as_str(), "https://github.com/");
        assert_eq!(config.github.api_url.as_str(), "https://api.github.com/");
        assert_eq!(config.admin_api.algorithm, SigningAlgorithm::Es256);
        assert_eq!(config.sandbox.max_issuers_per_account, 2);
        assert_eq!(config.sandbox.max_registrations_per_account, 10);
        assert_eq!(config.sandbox.max_active_issuers, 100);
        assert_eq!(config.sandbox.lifetime_days.get(), 30);
        assert!(!config.sandbox.manual_approval);
    }

    #[test]
    fn migration_configuration_needs_only_database_settings() {
        let vars = [
            ("APP_DATABASE__URL".to_owned(), DATABASE_URL.to_owned()),
            (
                "APP_DATABASE__TLS_CA_FILE".to_owned(),
                "/run/secrets/ca.pem".to_owned(),
            ),
            ("APP_LOG_FORMAT".to_owned(), "json".to_owned()),
        ];
        let config = MigrationConfig::from_vars(vars.into_iter().collect()).unwrap();
        assert_eq!(config.env, Env::Production);
        assert_eq!(config.log_format, LogFormat::Json);
        assert_eq!(config.database.url.expose_secret(), DATABASE_URL);
        assert_eq!(
            config.database.tls_ca_file,
            Some(PathBuf::from("/run/secrets/ca.pem"))
        );
    }

    #[test]
    fn reads_prefixed_and_nested_variables() {
        let config = load(&[
            ("APP_ENV", "development"),
            ("APP_LOG_FORMAT", "json"),
            ("APP_SERVER__BIND_ADDRESS", "0.0.0.0:9000"),
            ("APP_SERVER__FRONTEND_DIR", "frontend/build"),
            ("APP_DATABASE__POOL_SIZE", "3"),
            ("APP_ADMIN_API__ALGORITHM", "EdDSA"),
            ("APP_SANDBOX__MAX_ISSUERS_PER_ACCOUNT", "1"),
            ("APP_SANDBOX__MAX_REGISTRATIONS_PER_ACCOUNT", "4"),
            ("APP_SANDBOX__MAX_ACTIVE_ISSUERS", "500"),
            ("APP_SANDBOX__LIFETIME_DAYS", "7"),
            ("APP_SANDBOX__MANUAL_APPROVAL", "true"),
        ])
        .unwrap();

        assert_eq!(config.env, Env::Development);
        assert_eq!(config.log_format, LogFormat::Json);
        assert_eq!(config.server.bind_address, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(config.server.origin.as_str(), "https://portal.example.com");
        assert_eq!(
            config.server.frontend_dir,
            Some(PathBuf::from("frontend/build"))
        );
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
        assert_eq!(
            config.management_api.url.as_str(),
            "https://status.example.com/"
        );
        assert_eq!(config.management_api.audience, "https://status.example.com");
        assert_eq!(config.sandbox.max_issuers_per_account, 1);
        assert_eq!(config.sandbox.max_registrations_per_account, 4);
        assert_eq!(config.sandbox.max_active_issuers, 500);
        assert_eq!(config.sandbox.lifetime_days.get(), 7);
        assert!(config.sandbox.manual_approval);
        assert_eq!(config.terms.version, "2026-10-22");
        assert_eq!(
            config.terms.url.as_str(),
            "https://portal.example.com/terms"
        );
    }

    #[test]
    fn missing_required_settings_are_named() {
        for (variable, setting) in [
            ("APP_SERVER__ORIGIN", "server"),
            ("APP_DATABASE__URL", "database"),
            ("APP_SESSION__KEY", "session"),
            ("APP_GITHUB__CLIENT_SECRET", "client_secret"),
            ("APP_ADMIN_API__PRINCIPAL", "principal"),
            ("APP_ADMIN_API__AUDIENCE", "audience"),
            ("APP_ADMIN_API__SIGNING_KEY_FILE", "signing_key_file"),
            ("APP_ADMIN_API__KEY_ID", "key_id"),
            ("APP_MANAGEMENT_API__AUDIENCE", "audience"),
            ("APP_TERMS__VERSION", "version"),
        ] {
            let error = load_without(variable).unwrap_err();

            assert!(error.to_string().contains(setting), "{variable}: {error}");
        }
    }

    #[test]
    fn empty_required_settings_are_missing() {
        for (variable, setting) in [
            ("APP_GITHUB__CLIENT_ID", "github.client_id"),
            ("APP_ADMIN_API__PRINCIPAL", "admin_api.principal"),
            ("APP_ADMIN_API__AUDIENCE", "admin_api.audience"),
            ("APP_ADMIN_API__KEY_ID", "admin_api.key_id"),
        ] {
            let error = load(&[(variable, "")]).unwrap_err();

            let missing = format!("missing configuration field \"{setting}\"");
            assert!(error.to_string().contains(&missing), "{variable}: {error}");
        }
    }

    #[test]
    fn debug_output_hides_secrets() {
        let config = load(&[]).unwrap();

        let debug = format!("{config:?}");

        assert!(!debug.contains("s3cr3t"), "{debug}");
    }

    #[test]
    fn secrets_can_come_from_files() {
        let dir = std::env::temp_dir().join(format!("portal-secrets-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let key_file = dir.join("session-key");
        let secret_file = dir.join("client-secret");
        std::fs::write(&key_file, format!("{SESSION_KEY}\n")).unwrap();
        std::fs::write(&secret_file, "s3cr3t-from-file\n").unwrap();
        let from_files = [
            ("APP_SESSION__KEY_FILE", key_file.to_str().unwrap()),
            (
                "APP_GITHUB__CLIENT_SECRET_FILE",
                secret_file.to_str().unwrap(),
            ),
        ];
        let from_variables = ["APP_SESSION__KEY", "APP_GITHUB__CLIENT_SECRET"];

        let config = load_exactly(
            REQUIRED
                .iter()
                .filter(|(name, _)| !from_variables.contains(name))
                .chain(&from_files),
        );
        std::fs::remove_dir_all(&dir).unwrap();

        let config = config.unwrap();
        assert_eq!(
            config.github.client_secret.expose_secret(),
            "s3cr3t-from-file"
        );
        let from_variable = load(&[]).unwrap();
        assert_eq!(
            config.session.key.key().master(),
            from_variable.session.key.key().master()
        );
    }

    #[test]
    fn the_migration_job_reads_the_database_url_from_a_file() {
        let file = std::env::temp_dir().join(format!("portal-db-url-{}", uuid::Uuid::new_v4()));
        std::fs::write(&file, format!("{DATABASE_URL}\n")).unwrap();
        let vars = [(
            "APP_DATABASE__URL_FILE".to_owned(),
            file.to_str().unwrap().to_owned(),
        )];

        let config = MigrationConfig::from_vars(vars.into_iter().collect());
        std::fs::remove_file(&file).unwrap();

        assert_eq!(config.unwrap().database.url.expose_secret(), DATABASE_URL);
    }

    #[test]
    fn the_migration_job_needs_no_other_secret_file() {
        let vars = [
            ("APP_DATABASE__URL", DATABASE_URL),
            ("APP_SESSION__KEY_FILE", "/nonexistent/session-key"),
            (
                "APP_GITHUB__CLIENT_SECRET_FILE",
                "/nonexistent/client-secret",
            ),
        ];

        let config = MigrationConfig::from_vars(
            vars.into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
        );

        assert_eq!(config.unwrap().database.url.expose_secret(), DATABASE_URL);
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
    fn rate_limits_can_be_changed_one_number_at_a_time() {
        let config = load(&[("APP_RATE_LIMITS__REGISTER__BURST", "2")]).unwrap();

        assert_eq!(config.rate_limits.register, RateLimit::new(2, 600));
        let defaults = RateLimits::default();
        assert_eq!(config.rate_limits.renew, defaults.renew);
        assert_eq!(config.rate_limits.add_key, defaults.add_key);
        assert_eq!(config.rate_limits.revoke_key, defaults.revoke_key);
        assert_eq!(config.rate_limits.delete, defaults.delete);
        assert_eq!(config.rate_limits.sign_in, defaults.sign_in);

        let error = load(&[("APP_RATE_LIMITS__SIGN_IN__REFILL_SECONDS", "0")]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("rate_limits.sign_in.refill_seconds"),
            "{error}"
        );
    }

    #[test]
    fn github_can_be_stood_in_for_outside_production() {
        let stand_in = [
            ("APP_GITHUB__WEB_URL", "http://127.0.0.1:18090/"),
            ("APP_GITHUB__API_URL", "http://127.0.0.1:18090/api/"),
        ];

        let config = load(&[("APP_ENV", "development"), stand_in[0], stand_in[1]]).unwrap();

        assert_eq!(config.github.web_url.as_str(), "http://127.0.0.1:18090/");
        assert_eq!(
            config.github.api_url.as_str(),
            "http://127.0.0.1:18090/api/"
        );
        let in_production = [("APP_ENV", "production"), stand_in[1]];
        let error = load(&in_production).unwrap_err();
        assert!(error.to_string().contains("APP_GITHUB__WEB_URL"), "{error}");
        assert!(load(&[("APP_ENV", "production")]).is_ok());
    }

    #[test]
    fn trusted_proxies_are_none_unless_listed() {
        assert_eq!(
            load(&[]).unwrap().server.trusted_proxies,
            TrustedProxies::default()
        );

        let config = load(&[("APP_SERVER__TRUSTED_PROXIES", "10.0.0.0/8, 192.168.1.10")]).unwrap();
        let expected = TrustedProxies::parse("10.0.0.0/8,192.168.1.10").unwrap();
        assert_eq!(config.server.trusted_proxies, expected);

        let error = load(&[("APP_SERVER__TRUSTED_PROXIES", "the ingress")]).unwrap_err();
        assert!(
            error.to_string().contains("server.trusted_proxies"),
            "{error}"
        );
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
    fn renewals_last_at_least_a_day() {
        let error = load(&[("APP_SANDBOX__LIFETIME_DAYS", "0")]).unwrap_err();

        assert!(
            error.to_string().contains("sandbox.lifetime_days"),
            "{error}"
        );
    }

    #[test]
    fn admin_keys_use_an_algorithm_the_server_accepts() {
        let error = load(&[("APP_ADMIN_API__ALGORITHM", "HS256")]).unwrap_err();

        assert!(error.to_string().contains("admin_api.algorithm"), "{error}");
    }
}
