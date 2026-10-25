//! The portal's metrics. Their names follow the OpenTelemetry semantic conventions where
//! one applies, and `docs/telemetry.md` lists them. [`crate::telemetry`] exports them when
//! an OTLP endpoint is configured.

use std::time::Duration;

use http::{Method, StatusCode};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};

use crate::db;

/// Bucket boundaries, in seconds, that the semantic conventions advise for HTTP durations.
pub const DURATION_BUCKETS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

/// The database pool, as its metrics name it.
const POOL_NAME: &str = "portal";

/// The instruments the portal records with. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Metrics {
    meter: Meter,
    request_duration: Histogram<f64>,
    active_requests: UpDownCounter<i64>,
    sign_ins: Counter<u64>,
    registrations: Counter<u64>,
    sandbox_refusals: Counter<u64>,
}

/// How a sign-in with GitHub ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInOutcome {
    Succeeded,
    /// The developer declined, the sign-in expired or started elsewhere, or the account has
    /// no verified primary email.
    Refused,
    /// GitHub or the database could not be reached, or GitHub refused the OAuth app.
    Failed,
}

/// What became of a developer's registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationOutcome {
    /// The issuer was created at once.
    Created,
    /// The registration waits for an operator's approval.
    Requested,
}

/// A limit that refused an active issuer: see `config::Sandbox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxLimit {
    RegistrationsPerAccount,
    IssuersPerAccount,
    ActiveIssuers,
}

impl Metrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            meter: meter.clone(),
            request_duration: meter
                .f64_histogram("http.server.request.duration")
                .with_description("How long the server took to answer requests.")
                .with_unit("s")
                .with_boundaries(DURATION_BUCKETS.to_vec())
                .build(),
            active_requests: meter
                .i64_up_down_counter("http.server.active_requests")
                .with_description("Requests the server is answering.")
                .with_unit("{request}")
                .build(),
            sign_ins: meter
                .u64_counter("portal.sign_ins")
                .with_description("Sign-ins with GitHub, by outcome.")
                .with_unit("{sign_in}")
                .build(),
            registrations: meter
                .u64_counter("portal.registrations")
                .with_description("Sandbox issuers registered, by outcome.")
                .with_unit("{registration}")
                .build(),
            sandbox_refusals: meter
                .u64_counter("portal.sandbox.refusals")
                .with_description("Issuers that a sandbox limit refused, by limit.")
                .with_unit("{refusal}")
                .build(),
        }
    }

    /// Reports the connections of `pool`, idle and in use, and the requests that wait for
    /// one, each time metrics are collected.
    pub fn track_db_pool(&self, pool: &db::Pool) {
        let counted = pool.clone();
        self.meter
            .i64_observable_up_down_counter("db.client.connection.count")
            .with_description("Connections in the database pool, by state.")
            .with_unit("{connection}")
            .with_callback(move |observer| {
                let status = counted.status();
                let used = status.size.saturating_sub(status.available);
                for (state, count) in [("idle", status.available), ("used", used)] {
                    let attributes = [
                        KeyValue::new("db.client.connection.pool.name", POOL_NAME),
                        KeyValue::new("db.client.connection.state", state),
                    ];
                    observer.observe(as_i64(count), &attributes);
                }
            })
            .build();

        let waiting = pool.clone();
        self.meter
            .i64_observable_up_down_counter("db.client.connection.pending_requests")
            .with_description("Requests waiting for a database connection.")
            .with_unit("{request}")
            .with_callback(move |observer| {
                let attributes = [KeyValue::new("db.client.connection.pool.name", POOL_NAME)];
                observer.observe(as_i64(waiting.status().waiting), &attributes);
            })
            .build();
    }

    /// Counts a request as active until the returned guard is dropped.
    #[must_use]
    pub(crate) fn active_request(&self, method: &Method) -> ActiveRequest<'_> {
        let attributes = [KeyValue::new("http.request.method", method_name(method))];
        self.active_requests.add(1, &attributes);
        ActiveRequest {
            counter: &self.active_requests,
            attributes,
        }
    }

    /// Records how long a request took. `route` is the matched route, never the path, so
    /// the number of series stays bounded. A server error also names its status as the
    /// error's type, as the semantic conventions advise.
    pub(crate) fn record_request(
        &self,
        method: &Method,
        route: Option<&str>,
        status: StatusCode,
        duration: Duration,
    ) {
        let attributes: Vec<_> = [
            KeyValue::new("http.request.method", method_name(method)),
            KeyValue::new("http.response.status_code", i64::from(status.as_u16())),
        ]
        .into_iter()
        .chain(route.map(|route| KeyValue::new("http.route", route.to_owned())))
        .chain(
            status
                .is_server_error()
                .then(|| KeyValue::new("error.type", status.as_str().to_owned())),
        )
        .collect();
        self.request_duration
            .record(duration.as_secs_f64(), &attributes);
    }

    pub fn sign_in(&self, outcome: SignInOutcome) {
        let outcome = match outcome {
            SignInOutcome::Succeeded => "succeeded",
            SignInOutcome::Refused => "refused",
            SignInOutcome::Failed => "failed",
        };
        self.sign_ins.add(1, &[KeyValue::new("outcome", outcome)]);
    }

    pub fn registration(&self, outcome: RegistrationOutcome) {
        let outcome = match outcome {
            RegistrationOutcome::Created => "created",
            RegistrationOutcome::Requested => "requested",
        };
        self.registrations
            .add(1, &[KeyValue::new("outcome", outcome)]);
    }

    pub fn sandbox_refusal(&self, limit: SandboxLimit) {
        let limit = match limit {
            SandboxLimit::RegistrationsPerAccount => "registrations_per_account",
            SandboxLimit::IssuersPerAccount => "issuers_per_account",
            SandboxLimit::ActiveIssuers => "active_issuers",
        };
        self.sandbox_refusals
            .add(1, &[KeyValue::new("limit", limit)]);
    }
}

/// Keeps a request counted as active while it lives.
pub(crate) struct ActiveRequest<'a> {
    counter: &'a UpDownCounter<i64>,
    attributes: [KeyValue; 1],
}

impl Drop for ActiveRequest<'_> {
    fn drop(&mut self) {
        self.counter.add(-1, &self.attributes);
    }
}

/// `method`, or `_OTHER` for a method HTTP does not define, as the semantic conventions
/// ask: clients cannot add series by inventing methods.
pub(crate) fn method_name(method: &Method) -> &'static str {
    match method.as_str() {
        "CONNECT" => "CONNECT",
        "DELETE" => "DELETE",
        "GET" => "GET",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        "PATCH" => "PATCH",
        "POST" => "POST",
        "PUT" => "PUT",
        "TRACE" => "TRACE",
        _ => "_OTHER",
    }
}

fn as_i64(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invented_methods_share_one_name() {
        assert_eq!(method_name(&Method::PATCH), "PATCH");
        let invented = Method::from_bytes(b"BREW").unwrap();
        assert_eq!(method_name(&invented), "_OTHER");
    }
}
