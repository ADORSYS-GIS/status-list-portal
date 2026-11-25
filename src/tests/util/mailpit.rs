//! Mailpit, the SMTP server the tests send email to: `docker compose up -d mailpit`.
//!
//! `TEST_SMTP_URL` and `TEST_MAILPIT_URL`, from the environment or from `.env`, name its
//! SMTP server and its API. Every test sends to addresses of its own, since Mailpit keeps
//! every test's messages. Chaos, which makes Mailpit refuse emails, applies to all of them,
//! so a test that turns it on runs alone.

use std::sync::LazyLock;

use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use status_list_portal::Env;
use status_list_portal::config;
use status_list_portal::email::Mailer;
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use url::Url;

use super::test_setting;

pub const SENDER: &str = "Status List Portal <noreply@portal.example.com>";

static MAILPIT: LazyLock<Mailpit> = LazyLock::new(Mailpit::from_settings);
/// Read by tests that send email, written by tests that turn chaos on.
static CHAOS: RwLock<()> = RwLock::const_new(());

pub struct Mailpit {
    smtp_url: String,
    api: Url,
    http: reqwest::Client,
}

/// A message as Mailpit lists it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Summary {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "MessageID")]
    pub message_id: String,
    pub subject: String,
}

#[derive(Debug, Deserialize)]
struct Messages {
    messages: Vec<Summary>,
}

/// A message's parts, as Mailpit reads them.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Message {
    #[serde(rename = "HTML")]
    pub html: String,
    pub text: String,
}

/// Mailpit refuses every recipient until this is dropped.
pub struct Chaos {
    _alone: RwLockWriteGuard<'static, ()>,
}

impl Mailpit {
    /// Mailpit, for a test that sends email while no chaos is on.
    pub async fn get() -> (&'static Mailpit, RwLockReadGuard<'static, ()>) {
        let sending = CHAOS.read().await;
        // A test run that was killed leaves chaos on.
        MAILPIT.set_recipient_chaos(451, 0).await;
        (&MAILPIT, sending)
    }

    fn from_settings() -> Self {
        let setting = |name: &str| {
            test_setting(name)
                .unwrap_or_else(|| panic!("{name} must be set, in the environment or in .env"))
        };
        Self {
            smtp_url: setting("TEST_SMTP_URL"),
            api: Url::parse(&setting("TEST_MAILPIT_URL")).expect("TEST_MAILPIT_URL is a URL"),
            // Each test has a runtime of its own, and a pooled connection works only on the
            // runtime that opened it.
            http: reqwest::Client::builder()
                .pool_max_idle_per_host(0)
                .build()
                .expect("failed to build the HTTP client"),
        }
    }

    /// A mailer that sends to Mailpit.
    pub fn mailer(&self) -> Mailer {
        Self::mailer_for(&self.smtp_url)
    }

    /// A mailer that sends to the SMTP server at `smtp_url`.
    pub fn mailer_for(smtp_url: &str) -> Mailer {
        let config = config::Email {
            smtp_url: SecretString::from(smtp_url),
            from: SENDER.parse().expect("the sender is valid"),
            reply_to: None,
        };
        Mailer::new(&config, Env::Test).expect("the mailer is valid")
    }

    /// The messages to `address`, newest first.
    pub async fn messages_to(&self, address: &str) -> Vec<Summary> {
        let mut url = self.api.join("api/v1/search").unwrap();
        url.query_pairs_mut()
            .append_pair("query", &format!("to:\"{address}\""));
        let messages: Messages = self.get_json(url).await;
        messages.messages
    }

    pub async fn message(&self, id: &str) -> Message {
        self.get_json(self.api.join(&format!("api/v1/message/{id}")).unwrap())
            .await
    }

    /// Mailpit, refusing every recipient with `code` until the guard is dropped. Waits for
    /// the tests that send email to end, and holds off the others.
    pub async fn refusing_recipients(code: u16) -> (&'static Mailpit, Chaos) {
        let alone = CHAOS.write().await;
        MAILPIT.set_recipient_chaos(code, 100).await;
        (&MAILPIT, Chaos { _alone: alone })
    }

    async fn set_recipient_chaos(&self, code: u16, probability: u8) {
        let body = json!({ "Recipient": { "ErrorCode": code, "Probability": probability } });
        self.http
            .put(self.api.join("api/v1/chaos").unwrap())
            .json(&body)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .expect("Mailpit's chaos must be on: set MP_ENABLE_CHAOS");
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: Url) -> T {
        self.http
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .expect("Mailpit is not reachable: `docker compose up -d mailpit`")
            .json()
            .await
            .expect("Mailpit sent unexpected JSON")
    }
}

impl Drop for Chaos {
    /// Turns chaos off, also when the test panics. Drop cannot wait, so a thread of its own
    /// does.
    fn drop(&mut self) {
        let reset = std::thread::spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to start a runtime")
                .block_on(MAILPIT.set_recipient_chaos(451, 0));
        });
        if reset.join().is_err() && !std::thread::panicking() {
            panic!("failed to turn Mailpit's chaos off");
        }
    }
}
