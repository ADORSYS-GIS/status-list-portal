//! Which traces are kept.

use opentelemetry::trace::{Link, SpanKind, TraceId};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::trace::{Sampler, SamplingResult, ShouldSample};

/// The name of an operator command's root span.
pub const OPERATOR_COMMAND_SPAN: &str = "operator.command";

/// Requests whose traces are always kept: sign-ins and registrations. They are rare, and the
/// first thing to look at when a developer reports a problem.
const ALWAYS_SAMPLED_ROUTES: [(&str, &str); 2] = [
    ("GET", "/api/v1/session/github/callback"),
    ("POST", "/api/v1/issuers"),
];

/// Keeps every trace of an operator command, a sign-in or a registration, and leaves other
/// traces to `baseline`. A span with a parent follows its parent, so a kept trace keeps all
/// its spans. Health checks never get here: their spans are `DEBUG`, below what is exported.
pub fn sampler(baseline: Box<dyn ShouldSample>) -> Sampler {
    Sampler::ParentBased(Box::new(PortalSampler { baseline }))
}

#[derive(Debug, Clone)]
struct PortalSampler {
    baseline: Box<dyn ShouldSample>,
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

        let sampler: &dyn ShouldSample = match (method.as_deref(), route.as_deref()) {
            (Some(method), Some(route)) if ALWAYS_SAMPLED_ROUTES.contains(&(method, route)) => {
                &Sampler::AlwaysOn
            }
            _ if name == OPERATOR_COMMAND_SPAN => &Sampler::AlwaysOn,
            _ => self.baseline.as_ref(),
        };
        sampler.should_sample(parent, trace_id, name, kind, attributes, links)
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
        let sampler = PortalSampler {
            baseline: Box::new(baseline),
        };
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
    fn other_requests_follow_the_baseline() {
        assert_eq!(
            decision("request", &request("GET", "/api/v1/issuers")),
            SamplingDecision::Drop
        );
        assert_eq!(decision("request", &[]), SamplingDecision::Drop);
    }
}
