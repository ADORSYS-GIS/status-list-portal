//! Which signals the standard `OTEL_*` variables send to an OTLP endpoint.

use std::time::Duration;

use anyhow::bail;
use opentelemetry_otlp::{
    OTEL_EXPORTER_OTLP_ENDPOINT, OTEL_EXPORTER_OTLP_LOGS_ENDPOINT,
    OTEL_EXPORTER_OTLP_LOGS_PROTOCOL, OTEL_EXPORTER_OTLP_LOGS_TIMEOUT,
    OTEL_EXPORTER_OTLP_METRICS_ENDPOINT, OTEL_EXPORTER_OTLP_METRICS_PROTOCOL,
    OTEL_EXPORTER_OTLP_METRICS_TIMEOUT, OTEL_EXPORTER_OTLP_PROTOCOL,
    OTEL_EXPORTER_OTLP_PROTOCOL_HTTP_PROTOBUF, OTEL_EXPORTER_OTLP_TIMEOUT,
    OTEL_EXPORTER_OTLP_TIMEOUT_DEFAULT, OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
    OTEL_EXPORTER_OTLP_TRACES_PROTOCOL, OTEL_EXPORTER_OTLP_TRACES_TIMEOUT,
};

/// The variables that configure one signal.
struct Signal {
    /// `otlp` or `none`.
    exporter: &'static str,
    endpoint: &'static str,
    protocol: &'static str,
    timeout: &'static str,
}

const TRACES: Signal = Signal {
    exporter: "OTEL_TRACES_EXPORTER",
    endpoint: OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
    protocol: OTEL_EXPORTER_OTLP_TRACES_PROTOCOL,
    timeout: OTEL_EXPORTER_OTLP_TRACES_TIMEOUT,
};

const METRICS: Signal = Signal {
    exporter: "OTEL_METRICS_EXPORTER",
    endpoint: OTEL_EXPORTER_OTLP_METRICS_ENDPOINT,
    protocol: OTEL_EXPORTER_OTLP_METRICS_PROTOCOL,
    timeout: OTEL_EXPORTER_OTLP_METRICS_TIMEOUT,
};

const LOGS: Signal = Signal {
    exporter: "OTEL_LOGS_EXPORTER",
    endpoint: OTEL_EXPORTER_OTLP_LOGS_ENDPOINT,
    protocol: OTEL_EXPORTER_OTLP_LOGS_PROTOCOL,
    timeout: OTEL_EXPORTER_OTLP_LOGS_TIMEOUT,
};

/// The signals to export. A signal is exported when an endpoint is set for it, or for all
/// signals, unless its `OTEL_*_EXPORTER` is `none`; `otlp` exports it to the SDK's default
/// endpoint even without one. `OTEL_SDK_DISABLED=true` exports nothing.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Signals {
    pub(super) traces: bool,
    pub(super) metrics: bool,
    pub(super) logs: bool,
}

impl Signals {
    /// Reads the variables through `var`. An empty variable counts as unset, as it does for
    /// the portal's own settings.
    pub(super) fn from_lookup(var: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let var = |name: &str| var(name).filter(|value| !value.is_empty());
        if var("OTEL_SDK_DISABLED").is_some_and(|value| value.eq_ignore_ascii_case("true")) {
            return Ok(Self::default());
        }

        let enabled = |signal: &Signal| -> anyhow::Result<bool> {
            let exported = match var(signal.exporter).as_deref() {
                None => {
                    var(OTEL_EXPORTER_OTLP_ENDPOINT).is_some() || var(signal.endpoint).is_some()
                }
                Some("otlp") => true,
                Some("none") => false,
                Some(other) => bail!(
                    "{} is {other:?}; the portal supports `otlp` and `none`",
                    signal.exporter
                ),
            };
            if exported {
                let protocol = [signal.protocol, OTEL_EXPORTER_OTLP_PROTOCOL]
                    .into_iter()
                    .find_map(|name| var(name).map(|value| (name, value)));
                if let Some((name, value)) = protocol
                    && value != OTEL_EXPORTER_OTLP_PROTOCOL_HTTP_PROTOBUF
                {
                    bail!(
                        "{name} is {value:?}; the portal exports over \
                         {OTEL_EXPORTER_OTLP_PROTOCOL_HTTP_PROTOBUF} only"
                    );
                }
            }
            Ok(exported)
        };

        Ok(Self {
            traces: enabled(&TRACES)?,
            metrics: enabled(&METRICS)?,
            logs: enabled(&LOGS)?,
        })
    }

    /// How long the last exports may take when the exporters shut down: the longest export
    /// timeout of the exported signals, read through `var`.
    pub(super) fn shutdown_timeout(&self, var: impl Fn(&str) -> Option<String>) -> Duration {
        [
            (self.traces, &TRACES),
            (self.metrics, &METRICS),
            (self.logs, &LOGS),
        ]
        .into_iter()
        .filter(|(exported, _)| *exported)
        .map(|(_, signal)| export_timeout(signal, &var))
        .max()
        .unwrap_or(OTEL_EXPORTER_OTLP_TIMEOUT_DEFAULT)
    }
}

/// How long one of `signal`'s exports may take, as the OTLP exporters read it: its
/// `OTEL_EXPORTER_OTLP_*_TIMEOUT`, else `OTEL_EXPORTER_OTLP_TIMEOUT`, in milliseconds, else
/// 10 seconds. A value that is not a number is skipped, as the exporters skip it.
fn export_timeout(signal: &Signal, var: impl Fn(&str) -> Option<String>) -> Duration {
    [signal.timeout, OTEL_EXPORTER_OTLP_TIMEOUT]
        .into_iter()
        .find_map(|name| var(name)?.parse().ok())
        .map_or(OTEL_EXPORTER_OTLP_TIMEOUT_DEFAULT, Duration::from_millis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn signals(vars: &[(&str, &str)]) -> anyhow::Result<Signals> {
        Signals::from_lookup(lookup(vars))
    }

    fn lookup<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'a {
        let vars: HashMap<_, _> = vars.iter().copied().collect();
        move |name| vars.get(name).map(|value| value.to_string())
    }

    fn shutdown_timeout(vars: &[(&str, &str)]) -> Duration {
        signals(vars).unwrap().shutdown_timeout(lookup(vars))
    }

    const ALL: Signals = Signals {
        traces: true,
        metrics: true,
        logs: true,
    };

    #[test]
    fn nothing_is_exported_without_an_endpoint() {
        assert_eq!(signals(&[]).unwrap(), Signals::default());
        assert_eq!(
            signals(&[(OTEL_EXPORTER_OTLP_ENDPOINT, "")]).unwrap(),
            Signals::default()
        );
    }

    #[test]
    fn a_shared_endpoint_exports_every_signal() {
        let vars = [(OTEL_EXPORTER_OTLP_ENDPOINT, "http://collector:4318")];
        assert_eq!(signals(&vars).unwrap(), ALL);
    }

    #[test]
    fn a_signal_endpoint_exports_that_signal_only() {
        let vars = [(
            OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
            "http://tempo:4318/v1/traces",
        )];
        let expected = Signals {
            traces: true,
            ..Signals::default()
        };
        assert_eq!(signals(&vars).unwrap(), expected);
    }

    #[test]
    fn the_exporter_variables_turn_signals_on_and_off() {
        let vars = [
            (OTEL_EXPORTER_OTLP_ENDPOINT, "http://collector:4318"),
            ("OTEL_METRICS_EXPORTER", "none"),
        ];
        let expected = Signals {
            metrics: false,
            ..ALL
        };
        assert_eq!(signals(&vars).unwrap(), expected);

        let expected = Signals {
            logs: true,
            ..Signals::default()
        };
        assert_eq!(
            signals(&[("OTEL_LOGS_EXPORTER", "otlp")]).unwrap(),
            expected
        );
    }

    #[test]
    fn the_sdk_can_be_disabled() {
        let vars = [
            (OTEL_EXPORTER_OTLP_ENDPOINT, "http://collector:4318"),
            ("OTEL_SDK_DISABLED", "TRUE"),
        ];
        assert_eq!(signals(&vars).unwrap(), Signals::default());
    }

    #[test]
    fn unsupported_exporters_are_refused() {
        let error = signals(&[("OTEL_TRACES_EXPORTER", "console")]).unwrap_err();
        assert_eq!(
            error.to_string(),
            r#"OTEL_TRACES_EXPORTER is "console"; the portal supports `otlp` and `none`"#
        );
    }

    #[test]
    fn only_http_protobuf_is_spoken() {
        let endpoint = (OTEL_EXPORTER_OTLP_ENDPOINT, "http://collector:4318");
        let error = signals(&[endpoint, (OTEL_EXPORTER_OTLP_PROTOCOL, "grpc")]).unwrap_err();
        assert_eq!(
            error.to_string(),
            r#"OTEL_EXPORTER_OTLP_PROTOCOL is "grpc"; the portal exports over http/protobuf only"#
        );

        let vars = [
            endpoint,
            (OTEL_EXPORTER_OTLP_PROTOCOL, "grpc"),
            (OTEL_EXPORTER_OTLP_TRACES_PROTOCOL, "http/protobuf"),
            (OTEL_EXPORTER_OTLP_METRICS_PROTOCOL, "http/protobuf"),
            (OTEL_EXPORTER_OTLP_LOGS_PROTOCOL, "http/protobuf"),
        ];
        assert_eq!(signals(&vars).unwrap(), ALL);

        // A signal that is not exported may name any protocol.
        let vars = [(OTEL_EXPORTER_OTLP_PROTOCOL, "grpc")];
        assert_eq!(signals(&vars).unwrap(), Signals::default());
    }

    #[test]
    fn the_shutdown_waits_as_long_as_the_slowest_export() {
        let endpoint = (OTEL_EXPORTER_OTLP_ENDPOINT, "http://collector:4318");
        assert_eq!(shutdown_timeout(&[endpoint]), Duration::from_secs(10));

        let vars = [
            endpoint,
            (OTEL_EXPORTER_OTLP_TIMEOUT, "2000"),
            (OTEL_EXPORTER_OTLP_LOGS_TIMEOUT, "7000"),
        ];
        assert_eq!(shutdown_timeout(&vars), Duration::from_secs(7));
    }

    #[test]
    fn only_exported_signals_set_the_shutdown_timeout() {
        let vars = [
            (OTEL_EXPORTER_OTLP_ENDPOINT, "http://collector:4318"),
            (OTEL_EXPORTER_OTLP_TIMEOUT, "2000"),
            (OTEL_EXPORTER_OTLP_METRICS_TIMEOUT, "30000"),
            ("OTEL_METRICS_EXPORTER", "none"),
        ];
        assert_eq!(shutdown_timeout(&vars), Duration::from_secs(2));
    }

    #[test]
    fn timeouts_that_are_not_numbers_are_skipped() {
        let vars = [
            (
                OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
                "http://tempo:4318/v1/traces",
            ),
            (OTEL_EXPORTER_OTLP_TRACES_TIMEOUT, "soon"),
            (OTEL_EXPORTER_OTLP_TIMEOUT, "3000"),
        ];
        assert_eq!(shutdown_timeout(&vars), Duration::from_secs(3));
    }
}
