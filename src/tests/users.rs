use crate::util::TestDatabase;
use status_list_portal::models::{NewUser, User};

const OCTOCAT: NewUser<'static> = NewUser {
    gh_id: 583231,
    gh_login: "octocat",
    name: Some("The Octocat"),
    email: Some("octocat@example.com"),
};

#[tokio::test]
async fn first_sign_in_creates_the_account() {
    let db = TestDatabase::new().await;
    let mut conn = db.connect().await;

    let user = OCTOCAT.insert_or_update(&mut conn).await.unwrap();

    assert_eq!(user.gh_id, 583231);
    assert_eq!(user.gh_login, "octocat");
    assert_eq!(user.name.as_deref(), Some("The Octocat"));
    assert_eq!(user.email.as_deref(), Some("octocat@example.com"));
    assert_eq!(User::find(&mut conn, user.id).await.unwrap(), user);
}

#[tokio::test]
async fn later_sign_ins_refresh_the_account_details() {
    let db = TestDatabase::new().await;
    let mut conn = db.connect().await;
    let first = OCTOCAT.insert_or_update(&mut conn).await.unwrap();

    let renamed = NewUser {
        gh_login: "octocat-renamed",
        name: None,
        email: None,
        ..OCTOCAT
    };
    let second = renamed.insert_or_update(&mut conn).await.unwrap();

    assert_eq!(second.id, first.id);
    assert_eq!(second.gh_login, "octocat-renamed");
    assert_eq!(second.name, None);
    assert_eq!(second.email, None);
    assert_eq!(second.created_at, first.created_at);
}

#[tokio::test]
async fn unknown_ids_are_not_found() {
    let db = TestDatabase::new().await;
    let mut conn = db.connect().await;

    let result = User::find(&mut conn, 1).await;

    assert!(
        matches!(result, Err(diesel::result::Error::NotFound)),
        "{result:?}"
    );
}
