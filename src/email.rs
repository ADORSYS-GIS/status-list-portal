//! Email, sent through any SMTP server: a provider's, such as Mailgun, or Mailpit in
//! development. Changing provider changes the settings only.

use std::time::Duration;

use anyhow::{Context, bail};
use lettre::address::AddressError;
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::Error as SmtpError;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use secrecy::ExposeSecret;
use url::Url;

use crate::Env;
use crate::config;

/// How long the SMTP server may take to answer each command.
const SMTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Sends the portal's emails, from its configured sender.
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    reply_to: Option<Mailbox>,
}

/// An email to one recipient, with an HTML part and a plain-text one that says the same.
pub struct Email {
    /// The recipient's address.
    pub to: String,
    pub subject: String,
    pub html: String,
    pub text: String,
    /// The same for every try at the same email, so a mailbox keeps one copy of it.
    pub message_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("the recipient's address is invalid: {0}")]
    InvalidAddress(#[from] AddressError),
    #[error("the email could not be built")]
    Build(#[from] lettre::error::Error),
    /// The SMTP server refused the email, or could not be reached. Its answer can name the
    /// recipient.
    #[error(transparent)]
    Smtp(#[from] SmtpError),
}

impl SendError {
    /// Whether sending the same email again would fail the same way.
    pub fn is_permanent(&self) -> bool {
        match self {
            Self::InvalidAddress(_) | Self::Build(_) => true,
            Self::Smtp(error) => error.is_permanent(),
        }
    }

    /// The SMTP server's reply code, when it answered.
    pub fn smtp_code(&self) -> Option<u16> {
        match self {
            Self::Smtp(error) => error.status().map(u16::from),
            Self::InvalidAddress(_) | Self::Build(_) => None,
        }
    }
}

impl Mailer {
    /// Builds the mailer. Connects to nothing yet. In production, refuses an SMTP server
    /// reached in the clear, unless it listens on loopback, such as a relay in the same pod.
    pub fn new(config: &config::Email, env: Env) -> anyhow::Result<Self> {
        let smtp_url = config.smtp_url.expose_secret();
        // The URL holds the password, so no error message quotes it.
        let url = Url::parse(smtp_url).context("APP_EMAIL__SMTP_URL is not a URL")?;
        if env == Env::Production && !is_encrypted(&url) && !config::is_loopback(&url) {
            bail!(
                "APP_EMAIL__SMTP_URL sends email in the clear: use smtps:// or ?tls=required, \
                 unless the SMTP server listens on loopback"
            );
        }
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(smtp_url)
            .context("APP_EMAIL__SMTP_URL is not a valid SMTP URL")?
            .timeout(Some(SMTP_TIMEOUT))
            .build();
        Ok(Self {
            transport,
            from: config.from.clone(),
            reply_to: config.reply_to.clone(),
        })
    }

    /// Connects and signs in to the SMTP server, so that a server that is down or refuses
    /// the credentials fails before any email is tried.
    pub async fn check(&self) -> anyhow::Result<()> {
        let connected = self
            .transport
            .test_connection()
            .await
            .context("cannot reach the SMTP server")?;
        if !connected {
            bail!("the SMTP server closed the connection");
        }
        Ok(())
    }

    pub async fn send(&self, email: Email) -> Result<(), SendError> {
        let builder = Message::builder()
            .from(self.from.clone())
            .to(Mailbox::new(None, email.to.parse()?))
            .subject(email.subject)
            .message_id(Some(email.message_id));
        let builder = match &self.reply_to {
            Some(reply_to) => builder.reply_to(reply_to.clone()),
            None => builder,
        };
        let message =
            builder.multipart(MultiPart::alternative_plain_html(email.text, email.html))?;
        self.transport.send(message).await?;
        Ok(())
    }
}

/// Whether the URL asks for TLS from the start or for STARTTLS without falling back.
/// `?tls=opportunistic` falls back to the clear when the server offers no STARTTLS, so it
/// counts as unencrypted.
fn is_encrypted(url: &Url) -> bool {
    url.scheme() == "smtps"
        || url
            .query_pairs()
            .any(|(name, value)| name == "tls" && value == "required")
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn email_config(smtp_url: &str) -> config::Email {
        config::Email {
            smtp_url: SecretString::from(smtp_url),
            from: "Status List Portal <noreply@example.com>".parse().unwrap(),
            reply_to: None,
        }
    }

    #[test]
    fn production_refuses_smtp_in_the_clear() {
        for url in [
            "smtp://user:s3cr3t@smtp.example.com:587",
            "smtp://user:s3cr3t@smtp.example.com:587?tls=opportunistic",
        ] {
            let error = Mailer::new(&email_config(url), Env::Production)
                .err()
                .expect("plain SMTP is refused");
            let message = format!("{error:#}");
            assert!(message.contains("in the clear"), "{message}");
            assert!(!message.contains("s3cr3t"), "{message}");
        }
    }

    #[test]
    fn production_accepts_tls_and_loopback() {
        for url in [
            "smtps://user:s3cr3t@smtp.example.com",
            "smtp://user:s3cr3t@smtp.example.com:587?tls=required",
            "smtp://localhost:1025",
            "smtp://127.0.0.1:1025",
        ] {
            Mailer::new(&email_config(url), Env::Production).expect(url);
        }
    }

    #[test]
    fn development_accepts_smtp_in_the_clear() {
        Mailer::new(&email_config("smtp://mailpit:1025"), Env::Development).unwrap();
    }

    #[test]
    fn invalid_urls_are_refused_without_quoting_them() {
        for url in ["not a url s3cr3t", "https://user:s3cr3t@smtp.example.com"] {
            let error = Mailer::new(&email_config(url), Env::Development)
                .err()
                .expect("the URL is refused");
            assert!(!format!("{error:#}").contains("s3cr3t"), "{error:#}");
        }
    }
}
