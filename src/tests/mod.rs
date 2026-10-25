//! Integration tests. Requests run through the full router, middleware included; the admin
//! API client is called directly.

mod admin_api;
mod cross_origin;
mod frontend;
mod headers;
mod health;
mod issuers;
mod me;
mod method_not_allowed;
mod migrations;
mod not_found;
mod pool;
mod registrations;
mod session;
mod telemetry;
mod terms;
mod users;
mod util;
