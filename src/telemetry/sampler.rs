//! Which traces are kept.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use opentelemetry::trace::{Link, SpanKind, TraceId};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::trace::{Sampler, SamplingResult, ShouldSample};

/// The name of an operator command's root span.
pub const OPERATOR_COMMAND_SPAN: &str = "operator.command";

/// Sign-ins and registrations, whose traces are kept whatever the baseline says. They are
/// rare, and the first thing to look at when a developer reports a problem.
const SIGN_IN: (&str, &str) = ("GET", "/api/v1/session/github/callback");
const REGISTRATION: (&str, &str) = ("POST", "/api/v1/issuers");

/// How many sign-in traces, and how many registration traces, are kept each second
/// whatever the baseline says. Anyone can send both, a registration without a session
/// included, since the sampler decides before the request is authenticated. A flood would
/// otherwise be kept in full and crowd out the other traces; beyond this, the baseline
/// decides.
const KEPT_PER_SECOND: u32 = 10;

/// Keeps every trace of an operator command, and of a sign-in or a registration up to
/// [`KEPT_PER_SECOND`], and leaves other traces to `baseline`. A span with a parent follows
/// its parent, so a kept trace keeps all its spans. Health checks never get here: their
/// spans are `DEBUG`, below what is exported.
pub fn sampler(baseline: Box<dyn ShouldSample>) -> Sampler {
    Sampler::ParentBased(Box::new(PortalSampler::new(baseline)))
}

#[derive(Debug, Clone)]
struct PortalSampler {
    baseline: Box<dyn ShouldSample>,
    /// The limits are shared by the sampler's clones, which the SDK may make.
    sign_ins: Arc<RateLimit>,
    registrations: Arc<RateLimit>,
}

impl PortalSampler {
    fn new(baseline: Box<dyn ShouldSample>) -> Self {
        Self {
            baseline,
            sign_ins: Arc::new(RateLimit::per_second(KEPT_PER_SECOND)),
            registrations: Arc::new(RateLimit::per_second(KEPT_PER_SECOND)),
        }
    }
}

impl ShouldSample for PortalSampler {
    fn should_sample(
        &self,
        parent: Option<&Context>,
        trace_id: TraceId,
        name: &str,
        kind: &SpanKind,
        attributes: &[KeyValue],
        links: &[Link],
    ) -> SamplingResult {
        let attribute = |key: &str| {
            attributes
                .iter()
                .find(|attribute| attribute.key.as_str() == key)
                .map(|attribute| attribute.value.as_str())
        };
        let method = attribute("http.request.method");
        let route = attribute("http.route");

        let limit = match method.as_deref().zip(route.as_deref()) {
            Some(SIGN_IN) => Some(&self.sign_ins),
            Some(REGISTRATION) => Some(&self.registrations),
            _ => None,
        };
        let always = name == OPERATOR_COMMAND_SPAN
            || limit.is_some_and(|limit| limit.try_take(Instant::now()));
        let sampler: &dyn ShouldSample = if always {
            &Sampler::AlwaysOn
        } else {
            self.baseline.as_ref()
        };
        sampler.should_sample(parent, trace_id, name, kind, attributes, links)
    }
}

/// Lets through up to a number of takes in each second, counted from the second's first take.
#[derive(Debug)]
struct RateLimit {
    per_second: u32,
    window: Mutex<Window>,
}

#[derive(Debug, Default)]
struct Window {
    /// When the current second began, if anything was taken yet.
    start: Option<Instant>,
    taken: u32,
}

impl RateLimit {
    fn per_second(per_second: u32) -> Self {
        Self {
            per_second,
            window: Mutex::default(),
        }
    }

    /// Whether a take at `now` is within the limit, counting it if it is.
    fn try_take(&self, now: Instant) -> bool {
        // Two plain numbers, consistent even if a holder panicked.
        let mut window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        let second_is_over = window
            .start
            .is_none_or(|start| now.saturating_duration_since(start) >= Duration::from_secs(1));
        if second_is_over {
            *window = Window {
                start: Some(now),
                taken: 0,
            };
        }
        if window.taken >= self.per_second {
            return false;
        }
        window.taken += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_sdk::trace::SamplingDecision;

    /// What the sampler decides with a baseline that drops every trace.
    fn decision(name: &str, attributes: &[KeyValue]) -> SamplingDecision {
        decision_with(Sampler::AlwaysOff, name, attributes)
    }

    fn decision_with(baseline: Sampler, name: &str, attributes: &[KeyValue]) -> SamplingDecision {
        decide(&PortalSampler::new(Box::new(baseline)), name, attributes)
    }

    fn decide(sampler: &PortalSampler, name: &str, attributes: &[KeyValue]) -> SamplingDecision {
        let result = sampler.should_sample(
            None,
            TraceId::from(1),
            name,
            &SpanKind::Server,
            attributes,
            &[],
        );
        result.decision
    }

    fn request(method: &'static str, route: &'static str) -> [KeyValue; 2] {
        [
            KeyValue::new("http.request.method", method),
            KeyValue::new("http.route", route),
        ]
    }

    #[test]
    fn operator_commands_are_always_kept() {
        assert_eq!(
            decision(OPERATOR_COMMAND_SPAN, &[]),
            SamplingDecision::RecordAndSample
        );
    }

    #[test]
    fn sign_ins_and_registrations_are_always_kept() {
        let sign_in = request("GET", "/api/v1/session/github/callback");
        assert_eq!(
            decision("request", &sign_in),
            SamplingDecision::RecordAndSample
        );
        let registration = request("POST", "/api/v1/issuers");
        assert_eq!(
            decision("request", &registration),
            SamplingDecision::RecordAndSample
        );
    }

    #[test]
    fn sign_ins_and_registrations_beyond_their_limits_follow_the_baseline() {
        let sampler = PortalSampler::new(Box::new(Sampler::AlwaysOff));
        let sign_in = request("GET", "/api/v1/session/github/callback");
        let registration = request("POST", "/api/v1/issuers");
        // Each has a limit of its own: a flood of sign-ins leaves registrations kept.
        for request in [sign_in, registration] {
            for _ in 0..KEPT_PER_SECOND {
                assert_eq!(
                    decide(&sampler, "request", &request),
                    SamplingDecision::RecordAndSample
                );
            }
            assert_eq!(
                decide(&sampler, "request", &request),
                SamplingDecision::Drop
            );
        }
    }

    #[test]
    fn the_rate_limit_starts_over_each_second() {
        let limit = RateLimit::per_second(2);
        let start = Instant::now();
        let later = |millis| start + Duration::from_millis(millis);

        assert!(limit.try_take(start));
        assert!(limit.try_take(later(500)));
        assert!(!limit.try_take(later(999)));
        assert!(limit.try_take(later(1000)));
        assert!(limit.try_take(later(1200)));
        assert!(!limit.try_take(later(1999)));
    }

    #[test]
    fn other_requests_follow_the_baseline() {
        assert_eq!(
            decision("request", &request("GET", "/api/v1/issuers")),
            SamplingDecision::Drop
        );
        assert_eq!(decision("request", &[]), SamplingDecision::Drop);
    }
}
