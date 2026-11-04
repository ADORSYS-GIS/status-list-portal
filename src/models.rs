//! Rows of the portal's tables, and the queries that read and write them.

mod terms_acceptance;
mod user;

pub use self::terms_acceptance::TermsAcceptance;
pub use self::user::{NewUser, User};
