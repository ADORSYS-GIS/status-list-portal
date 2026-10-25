# Telemetry

The portal always writes log lines: the server to stdout, operator commands to stderr. When an OTLP endpoint is configured, it also sends traces, metrics and logs there, over OTLP/HTTP with protobuf. Any backend that accepts OTLP works, directly or through an [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/).

## Configuration

The portal reads the standard [OpenTelemetry variables](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/). The ones that matter most:

| Variable | Default | Meaning |
| --- | --- | --- |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | none | Base URL for every signal, such as `http://otel-collector:4318`. The portal adds `/v1/traces`, `/v1/metrics` and `/v1/logs`. Without it, and without the variables below, nothing is exported. |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, `_METRICS_ENDPOINT`, `_LOGS_ENDPOINT` | none | Full URL for one signal. Set alone, it exports that signal only. |
| `OTEL_TRACES_EXPORTER`, `OTEL_METRICS_EXPORTER`, `OTEL_LOGS_EXPORTER` | none | `none` stops a signal. `otlp` exports it even without an endpoint, to `http://localhost:4318`. Unset, a signal is exported when an endpoint is set for it. |
| `OTEL_EXPORTER_OTLP_HEADERS` | none | Headers sent with every export, as `key=value` pairs separated by commas, such as an API key for a hosted backend. Keep it as secret as the portal's other secrets. |
| `OTEL_EXPORTER_OTLP_COMPRESSION` | none | `gzip` compresses exports. |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | `10000` | Milliseconds an export may take. |
| `OTEL_SERVICE_NAME` | `status-list-portal` | The service's name in the backend. |
| `OTEL_RESOURCE_ATTRIBUTES` | none | More attributes for everything the portal sends, such as `k8s.namespace.name=portal`. |
| `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG` | keep every trace | Which traces to keep, such as `traceidratio` with `0.1` for one in ten. See [Traces](#traces) for the exceptions. |
| `OTEL_METRIC_EXPORT_INTERVAL` | `60000` | Milliseconds between metric exports. |
| `OTEL_SDK_DISABLED` | `false` | `true` exports nothing. |

The portal speaks `http/protobuf` only. It refuses to start when `OTEL_EXPORTER_OTLP_PROTOCOL`, or a signal's own protocol variable, names another protocol for an exported signal, and when an `OTEL_*_EXPORTER` variable is neither `otlp` nor `none`. As with the portal's own settings, an empty variable counts as unset.

Use `https` for an endpoint outside the portal's private network: exports carry request paths, user agents and the portal's user ids. On shutdown, the portal waits for its last exports, up to `OTEL_EXPORTER_OTLP_TIMEOUT`, so give it at least that long to stop.

Everything the portal sends names the service with `service.name`, `service.version` and a `service.instance.id` that is new for each process, and the environment with `deployment.environment.name` (from `APP_ENV`).

## Traces

Each request is a trace of its own, named after its method and route, such as `POST /api/v1/issuers`. A trace context that a request carries is ignored, since anyone can send one. Within a request:

- Each call to the Status List Server's admin API is a span named after the call's route, such as `POST /admin/v1/issuers`. The call carries a `traceparent` header, so when the Status List Server exports traces too, its spans join the portal's trace.
- Each call to GitHub during a sign-in is a span of its own. No trace context goes to GitHub.
- Log lines written during the request are events of its span.

Each operator command, such as `portal approve-registration`, is a trace of its own, whose root span is `operator.command`.

Some traces do not depend on `OTEL_TRACES_SAMPLER`. Sign-ins (`GET /api/v1/session/github/callback`), registrations (`POST /api/v1/issuers`) and operator commands are always kept: they are rare, and the first thing to look at when a developer reports a problem. Health checks (`/health/*`) are never kept: a probe sends one every few seconds.

Anyone can call the sign-in callback, so a flood of such requests is kept in full, whatever the sampler says. The exporter's queue is bounded: when it is full, spans are dropped rather than slow the portal down.

## Metrics

| Metric | Type | Attributes | What it answers |
| --- | --- | --- | --- |
| `http.server.request.duration` (s) | histogram | `http.request.method`, `http.route`, `http.response.status_code`, and `error.type` for a server error | How many requests each route answers, how many fail, and how fast. |
| `http.server.active_requests` | up-down counter | `http.request.method` | How many requests are in progress. |
| `http.client.request.duration` (s) | histogram | `http.request.method`, `url.template`, `server.address`, and `http.response.status_code` or, without a response, `error.type` | How the admin API answers the portal, and how long it takes until its response headers arrive. |
| `db.client.connection.count` | up-down counter | `db.client.connection.pool.name`, `db.client.connection.state` (`idle`, `used`) | How full the database pool is. |
| `db.client.connection.pending_requests` | up-down counter | `db.client.connection.pool.name` | How many requests wait for a database connection: above zero, the pool is too small. |
| `portal.sign_ins` | counter | `outcome`: `succeeded`, `refused`, `failed` | Whether developers can sign in. `refused` means the developer declined, the sign-in expired, or the account has no verified primary email; `failed` means GitHub or the database could not be reached, or GitHub refused the OAuth app. |
| `portal.registrations` | counter | `outcome`: `created`, `requested` | How many sandbox issuers developers register, at once or for approval. |
| `portal.sandbox.refusals` | counter | `limit`: `registrations_per_account`, `issuers_per_account`, `active_issuers` | Which limit turns developers away. `active_issuers` means the sandbox is full. |

The names follow the [semantic conventions](https://opentelemetry.io/docs/specs/semconv/) where one applies. Prometheus's default translation writes them with underscores and a unit or `_total` suffix, such as `http_server_request_duration_seconds` and `portal_sign_ins_total`. A request that matches no route has no `http.route`, and a method HTTP does not define is recorded as `_OTHER`, so no client can add series by making up paths or methods.

Operator commands export no metrics: a process that lasts a second would only add series that end at once.

## Logs

Log lines go to stdout, or stderr for operator commands, as `APP_LOG_FORMAT` says, `text` or `json`, and `RUST_LOG` filters them. A JSON line written during a request carries the request's trace and span ids under `openTelemetry`, so a log collector can link it to the trace.

When logs are exported, the lines `RUST_LOG` lets through go to the OTLP endpoint too, with their trace and span ids. The exporters' own HTTP clients and the OpenTelemetry SDK are left out, so exports do not report on themselves.

## Privacy

Telemetry never carries query strings, which hold the code and state of a GitHub sign-in, cookies, tokens, email addresses, GitHub logins or IP addresses. Log lines name accounts by the portal's internal user id. Spans carry the request path, which can hold an issuer's id, and the client's user agent, each cut at 256 bytes. Error messages are logged as they are, without the URLs of the admin API's requests, whose queries name an issuer's owner.

## Trying it locally

```sh
docker compose --profile observability up -d
```

This starts [`grafana/otel-lgtm`](https://github.com/grafana/docker-otel-lgtm), Grafana with a collector, Tempo, Loki and Prometheus behind it. Set `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318` in `.env`, run the portal, and open Grafana at <http://localhost:3000>. Set `GRAFANA_PORT` to use another port. Lower `OTEL_METRIC_EXPORT_INTERVAL`, to `5000` for example, to see metrics sooner.

## Next to the Status List Server

The Status List Server's Helm chart runs an OpenTelemetry Collector that takes OTLP/HTTP on port 4318. Point the portal at it, such as `OTEL_EXPORTER_OTLP_ENDPOINT=http://<release>-opentelemetry-collector.<namespace>.svc.cluster.local:4318`. The chart's collector sends traces to Tempo and logs to Loki, but OTLP metrics only to its `debug` exporter: add a metrics exporter to its pipeline, such as `otlphttp` to Mimir or `prometheusremotewrite`, to keep the portal's metrics.
