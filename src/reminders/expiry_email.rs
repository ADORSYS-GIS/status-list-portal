//! The expiry reminder, as developers read it.

use anyhow::{Context, anyhow};
use askama::Template;
use time::format_description::{self, BorrowedFormatItem};
use time::{Duration, OffsetDateTime, UtcOffset};
use url::Url;

use crate::admin_api::Issuer;
use crate::config::PublicOrigin;
use crate::email::Email;

/// Where the portal serves the wordmark the emails show: the frontend's
/// `static/email/adorsys-logo.png`. Mail clients do not show SVG.
const LOGO_PATH: &str = "/email/adorsys-logo.png";
/// The expiry as the email gives it, such as "Tuesday, 1 December 2026, 14:05 UTC".
const EXPIRY_FORMAT: &str =
    "[weekday], [day padding:none] [month repr:long] [year], [hour]:[minute] UTC";

/// What every email shows around its content.
struct Layout {
    origin: String,
    logo_url: String,
}

/// The reminder that one issuer expires soon.
pub(super) struct ExpiryEmail<'a> {
    layout: Layout,
    /// The issuer's display name, or its id when it has none.
    issuer_name: &'a str,
    issuer_id: &'a str,
    /// The expiry, in UTC.
    expires: String,
    /// When that is from now, such as "in 7 days".
    when: String,
    issuer_url: String,
    /// The host the email's `Message-ID` names.
    host: String,
}

#[derive(Template)]
#[template(path = "email/expiry_reminder.html")]
struct Html<'a> {
    layout: &'a Layout,
    email: &'a ExpiryEmail<'a>,
}

#[derive(Template)]
#[template(path = "email/expiry_reminder.txt")]
struct Text<'a> {
    layout: &'a Layout,
    email: &'a ExpiryEmail<'a>,
}

impl<'a> ExpiryEmail<'a> {
    pub(super) fn new(
        issuer: &'a Issuer,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
        origin: &PublicOrigin,
    ) -> anyhow::Result<Self> {
        let format: Vec<BorrowedFormatItem<'_>> =
            format_description::parse_borrowed::<1>(EXPIRY_FORMAT)?;
        let origin_url = Url::parse(origin.as_str()).context("the origin is not a URL")?;
        let mut issuer_url = origin_url.clone();
        issuer_url
            .path_segments_mut()
            .map_err(|()| anyhow!("the origin cannot have a path"))?
            .pop_if_empty()
            .push("issuers")
            .push(&issuer.id);
        Ok(Self {
            layout: Layout {
                origin: origin.as_str().to_owned(),
                logo_url: origin.url_for(LOGO_PATH),
            },
            issuer_name: issuer.display_name.as_deref().unwrap_or(&issuer.id),
            issuer_id: &issuer.id,
            expires: expires_at.to_offset(UtcOffset::UTC).format(&format)?,
            when: when(expires_at - now),
            issuer_url: issuer_url.into(),
            host: origin_url
                .host_str()
                .context("the origin has no host")?
                .to_owned(),
        })
    }

    fn subject(&self) -> String {
        // Display names hold no control characters, and a subject is one line.
        let name: String = self
            .issuer_name
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        format!("Your sandbox issuer \"{name}\" expires {}", self.when)
    }

    /// The email to `to`, whose `Message-ID` names the reminder `reminder_id`.
    pub(super) fn into_email(self, to: &str, reminder_id: i64) -> anyhow::Result<Email> {
        let html = Html {
            layout: &self.layout,
            email: &self,
        }
        .render()?;
        let text = Text {
            layout: &self.layout,
            email: &self,
        }
        .render()?;
        Ok(Email {
            to: to.to_owned(),
            subject: self.subject(),
            html,
            text,
            message_id: format!("<reminder.{reminder_id}@{}>", self.host),
        })
    }
}

/// When an issuer expires, `left` from now, rounded to the nearest day.
fn when(left: Duration) -> String {
    match (left + Duration::hours(12)).whole_days() {
        ..=0 => "in less than 12 hours".to_owned(),
        1 => "in about a day".to_owned(),
        days => format!("in {days} days"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://portal.example.com";
    const ISSUER_ID: &str = "6f1c2a9e-0b7d-4c1e-9a53-2f8d7e4b1c00";

    fn issuer(display_name: Option<&str>) -> Issuer {
        serde_json::from_value(serde_json::json!({
            "id": ISSUER_ID,
            "aggregation_id": "0b8ef1c4-3c6a-4d2e-9f1a-5b7c9d2e4f60",
            "display_name": display_name,
            "source": "portal",
            "tier": "sandbox",
            "state": "active",
            "owner": "github|42",
            "created_at": null,
            "expires_at": null,
            "keys": [],
            "usage": { "lists": 0, "lists_limit": 10, "keys": 0, "keys_limit": null },
        }))
        .unwrap()
    }

    fn email(issuer: &Issuer, left: Duration) -> Email {
        // Tuesday, 1 December 2026, 14:05 UTC.
        let expires_at = OffsetDateTime::from_unix_timestamp(1_796_133_900).unwrap();
        ExpiryEmail::new(
            issuer,
            expires_at,
            expires_at - left,
            &PublicOrigin::parse(ORIGIN).unwrap(),
        )
        .unwrap()
        .into_email("dev@example.com", 17)
        .unwrap()
    }

    #[test]
    fn the_reminder_renders_as_html_and_text() {
        let issuer = issuer(Some("Acme Wallet Issuer"));
        let email = email(&issuer, Duration::days(7) - Duration::minutes(30));

        assert_eq!(
            email.subject,
            "Your sandbox issuer \"Acme Wallet Issuer\" expires in 7 days"
        );
        assert_eq!(email.message_id, "<reminder.17@portal.example.com>");
        insta::assert_snapshot!("expiry_reminder_html", email.html);
        insta::assert_snapshot!("expiry_reminder_text", email.text);
    }

    #[test]
    fn the_developers_text_is_escaped_in_html() {
        let issuer = issuer(Some("<b>Acme</b> & \"Co\""));
        let email = email(&issuer, Duration::days(1));

        assert!(!email.html.contains("<b>Acme"), "{}", email.html);
        assert!(
            email.text.contains("<b>Acme</b> & \"Co\""),
            "{}",
            email.text
        );
    }

    #[test]
    fn issuers_without_a_name_go_by_their_id() {
        let issuer = issuer(None);
        let email = email(&issuer, Duration::days(1));

        assert_eq!(
            email.subject,
            format!("Your sandbox issuer \"{ISSUER_ID}\" expires in about a day")
        );
    }

    #[test]
    fn the_time_left_is_rounded_to_days() {
        assert_eq!(when(Duration::hours(11)), "in less than 12 hours");
        assert_eq!(when(Duration::hours(12)), "in about a day");
        assert_eq!(when(Duration::hours(35)), "in about a day");
        assert_eq!(when(Duration::hours(36)), "in 2 days");
        assert_eq!(when(Duration::days(7) - Duration::hours(1)), "in 7 days");
    }

    #[test]
    fn issuer_ids_are_encoded_in_the_link() {
        let mut issuer = issuer(None);
        issuer.id = "a b/c".to_owned();
        let email = email(&issuer, Duration::days(1));

        assert!(
            email.text.contains(&format!("{ORIGIN}/issuers/a%20b%2Fc")),
            "{}",
            email.text
        );
    }
}
