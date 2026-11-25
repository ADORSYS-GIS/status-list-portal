//! Expiry reminders, sent to Mailpit. The admin API lists the sandbox issuers.

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde_json::{Value, json};
use status_list_portal::db;
use status_list_portal::email::Mailer;
use status_list_portal::models::User;
use status_list_portal::reminders::{Reminders, Summary};
use status_list_portal::schema::reminders;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use wiremock::Mock;
use wiremock::matchers::{method, path, query_param};

use crate::util::admin_api::{ISSUER_ID, issuer_json, page_response};
use crate::util::mailpit::Mailpit;
use crate::util::{ORIGIN, TestApp};

/// Nothing listens on port 1.
const UNREACHABLE_SMTP_URL: &str = "smtp://127.0.0.1:1";

/// When the tests send reminders, unless they say otherwise.
fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap()
}

/// A developer whose address no other test sends to, in this run or an earlier one.
async fn developer(app: &TestApp) -> User {
    let login = format!("dev-{}", Uuid::new_v4().simple());
    app.db_new_user(&login).await.user().clone()
}

fn issuer_expiring(owner: &User, expires_at: OffsetDateTime) -> Value {
    let mut issuer = issuer_json(owner.gh_id);
    issuer["expires_at"] = json!(expires_at.unix_timestamp());
    issuer
}

/// The admin API lists these issuers as the active sandbox issuers.
async fn admin_api_lists(app: &TestApp, issuers: &[Value]) {
    app.admin_api().reset().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .and(query_param("tier", "sandbox"))
        .and(query_param("state", "active"))
        .respond_with(page_response(issuers))
        .mount(app.admin_api())
        .await;
}

async fn send(app: &TestApp, mailer: &Mailer, now: OffsetDateTime) -> anyhow::Result<Summary> {
    let ctx = app.ctx();
    Reminders {
        db: &ctx.db,
        admin_api: &ctx.admin_api,
        mailer,
        origin: &ctx.config.server.origin,
    }
    .send_due(now)
    .await
}

fn sent(count: usize) -> Summary {
    Summary {
        sent: count,
        ..Summary::default()
    }
}

/// A row of the reminders table.
#[derive(Debug, PartialEq, Eq, Queryable)]
struct Row {
    window_days: i16,
    attempts: i16,
    sent: bool,
    given_up: bool,
    last_error: Option<String>,
}

async fn rows(app: &TestApp) -> Vec<Row> {
    reminders::table
        .order(reminders::id)
        .select((
            reminders::window_days,
            reminders::attempts,
            reminders::sent_at.is_not_null(),
            reminders::gave_up_at.is_not_null(),
            reminders::last_error,
        ))
        .load(&mut app.db_conn().await)
        .await
        .unwrap()
}

#[tokio::test]
async fn each_window_is_reminded_once() {
    let (mailpit, _sending) = Mailpit::get().await;
    let (app, _) = TestApp::init().empty().await;
    let developer = developer(&app).await;
    let expires_at = now() + Duration::days(6);
    admin_api_lists(&app, &[issuer_expiring(&developer, expires_at)]).await;

    assert_eq!(send(&app, &mailpit.mailer(), now()).await.unwrap(), sent(1));
    let an_hour_later = now() + Duration::hours(1);
    assert_eq!(
        send(&app, &mailpit.mailer(), an_hour_later).await.unwrap(),
        Summary::default()
    );
    let messages = mailpit.messages_to(&developer.email).await;
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].subject,
        "Your sandbox issuer \"Acme Wallet Issuer\" expires in 6 days"
    );

    let the_day_before = expires_at - Duration::hours(23);
    assert_eq!(
        send(&app, &mailpit.mailer(), the_day_before).await.unwrap(),
        sent(1)
    );
    let messages = mailpit.messages_to(&developer.email).await;
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0].subject,
        "Your sandbox issuer \"Acme Wallet Issuer\" expires in about a day"
    );
    let windows: Vec<i16> = rows(&app).await.iter().map(|row| row.window_days).collect();
    assert_eq!(windows, [7, 1]);
}

#[tokio::test]
async fn the_email_links_to_the_issuer_in_both_parts() {
    let (mailpit, _sending) = Mailpit::get().await;
    let (app, _) = TestApp::init().empty().await;
    let developer = developer(&app).await;
    admin_api_lists(
        &app,
        &[issuer_expiring(&developer, now() + Duration::days(7))],
    )
    .await;

    send(&app, &mailpit.mailer(), now()).await.unwrap();

    let messages = mailpit.messages_to(&developer.email).await;
    let [summary] = messages.as_slice() else {
        panic!("expected one message, got {messages:?}");
    };
    assert!(
        summary.message_id.starts_with("reminder.")
            && summary.message_id.ends_with("@portal.example.com"),
        "{}",
        summary.message_id
    );
    let message = mailpit.message(&summary.id).await;
    let link = format!("{ORIGIN}/issuers/{ISSUER_ID}");
    assert!(message.html.contains(&format!("href=\"{link}\"")));
    assert!(message.text.contains(&link));
}

#[tokio::test]
async fn a_renewed_issuer_is_reminded_again() {
    let (mailpit, _sending) = Mailpit::get().await;
    let (app, _) = TestApp::init().empty().await;
    let developer = developer(&app).await;
    let expires_at = now() + Duration::days(7);
    admin_api_lists(&app, &[issuer_expiring(&developer, expires_at)]).await;
    send(&app, &mailpit.mailer(), now()).await.unwrap();

    let renewed_until = expires_at + Duration::days(30);
    admin_api_lists(&app, &[issuer_expiring(&developer, renewed_until)]).await;
    let a_week_before = renewed_until - Duration::days(7);

    assert_eq!(
        send(&app, &mailpit.mailer(), a_week_before).await.unwrap(),
        sent(1)
    );
    assert_eq!(mailpit.messages_to(&developer.email).await.len(), 2);
}

#[tokio::test]
async fn only_the_nearest_window_is_sent_when_several_are_due() {
    let (mailpit, _sending) = Mailpit::get().await;
    let (app, _) = TestApp::init().empty().await;
    let developer = developer(&app).await;
    admin_api_lists(
        &app,
        &[issuer_expiring(&developer, now() + Duration::hours(12))],
    )
    .await;

    assert_eq!(send(&app, &mailpit.mailer(), now()).await.unwrap(), sent(1));
    assert_eq!(mailpit.messages_to(&developer.email).await.len(), 1);
    let windows: Vec<i16> = rows(&app).await.iter().map(|row| row.window_days).collect();
    assert_eq!(windows, [1]);
}

#[tokio::test]
async fn nothing_due_needs_no_smtp_server() {
    let (app, _) = TestApp::init().empty().await;
    let developer = developer(&app).await;
    let not_yet = issuer_expiring(&developer, now() + Duration::days(8));
    let mut static_issuer = issuer_expiring(&developer, now() + Duration::days(1));
    static_issuer["owner"] = Value::Null;
    admin_api_lists(&app, &[not_yet, static_issuer]).await;

    let mailer = Mailpit::mailer_for(UNREACHABLE_SMTP_URL);

    assert_eq!(
        send(&app, &mailer, now()).await.unwrap(),
        Summary::default()
    );
}

#[tokio::test]
async fn issuers_of_accounts_that_never_signed_in_are_skipped() {
    let (mailpit, _sending) = Mailpit::get().await;
    let (app, _) = TestApp::init().empty().await;
    let mut issuer = issuer_json(1);
    issuer["owner"] = json!("github|987654321");
    issuer["expires_at"] = json!((now() + Duration::days(1)).unix_timestamp());
    admin_api_lists(&app, &[issuer]).await;

    assert_eq!(
        send(&app, &mailpit.mailer(), now()).await.unwrap(),
        Summary::default()
    );
    assert_eq!(rows(&app).await, []);
}

#[tokio::test]
async fn an_unreachable_smtp_server_fails_the_run_before_any_try_is_counted() {
    let (app, _) = TestApp::init().empty().await;
    let developer = developer(&app).await;
    admin_api_lists(
        &app,
        &[issuer_expiring(&developer, now() + Duration::days(1))],
    )
    .await;
    let mailer = Mailpit::mailer_for(UNREACHABLE_SMTP_URL);

    let error = send(&app, &mailer, now()).await.unwrap_err();

    assert!(
        format!("{error:#}").contains("cannot reach the SMTP server"),
        "{error:#}"
    );
    assert_eq!(rows(&app).await, []);
}

#[tokio::test]
async fn an_unreachable_admin_api_fails_the_run() {
    let (app, _) = TestApp::init().empty().await;
    let mailer = Mailpit::mailer_for(UNREACHABLE_SMTP_URL);

    let error = send(&app, &mailer, now()).await.unwrap_err();

    assert!(
        format!("{error:#}").contains("cannot list the sandbox issuers"),
        "{error:#}"
    );
}

#[tokio::test]
async fn one_run_sends_at_a_time() {
    let (mailpit, _sending) = Mailpit::get().await;
    let (app, _) = TestApp::init().empty().await;
    let developer = developer(&app).await;
    admin_api_lists(
        &app,
        &[issuer_expiring(&developer, now() + Duration::days(1))],
    )
    .await;
    let mut other_run = app.db_conn().await;
    assert!(db::try_lock_reminders(&mut other_run).await.unwrap());

    assert_eq!(
        send(&app, &mailpit.mailer(), now()).await.unwrap(),
        Summary::default()
    );
    assert_eq!(mailpit.messages_to(&developer.email).await.len(), 0);

    assert!(db::unlock_reminders(&mut other_run).await.unwrap());
    assert_eq!(send(&app, &mailpit.mailer(), now()).await.unwrap(), sent(1));
}

#[tokio::test]
async fn a_temporary_failure_is_retried_by_the_next_run() {
    let (app, _) = TestApp::init().empty().await;
    let developer = developer(&app).await;
    admin_api_lists(
        &app,
        &[issuer_expiring(&developer, now() + Duration::days(1))],
    )
    .await;
    let (mailpit, chaos) = Mailpit::refusing_recipients(451).await;

    let summary = send(&app, &mailpit.mailer(), now()).await.unwrap();

    assert_eq!(
        summary,
        Summary {
            retrying: 1,
            ..Summary::default()
        }
    );
    drop(chaos);
    let (mailpit, _sending) = Mailpit::get().await;
    let an_hour_later = now() + Duration::hours(1);
    assert_eq!(
        send(&app, &mailpit.mailer(), an_hour_later).await.unwrap(),
        sent(1)
    );
    assert_eq!(mailpit.messages_to(&developer.email).await.len(), 1);
    let [row] = rows(&app).await.try_into().unwrap();
    assert_eq!((row.attempts, row.sent, row.last_error), (2, true, None));
}

#[tokio::test]
async fn a_reminder_is_given_up_after_four_failed_tries() {
    let (app, _) = TestApp::init().empty().await;
    let developer = developer(&app).await;
    admin_api_lists(
        &app,
        &[issuer_expiring(&developer, now() + Duration::days(1))],
    )
    .await;
    let (mailpit, _chaos) = Mailpit::refusing_recipients(451).await;
    let retrying = Summary {
        retrying: 1,
        ..Summary::default()
    };

    for hours in 0..3 {
        let at = now() + Duration::hours(hours);
        assert_eq!(send(&app, &mailpit.mailer(), at).await.unwrap(), retrying);
    }
    let fourth = send(&app, &mailpit.mailer(), now() + Duration::hours(3)).await;
    let fifth = send(&app, &mailpit.mailer(), now() + Duration::hours(4)).await;

    assert_eq!(
        fourth.unwrap(),
        Summary {
            given_up: 1,
            ..Summary::default()
        }
    );
    assert_eq!(fifth.unwrap(), Summary::default());
    let [row] = rows(&app).await.try_into().unwrap();
    assert_eq!((row.attempts, row.sent, row.given_up), (4, false, true));
    assert!(row.last_error.unwrap().contains("451"));
}

#[tokio::test]
async fn a_permanent_refusal_is_given_up_at_once() {
    let (app, _) = TestApp::init().empty().await;
    let developer = developer(&app).await;
    admin_api_lists(
        &app,
        &[issuer_expiring(&developer, now() + Duration::days(1))],
    )
    .await;
    let (mailpit, _chaos) = Mailpit::refusing_recipients(550).await;

    let summary = send(&app, &mailpit.mailer(), now()).await.unwrap();

    assert_eq!(
        summary,
        Summary {
            given_up: 1,
            ..Summary::default()
        }
    );
    let [row] = rows(&app).await.try_into().unwrap();
    assert_eq!((row.attempts, row.given_up), (1, true));
}
