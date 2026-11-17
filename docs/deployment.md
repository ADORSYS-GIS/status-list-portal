# Deployment

## Backtraces

Set `RUST_BACKTRACE=1` to include stack traces in panic logs. Release builds keep
function names, but strip debug information, so file names and line numbers may
be unavailable. Rust's default panic hook writes the trace to stderr as plain text;
it is separate from JSON logs written to stdout when `APP_LOG_FORMAT=json`.

## Kubernetes probes

Set `APP_SERVER__BIND_ADDRESS=0.0.0.0:8080` in the container. The HTTP listener
starts after database migrations finish. Give startup enough time for the longest
migration and for waiting on another replica's migration lock. For example, these
container probes allow up to five minutes of startup before liveness takes effect:

```yaml
startupProbe:
  httpGet:
    path: /health/live
    port: 8080
  periodSeconds: 5
  timeoutSeconds: 3
  failureThreshold: 60
livenessProbe:
  httpGet:
    path: /health/live
    port: 8080
  periodSeconds: 10
  timeoutSeconds: 3
readinessProbe:
  httpGet:
    path: /health/ready
    port: 8080
  periodSeconds: 5
  timeoutSeconds: 3
```

Readiness begins as unavailable. A separate one-connection pool checks PostgreSQL every
five seconds, with a two-second deadline. One failed check is tolerated; two consecutive
failures make the instance unready. Results older than 15 seconds are unready. Size the
database connection limit for one extra connection per portal replica.

Restrict access to the container port to the ingress controller or reverse proxy,
for example with a Kubernetes NetworkPolicy. Otherwise, other in-cluster clients
can connect directly and bypass the proxy's timeouts.

## Reverse proxy

Put a reverse proxy in front of the portal and configure a header read timeout there.
`axum::serve` does not impose one, so a client can otherwise hold a connection open while
sending request headers slowly. For example, set `client_header_timeout 10s;` in an NGINX
server block, or use your proxy's equivalent header read timeout. Configure an idle
connection timeout too, such as NGINX's `keepalive_timeout`. The portal's 30-second
request timeout starts once the request headers have arrived and also covers reading
the body. It does not replace the proxy's header read timeout.
