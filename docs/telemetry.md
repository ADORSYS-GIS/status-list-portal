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
| `OTEL_EXPORTER_OTLP_TIMEOUT` | `10000` | Milliseconds an export may take. `OTEL_EXPORTER_OTLP_TRACES_TIMEOUT`, `_METRICS_TIMEOUT` and `_LOGS_TIMEOUT` set it for one signal. |
| `OTEL_SERVICE_NAME` | `status-list-portal` | The service's name in the backend. |
| `OTEL_RESOURCE_ATTRIBUTES` | none | More attributes for everything the portal sends, such as `k8s.namespace.name=portal`. |
| `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG` | keep every trace | Which traces to keep, such as `traceidratio` with `0.1` for one in ten. See [Traces](#traces) for the exceptions. |
| `OTEL_METRIC_EXPORT_INTERVAL` | `60000` | Milliseconds between metric exports. |
| `OTEL_SDK_DISABLED` | `false` | `true` exports nothing. |

The portal speaks `http/protobuf` only. It refuses to start when `OTEL_EXPORTER_OTLP_PROTOCOL`, or a signal's own protocol variable, names another protocol for an exported signal, and when an `OTEL_*_EXPORTER` variable is neither `otlp` nor `none`. As with the portal's own settings, an empty variable counts as unset.

Use `https` for an endpoint outside the portal's private network: exports carry request paths, user agents and the portal's user ids. On shutdown, the portal waits for its last traces and logs up to the longest export timeout, and for its last metrics up to 5 seconds, a limit of the OpenTelemetry SDK. Give it at least that long to stop.

Everything the portal sends names the service with `service.name`, `service.version` and a `service.instance.id` that is new for each process, and the environment with `deployment.environment.name` (from `APP_ENV`).

## Traces

Each request is a trace of its own, named after its method and route, such as `POST /api/v1/issuers`. A trace context that a request carries is ignored, since anyone can send one. Within a request:

- Each call to the Status List Server's admin API is a span named after the call's route, such as `POST /admin/v1/issuers`. The call carries a `traceparent` header, so when the Status List Server exports traces too, its spans join the portal's trace.
- Each call to GitHub during a sign-in is a span of its own. No trace context goes to GitHub.
- Log lines written during the request are events of its span.

Each operator command, such as `portal approve-registration`, is a trace of its own, whose root span is `operator.command`.

Some traces do not depend on `OTEL_TRACES_SAMPLER`. Operator commands are always kept, and sign-ins (`GET /api/v1/session/github/callback`) and registrations (`POST /api/v1/issuers`) up to 10 a second each: they are rare, and the first thing to look at when a developer reports a problem. Anyone can send both, a registration without a session included, so beyond that limit `OTEL_TRACES_SAMPLER` decides: with ratio sampling, a flood of them cannot crowd out other traces. Health checks (`/health/*`) are never traced: a probe sends one every few seconds, so their spans are `DEBUG`, below what is exported.

The exporter's queue is bounded: when it is full, spans are dropped rather than slow the portal down.

## Metrics

| Metric | Type | Attributes | What it answers |
| --- | --- | --- | --- |
| `http.server.request.duration` (s) | histogram | `http.request.method`, `http.route`, `http.response.status_code`, `url.scheme`, and `error.type` for a server error | How many requests each route answers, how many fail, and how fast. |
| `http.server.active_requests` | up-down counter | `http.request.method`, `url.scheme` | How many requests are in progress. |
| `http.client.request.duration` (s) | histogram | `http.request.method`, `url.template`, `server.address`, `server.port`, `http.response.status_code` when the admin API answered, and `error.type` for a server error or no answer | How the admin API answers the portal, and how long it takes until its response headers arrive. |
| `db.client.connection.count` | up-down counter | `db.client.connection.pool.name`, `db.client.connection.state` (`idle`, `used`) | How full the database pool is. |
| `db.client.connection.pending_requests` | up-down counter | `db.client.connection.pool.name` | How many requests wait for a database connection: above zero, the pool is too small. |
| `portal.sign_ins` | counter | `portal.sign_in.outcome`: `succeeded`, `denied`, `expired`, `no_verified_email`, `failed` | Whether developers can sign in. `denied` means the developer did not approve the portal on GitHub; `expired`, that the sign-in expired, was already used or did not start in this browser, which also counts anyone calling the callback; `no_verified_email`, that the GitHub account has no verified primary email; `failed`, that GitHub or the database could not be reached, or GitHub refused the OAuth app. |
| `portal.registrations` | counter | `portal.registration.outcome`: `created`, `requested` | How many sandbox issuers developers register, at once or for approval. |
| `portal.sandbox.refusals` | counter | `portal.sandbox.limit`: `registrations_per_account`, `issuers_per_account`, `active_issuers` | Which limit turns developers away. `active_issuers` means the sandbox is full. A renewal counts too when it is refused for lack of room: an expired issuer that comes back is active again. |

The names follow the [semantic conventions](https://opentelemetry.io/docs/specs/semconv/) where one applies. Prometheus's default translation writes them with underscores and a unit or `_total` suffix, such as `http_server_request_duration_seconds` and `portal_sign_ins_total`. `url.scheme` is the scheme of `APP_SERVER__ORIGIN`, which browsers use, even when a proxy forwards requests over plain HTTP. A request that matches no route has no `http.route`, and a method HTTP does not define is recorded as `_OTHER`, so no client can add series by making up paths or methods.

Operator commands export no metrics: a process that lasts a second would only add series that end at once.

## Logs

Log lines go to stdout, or stderr for operator commands, as `APP_LOG_FORMAT` says, `text` or `json`, and `RUST_LOG` filters them. A JSON line holds the event's fields at the top level, and the fields of the spans it was written in under `spans`: when two spans share a field, such as a request and its call to the admin API, the innermost one's value is kept. A line written during a request carries the request's trace and span ids under `openTelemetry`, so a log collector can link it to the trace. Health checks log their response at `DEBUG`, a failing one too, so the default `RUST_LOG=info` leaves them out.

When logs are exported, the lines `RUST_LOG` lets through go to the OTLP endpoint too, with their trace and span ids. Libraries' lines go there only from `INFO` up, whatever `RUST_LOG` says, the request logs (`tower_http`) aside: below `INFO`, some log the values they handle, such as a database query's parameters, which can hold a developer's login and email address. The exporters' own HTTP clients and the OpenTelemetry SDK are left out, so exports do not report on themselves.

## Privacy

Telemetry never carries query strings, which hold the code and state of a GitHub sign-in, cookies, tokens, email addresses, GitHub logins or IP addresses. Log lines name accounts by the portal's internal user id. Spans carry the request path, which can hold an issuer's id, and the client's user agent, each cut at 256 bytes. Error messages are logged as they are, without the URLs of the admin API's requests, whose queries name an issuer's owner.

## Trying it locally

```sh
docker compose --profile observability up -d
```

This starts [`grafana/otel-lgtm`](https://github.com/grafana/docker-otel-lgtm), Grafana with a collector, Tempo, Loki and Prometheus behind it. Set `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318` in `.env`, run the portal, and open Grafana at <http://localhost:3000>. Set `GRAFANA_PORT` to use another port. Lower `OTEL_METRIC_EXPORT_INTERVAL`, to `5000` for example, to see metrics sooner.
