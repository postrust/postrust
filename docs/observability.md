# Observability

Postrust can export traces and metrics with OpenTelemetry, over OTLP, and
serve its metrics as Prometheus text at `GET /metrics` on the admin port.

All of this is in the `otel` cargo feature. The published images
(`postrust/postrust` and its `-alpine` variant) are built with it, and it is
inactive until configured: with nothing set, the server does exactly what it
does without the feature. A binary you build yourself needs
`--features otel` (`--features postrust-lambda/otel` for the Lambda adapter).

The log, the request span and its fields, request ids and `traceparent`
handling do not need the feature; they are described under
[Tracing and logging](./deployment.md#tracing-and-logging).

## Turning it on

| What | Switched on by |
|------|----------------|
| Traces, over OTLP | `OTEL_EXPORTER_OTLP_ENDPOINT`, or `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` |
| Metrics, over OTLP | `OTEL_EXPORTER_OTLP_ENDPOINT`, or `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` |
| Metrics, at `/metrics` | `PGRST_ADMIN_SERVER_PORT` |

```bash
# A collector beside the server, on the OTLP/HTTP port.
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318
OTEL_SERVICE_NAME=postrust-orders     # default: postrust
PGRST_ADMIN_SERVER_PORT=3001          # and /metrics is on it
```

OTLP is sent over **HTTP with protobuf** only, to the collector's HTTP port
(4318 by convention). gRPC is not built in, so the gRPC stack is not in the
binary; a `OTEL_EXPORTER_OTLP_PROTOCOL` other than `http/protobuf` is
reported at start-up and otherwise ignored. With `OTEL_EXPORTER_OTLP_ENDPOINT`
the path for each signal is appended (`/v1/traces`, `/v1/metrics`); a
per-signal endpoint is used as given.

### OpenTelemetry variables

The standard variables are read by the OpenTelemetry SDK, as any other
service's would be:

| Variable | Effect |
|----------|--------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Base URL of the collector; turns on both signals |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | One signal, at this exact URL |
| `OTEL_EXPORTER_OTLP_HEADERS`, `..._TRACES_HEADERS`, `..._METRICS_HEADERS` | `key=value,key=value`, sent with every export -- an API key for a hosted backend |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | Milliseconds an export may take. Default 10000 |
| `OTEL_EXPORTER_OTLP_COMPRESSION` | `gzip` to compress exports. Unset, they are not compressed |
| `OTEL_SERVICE_NAME` | `service.name`. Default `postrust` |
| `OTEL_RESOURCE_ATTRIBUTES` | More resource attributes: `deployment.environment=prod,service.namespace=shop` |
| `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG` | Default `parentbased_always_on`: everything, unless the caller's `traceparent` says it is not sampling. `parentbased_traceidratio` with `0.1` keeps one trace in ten |
| `OTEL_BSP_SCHEDULE_DELAY`, `OTEL_BSP_MAX_QUEUE_SIZE`, `OTEL_BSP_MAX_EXPORT_BATCH_SIZE` | The span batcher. Defaults 5000 ms, 2048, 512 |
| `OTEL_METRIC_EXPORT_INTERVAL` | Milliseconds between metric exports. Default 60000 |
| `OTEL_TRACES_EXPORTER=none`, `OTEL_METRICS_EXPORTER=none` | Turn one signal off while the endpoint stays set |
| `OTEL_SDK_DISABLED=true` | Turn both OTLP exporters off. `/metrics` stays, since it is not an exporter but an endpoint the admin port was opened for |

`service.version` is set to the server's version.

## Traces

Each request is one trace, or one part of the caller's: a W3C `traceparent`
puts the request's span beneath the caller's, and `tracestate` is carried
with it. The exported span's trace id is the `trace_id` every log line for
the request carries and the `request.trace_id` the database sees, whether it
came from the caller or was made here -- so a slow statement in the
PostgreSQL log, a log line and a trace all name the same trace.

```
GET /api/{*path}              server span, `otel.name` = "{method} {route}"
├── auth                      role, jwt.cache = hit | miss | bypassed
├── plan
├── db.query                  db.system, db.operation.name, db.collection.name
│   ├── db.count              Prefer: count -- db.operation.name = count | explain
│   └── db.embed              an embed fetched by its own query (see below)
├── render
└── db.commit                 a COMMIT sent on its own
```

| Span | Attributes |
|------|------------|
| request | `http.request.method`, `url.path`, `http.route` (unset when nothing matched), `http.response.status_code`, `request_id`; status `ERROR` on a 5xx |
| `auth` | `role`; `jwt.cache` where the token cache is on: `hit`, `miss`, or `bypassed` when there was no token to look up |
| `db.query` | `db.system=postgresql`, `db.operation.name` (`select`, `insert`, `update`, `delete`, `call`), `db.collection.name` (the table or function) |
| `db.count` | `db.operation.name`: `count` for `count=exact`, `explain` for `count=planned`; `count=estimated` may run both |
| `db.embed` | `db.collection.name` (the embedded table), `postrust.embed.relation` (as the request named it) |
| `db.commit` | `db.operation.name=COMMIT` |
| `graphql` | `graphql.operation.name` |

`db.query.text` -- the statement, on `db.query`, `db.count` and `db.embed` --
is recorded only with `PGRST_OTEL_DB_STATEMENT=true`. The statements are
parameterized, so request values are not in them, but table, column and
function names are.

Where the time goes:

- An embed is usually part of the main statement and so of `db.query`.
  `db.embed` is the query an embed takes when it is fetched separately.
- A read whose transaction can end with its query commits in the same round
  trip, and has no `db.commit`. A read with a count commits on its own after
  the count; a write commits last, after `render`, so a response the client is
  refused can still roll it back.
- The `BEGIN`, the role switch and the request settings go out with the
  first statement and are part of `db.query`.

GraphQL operations have a `graphql` span beneath the request span; the
resolvers' queries are not broken out yet. (async-graphql's own
OpenTelemetry extension is built against an older `opentelemetry` than the
server uses, so it is not used.)

Only warnings and errors from Postrust itself are attached to spans as
events. The debug lines carry SQL text, and so does sqlx's slow-statement
warning, so neither is exported. The log is unaffected by any of this: the
exporter has a filter of its own, and the debug-level spans are exported
while the log stays at `PGRST_LOG_LEVEL`.

## Metrics

| Metric (OpenTelemetry) | Prometheus | Type | Attributes |
|------------------------|------------|------|------------|
| `http.server.request.duration` | `http_server_request_duration_seconds` | histogram, s | `http.request.method`, `http.route`, `http.response.status_code` |
| `http.server.active_requests` | `http_server_active_requests` | up-down counter | `http.request.method` |
| `db.client.connection.count` | `db_client_connection_count` | up-down counter | `db.client.connection.state` (`idle`, `used`), `db.client.connection.pool.name` |
| `db.client.connection.max` | `db_client_connection_max` | up-down counter | `db.client.connection.pool.name` |
| `db.client.connection.wait_time` | `db_client_connection_wait_time_seconds` | histogram, s | `db.client.connection.pool.name` |
| `postrust.jwt.cache.lookups` | `postrust_jwt_cache_lookups_total` | counter | `outcome` (`hit`, `miss`) |
| `postrust.jwt.jwks.refreshes` | `postrust_jwt_jwks_refreshes_total` | counter | `outcome` (`success`, `failure`) |
| `postrust.jwt.jwks.key_set.age` | `postrust_jwt_jwks_key_set_age_seconds` | gauge, s | |
| `postrust.schema_cache.reloads` | `postrust_schema_cache_reloads_total` | counter | `outcome` (`success`, `failure`) |
| `postrust.graphql.subscriptions.active` | `postrust_graphql_subscriptions_active` | up-down counter | |

Notes:

- `http.route` is the route, not the path -- `/api/{*path}` for every REST
  resource -- so the series stay few. A request that matched no route has no
  `http.route`; a method outside the standard nine is `_OTHER`.
- `db.client.connection.wait_time` is measured around beginning a REST
  request's transaction: the wait for a pooled connection, and `BEGIN`.
  Rising waits mean the pool (`PGRST_DB_POOL`) is too small for the load, or
  connections are held too long.
- The JWT cache counts lookups from REST and GraphQL alike. A request with no
  token is not a lookup.
- A schema reload is one requested with `NOTIFY pgrst, 'reload schema'`
  (`PGRST_DB_CHANNEL_ENABLED`).
- The JWKS metrics are there only with `PGRST_JWT_JWKS_URL`. A successful
  fetch is one that brought a set, changed or not. The key set's age is how
  long ago the set in use was last confirmed by a fetch, and is absent until
  one has been: alert on it growing well past `PGRST_JWT_JWKS_REFRESH`, or on
  failures rising while successes do not.
- The pool, cache, JWKS and subscription figures are read when metrics are
  collected, not recorded as they change.
- Prometheus also gets `target_info`, carrying the resource attributes.

### Prometheus

`/metrics` is served on the admin port only, never on the API's: what the
pool and the token cache are doing is the operator's business. It needs no
OTLP endpoint.

```bash
PGRST_ADMIN_SERVER_PORT=3001
curl -s localhost:3001/metrics | grep http_server_request_duration_seconds_count
```

```yaml
# prometheus.yml
scrape_configs:
  - job_name: postrust
    metrics_path: /metrics
    static_configs:
      - targets: ["postrust:3001"]
```

On Kubernetes, scrape the admin port's container port. The admin port also
serves `/live` and `/ready`, so it is the one to point probes at.

## AWS Lambda

Build the adapter with `--features postrust-lambda/otel`, add the [AWS
Distro for OpenTelemetry](https://aws-otel.github.io/docs/getting-started/lambda)
collector layer, and point the exporter at it:

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
OTEL_SERVICE_NAME=postrust-api
```

Lambda freezes the sandbox as soon as an invocation's response is returned,
and the exporters work on timers of their own, so whatever an invocation
recorded is flushed to the collector before its response goes back. That
adds the round trip to the collector -- in the same sandbox, typically a
millisecond or two -- to every invocation. Metrics go over OTLP only; a
function has no admin port.

A Lambda binary of your own gets the same from `postrust_lambda::run`; one
that serves the router itself wraps it in `postrust_lambda::FlushLayer`.

Cloudflare Workers are not covered: the OpenTelemetry SDK needs a tokio
runtime. A Worker's spans reach its own log only.

## What it costs

With the feature built in and nothing configured, the request path checks a
few flags that were set once at start-up and does nothing else: no exporter,
no extra layer in the log's subscriber, no clock reads beyond those it
already took. The token cache counts its hits and misses in every build, with
a relaxed atomic increment beside the lock each lookup already takes.

With an exporter, each request builds an OpenTelemetry span for itself and
each stage, and the batcher sends them in the background; metrics add a
histogram record and two counter updates per request. Measure it on your own
hardware with your own load: `docs/benchmarking.md` has the method. Sampling
(`OTEL_TRACES_SAMPLER=parentbased_traceidratio`) reduces the cost of traces
roughly in proportion; metrics are not sampled.

The process does not yet shut down gracefully on `SIGTERM`, so spans still in
the batcher when it is killed are lost (at most `OTEL_BSP_SCHEDULE_DELAY`'s
worth).

## Using it from your own binary

A binary that embeds the server (see [Custom Routes](./custom-routes.md))
builds its own subscriber. `postrust_server::otel::layer()` is the exporting
layer, filtered as the server's is, or `None` when no endpoint is set;
`postrust_server::otel::init_metrics(prometheus)` starts metrics, and
`custom::admin_router()` serves `/metrics`. Spans of your own routes, at
debug or above, are exported with the rest.
