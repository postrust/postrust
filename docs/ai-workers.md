# AI Workers

`postrust-ai` serves AI models on demand. It starts a model server when
something asks for the model, stops it when it has been idle, and runs queued
jobs against it by priority. A node is one process (`postrust-ai`) with a
PostgreSQL database; several nodes can share the database and see each other's
jobs, sessions and loaded models.

> **On its own `0.x` line, with no stability promise.** A minor release may
> break it. See [Stability and Versioning](./stability.md).
>
> **What has been run, and what has not.** The queue, scheduler, executor,
> workflows, sessions, gateway, caches, proxy registration and the process
> runtime are tested against a real PostgreSQL and a stub model server
> (`cargo test -p postrust-ai -- --include-ignored`). The parts that talk to
> real software have **not** been run against it: real vLLM, Ollama and
> whisper.cpp servers, the Docker/Podman container runtime, downloads from
> Hugging Face over the network, and forwarding between nodes on separate
> machines. Each is marked *(unverified)* below. Treat the example
> configuration as a starting point to check against the server versions you
> install.

## Running it

```bash
postrust-ai path/to/postrust-ai.toml
# or
POSTRUST_AI_CONFIG=path/to/postrust-ai.toml postrust-ai
```

With no file it runs from defaults and the environment. A complete annotated
configuration is
[`crates/postrust-ai/examples/postrust-ai.toml`](../crates/postrust-ai/examples/postrust-ai.toml);
a test parses it, so the keys in it exist.

The migration (schema `ai`) is applied at startup unless
`database.migrate = false`. Logging follows `RUST_LOG` and defaults to `info`.

## Concepts

### Models and servers

A model is declared in the config with an `id`, a `kind` (`text`, `audio`,
`video`, `image`, `embedding`), a `flavor` and a `runtime`. The model server is
always a **separate server**, never loaded into the node's own process.

| Runtime | What the node does |
| --- | --- |
| `process` | Starts the command, hands it a port, waits for the health check, stops it with SIGTERM to its whole process group, and SIGKILL if it has not exited after 30 seconds. |
| `container` | The same, through the `docker` or `podman` CLI *(unverified: no container runtime has been run)*. |
| `attached` | Starts nothing. The server is someone else's; for Ollama the node asks it to load and unload the model. One instance only. |

| Flavor | Health check | Default inference path | Can be parked |
| --- | --- | --- | --- |
| `vllm` | `/health` | `/v1/chat/completions` (`/v1/embeddings` for `kind = "embedding"`) | yes, unless attached |
| `whisper_cpp` | `/` | `/inference` | no |
| `ollama` | `/api/version` | `/v1/chat/completions` (`/v1/embeddings` for embeddings) | no |
| `generic` | `/health` | `/` | no |

`args` and `env` values in a `process` runtime may use `{port}`, `{gpu}`,
`{file:N}` (path of the Nth declared file), `{cache_dir}`, `{hf_home}`,
`{model}` (the model's `served_name`, else its `id`) and `{instance}`. The node
sets `CUDA_VISIBLE_DEVICES` to the GPU it reserved, or to nothing for a
CPU-only model, and `HF_HOME` under the cache directory. For vLLM it sets
`VLLM_SERVER_DEV_MODE=1`, which exposes the `/sleep` and `/wake_up` endpoints
parking needs; bind the server to `127.0.0.1`.

How each flavor is driven is written from the servers' documentation and has
not been run against the real servers *(unverified)*: vLLM's `/sleep?level=1`
and `/wake_up`, Ollama's `/api/show`, `/api/pull` and `keep_alive`, and
whisper.cpp's multipart `/inference`.

### Loading, parking and unloading

A model loads the first time a request or job needs it: files are fetched into
the cache, the server is started, and the instance is ready once its health
check passes (`load_timeout_secs`, default 300). It is stopped after
`idle_timeout_secs` without a request (default 600; 0 means never). A vLLM
instance started with `--enable-sleep-mode` can instead be **parked**: its
weights move to host memory and the GPU memory is released, so waking it takes
seconds rather than a reload. A parked instance is stopped after a further
`park_timeout_secs` (default 1800).

`pinned = true` means never unload. `min_warm = N` keeps N instances loaded and
loads them at startup (it cannot exceed `max_replicas`). `max_replicas` is how
many instances one node may run, and `max_concurrency` how many requests one
instance serves at once.

### Resources and making room

Accounting is by declaration. Each model states `memory_mb` (host memory) and
`gpu_mb` (VRAM on one GPU; 0 is CPU only), and the node keeps the sum of
what is loaded under the budget in `[resources]`: detected memory minus
`memory_reserve_mb`, and each GPU's VRAM minus `gpu_reserve_mb`. GPUs are
detected with `nvidia-smi` if it is installed, or listed in `resources.gpus`.
With `respect_probe = true` the probed free memory must also cover a load, so
memory taken by processes the node does not manage is noticed.

When a model does not fit, the node makes room in this order, and waits if
that is not enough:

1. **Idle instances are parked or stopped.**
2. **Superlow jobs holding instances are asked to pause**, which makes their
   instances idle, and step 1 applies on the next pass.

The rules for step 1:

- An instance is **parked** when only VRAM is short, the server can park, and
  host memory can take the weights. Otherwise it is stopped.
- The victim is chosen by lowest *value*, not by plain least-recently-used.
  Value rises with queued demand for the model (and the priority of that
  demand), the time the model took to load, and held session locks, and falls
  with idle time.
- A victim must hold some of the resource that is short. An instance that
  would free nothing the request lacks is left alone.
- A model is never taken below `min_warm` ready instances. A parked instance
  does not count as warm and may be stopped.
- An idle instance whose model has jobs waiting at a **higher** priority than
  the requester is protected.
- `pinned` models, instances in use, and instances holding session locks are
  not candidates; a `critical` requester may take the ones holding session
  locks.
- Superlow work only ever uses what is already free: it never unloads
  anything.

These rules are covered by tests in the crate's manager module.

### Probe and measured usage

Each node reads the machine every `scheduler.maintenance_secs` (default 5): host memory, GPUs
(`nvidia-smi`), the load average (`/proc/loadavg`) and the CPU count. **The
load average and CPU count are information only** and are not used in
scheduling or eviction. They appear in `/metrics` (`postrust_ai_cpu_load` with
a `window` label, and `postrust_ai_cpus`).

For instances the node started as processes, it also measures what the server
actually holds: host memory (resident set size, summed over the server's
process group) and VRAM. These are the `measured_mem_mb` and `measured_gpu_mb`
fields of each instance in `GET /v1/models`, and the
`postrust_ai_instance_measured_mem_mb` and
`postrust_ai_instance_measured_gpu_mb` gauges.

When an instance's usage exceeds its declared amount by both more than 10% and
at least 256 MiB, the node logs one warning and writes one `over_budget` row to
`ai.model_events` (once per instance). **Measurements never change a
reservation.** Raise `memory_mb` or `gpu_mb` yourself if the warning is right.

Caveats. RSS is summed over the process group and counts shared pages once per
process, so a multi-process server such as vLLM can report more than it really
holds, and `over_budget` can fire for that reason. Containers and attached
servers are not measured, and the fields are absent for them.

### Priorities

Every job has a class and an offset: `superlow < low < normal < high <
critical`, with bases 0, 100, 200, 300 and 400 and an offset of -49 to 49 inside
the band. Workers claim the highest first. A waiting job gains one priority
point per `scheduler.aging_secs` (default 60), up to `aging_cap` (default 99),
so low work is not starved. **Superlow never ages.** Synchronous gateway
requests count as `high` unless the `x-priority` request header says otherwise.

### Superlow

Superlow jobs run only on spare capacity: a free slot on an instance that is
ready now, or room to load one without unloading anything. They never cause an
unload. When a queue's slots are full and regular work is waiting, a running
superlow job is paused to free a slot, and when resources are needed the
manager pauses superlow jobs holding instances.

### Pausing

How a pause treats a running job is a **per-queue** setting, `pause_mode`:

| `pause_mode` | What happens |
| --- | --- |
| `checkpoint` (default) | The job is told and gets `pause_grace_secs` (default 30) to stop at a checkpoint. If it has not stopped by then it is cancelled and resumes from the last checkpoint it saved. |
| `requeue` | The job is cancelled at once and starts over when it next runs. |

A paused job always has `status = paused`. Its `pause_reason` is `preempted`
(made way for other work; it resumes by itself) or `manual` (paused through the
API; it waits for `POST /v1/jobs/{id}/resume`).

The built-in handler checkpoints a job whose input is a list of requests
(`{"requests": [...]}`) after each one, so pausing costs at most one request.
Split long work (a video's segments, an audio file's chunks, a document's
pages) into a list. A job with a single request has nothing to checkpoint, so a
paused one starts over. The built-in handler calls the model server over HTTP;
library users can register their own handler per queue.

### Jobs

`POST /v1/jobs` takes `queue`, `model_id`, `input` and optionally `priority`,
`priority_offset`, `params`, `idempotency_key`, `run_at` and `max_attempts`.
The built-in handler reads `input` as one request or a list:

```json
{ "request":  { "path": "/v1/chat/completions", "body": { "messages": [] } } }
{ "requests": [ { "body": {} }, { "body": {} } ] }
{ "request":  { "multipart": { "file_url": "https://example.com/a.wav",
                               "fields": { "response_format": "json" } } } }
```

`path` defaults to the flavor's inference path and `method` to `POST`. For
vLLM and Ollama, `body.model` is filled in with the model's served name if
absent. A 4xx answer from the model server fails the job permanently (except
408 and 429); anything else is retried up to the queue's `max_attempts` with a
backoff that starts at `retry_backoff_secs` and doubles, capped at an hour.
Submitting the same `idempotency_key` twice returns the first job. Writes from
a worker are conditional on it still holding the job, so a node that lost its
lease cannot overwrite the node that took over.

### Sessions

A **session lock** binds a `(session id, model)` pair to one instance on one
node, so a conversation keeps reaching the server whose prompt and KV cache is
warm. Send the session id in the `x-session-id` header (`sessions.header`).

- A request on a node that does not own the lock is forwarded to the node that
  does, once.
- While a lock lives its instance is not unloaded, except for `critical` work.
- A lock expires `sessions.ttl_secs` after its last request (default 600), or
  is released with `DELETE /v1/sessions/{model}/{session}`.
- One instance takes at most `sessions.max_per_instance` sessions (default 64)
  before new ones go to another replica.
- If the instance is gone (crash, critical eviction) the next request binds the
  session anew and the response carries `x-session-rebound: true`: the client's
  cache is cold.

The counts are per node; the table in the database is what nodes agree on.

### Caching

| Layer | What | How it is controlled |
| --- | --- | --- |
| 1 | Model files on local disk | `[cache]`: `dir`, `max_disk_gb`, `shared`. Files are stored by sha256 (if the model gives one) or by URL hash, least recently used dropped first, never one a loaded model uses. `shared = true` takes a lock file for downloads on an NFS or EFS directory. |
| 2 | Loaded models kept warm | `idle_timeout_secs`, `pinned`, `min_warm`, queue-driven preload |
| 3 | Parked in host memory | vLLM sleep mode, `park_timeout_secs` |
| 4 | Server-side prompt/KV caches | Made useful by session locks |
| 5 | Whole job results | `result_cache = true` on a model or a queue; entries last `cache.result_ttl_secs` (default 7 days, 0 for no expiry) |

Model file URLs may be `https://`, `http://`, `hf://org/repo[@revision]/path`
or `file:///path` (used in place, never copied or evicted). Give each file a
`sha256`: without it a changed upstream file is never noticed. Downloads from
the network have not been run against Hugging Face *(unverified)*; the
download, checksum, shared-lock and eviction paths are tested against a local
server.

The result cache is **off by default**, because it is wrong for sampled text
generation. Keys hash the model id, its `version`, the input and `params`. Bump
`version` to invalidate a model's entries, or call
`DELETE /v1/models/{id}/results`. A `file_url` in the input is part of the key
as text, so **changed content behind an unchanged URL still hits**: put a
content hash or version in the URL or `params`.

### Workflows

A workflow is a set of steps, each of which becomes a job (or one job per
element). Define one with `PUT /v1/workflows/{name}` and start a run with
`POST /v1/workflows/{name}/runs`. A complete example is
[`crates/postrust-ai/examples/video-summary.workflow.json`](../crates/postrust-ai/examples/video-summary.workflow.json),
and a test checks it against the engine's validation.

```json
{
  "steps": [
    { "id": "transcribe", "model": "whisper-large-v3-turbo",
      "input": { "request": { "multipart": { "file_url": "${input.audio_url}" } } } },
    { "id": "summarise", "model": "llama3-8b", "depends_on": ["transcribe"],
      "input": { "request": { "body": { "messages": [
        { "role": "user", "content": "Summarise: ${steps.transcribe.output.text}" } ] } } } }
  ],
  "output": { "summary": "${steps.summarise.output}" }
}
```

- `depends_on` lists the steps that must finish first; steps with no
  dependency between them run in parallel. Ids are non-empty and contain no `.`
  or `:`. A definition with duplicate ids, an unknown dependency, a cycle or
  an unknown model is rejected (400) when it is stored.
- `${path}` as a whole string is replaced by the value at `path`, keeping its
  JSON type; inside a longer string it is inserted as text. Paths start at
  `input`, `steps.<id>.output`, `item` or `index`, separated by dots, with array
  positions as numbers (`steps.a.output.results.0.text`). A path that resolves
  to nothing becomes `null`. A step's `input` must render to a JSON object.
- `for_each` is a template that resolves to an array; the step becomes one job
  per element, with `item` and `index` available in the templates. Its output
  is the array of the jobs' outputs, in order, and steps that depend on it wait
  for all of them (fan-in). An empty array completes the step with `[]` and no
  jobs. A `for_each` that is not an array fails the run.
- `when` skips a step unless a condition holds: `{"exists": "path"}` (not
  null), `{"path": "...", "equals": value}` or `{"path": "...", "not_equals":
  value}`. Steps that depend on a skipped step still run and see its output as
  `null`.
- A step can set its own `queue`, `priority` and `params`; otherwise it uses
  the run's queue and priority. `output` is a template for the run's output;
  without it the output is an object of the outputs of the steps nothing
  depends on.
- A step that fails permanently fails the run and cancels its outstanding jobs.

Run state is rebuilt from the run's jobs every time it advances, under a row
lock, and step jobs have idempotency keys `wf:{run}:{step}:{index}`, so
advancing twice (after a crash, or from two nodes) cannot enqueue a step twice.
A periodic reconcile pass covers a missed advance.

## The API

Everything except the three probes needs authentication when
`auth.jwt_secret` is set (see below). **Admin** marks endpoints limited to
`auth.admin_roles`.

| Method | Path | |
| --- | --- | --- |
| `POST` | `/v1/jobs` | Submit a job. 201 with the job. 404 for an unknown model or queue. |
| `GET` | `/v1/jobs` | List. Query: `queue`, `status`, `model_id`, `workflow_run_id`, `limit` (default 50, at most 500). |
| `GET` | `/v1/jobs/{id}` | One job. |
| `POST` | `/v1/jobs/{id}/pause` | Ask a running job to pause (`manual`). |
| `POST` | `/v1/jobs/{id}/resume` | Resume a paused job. |
| `POST` | `/v1/jobs/{id}/cancel` | Cancel. |
| `GET` | `/v1/queues` | Per queue: paused, waiting by class, running, paused jobs, dead. |
| `POST` | `/v1/queues/{name}/pause`, `/resume` | Pause or resume claiming on a queue. **Admin.** |
| `GET` | `/v1/models` | This node's model specs and their instances (state, measured usage). |
| `POST` | `/v1/models/{id}/load`, `/unload` | Load in the background (may unload idle models for it), or unload. **Admin.** |
| `DELETE` | `/v1/models/{id}/results` | Drop a model's cached results. **Admin.** |
| `ANY` | `/v1/infer/{model}/{*path}` | Synchronous inference: forwarded to `{instance}/{path}`, streaming both ways. |
| `GET` | `/v1/sessions` | This node's session locks. |
| `DELETE` | `/v1/sessions/{model}/{session}` | Release a session lock. |
| `PUT`, `GET` | `/v1/workflows/{name}` | Define (returns the version) or read. |
| `POST` | `/v1/workflows/{name}/runs` | Start a run. Body: `input`, optional `queue` (default: the first configured) and `priority`. |
| `GET` | `/v1/workflow-runs/{id}` | Run status, per-step state, output. |
| `POST` | `/v1/workflow-runs/{id}/cancel` | Cancel a run. 409 if it already finished. |
| `GET` | `/v1/nodes` | Nodes that have heartbeated recently, with their ready models. |
| `GET` | `/v1/resources` | This node's budget, reservations and running jobs. |
| `GET` | `/v1/cache/files` | Cached model files. |
| `GET` | `/healthz`, `/readyz`, `/metrics` | No authentication. `/readyz` checks the database. |

Errors are `{"error": "..."}`: 400 for a bad request or configuration, 404 for
an unknown job, model, queue or workflow, 409 for a conflict, 502 when the
model server failed to load or answered badly, 500 otherwise.

### Authentication

With no `auth.jwt_secret` the API is **open**, and every caller is an admin:
put it behind something that authenticates. With one, requests need an
`Authorization: Bearer` JWT signed with that HS256 secret (and with `aud` equal
to `auth.audience`, if set), validated by `postrust-auth` as described in
[Authentication](./authentication.md); `POSTRUST_AI_JWT_SECRET` overrides the
file. A missing or invalid token gets 401 with `WWW-Authenticate: Bearer`. A
valid token whose role is not in `auth.admin_roles` gets 403 on the admin
endpoints (an empty `admin_roles` means any authenticated caller is an admin).
Only the endpoints marked admin check the role: job control, session release
and workflow definition are open to any authenticated caller. There are no
per-queue permissions yet. The gateway passes request headers on to the model
server except `Authorization` and `Cookie`, which are meant for the node; a
request forwarded to the node holding a session keeps them, since that node
checks them again.

### Synchronous inference

`/v1/infer/{model}/{path}` is for requests a person is waiting on. It picks an
instance (a session's locked one, else a ready one here, else a peer that has
the model ready, else it loads one), then proxies the request there, streaming
the body in both directions.

If a model takes longer than `gateway.load_wait_secs` (default 30) to load, or
the resources cannot be found, the answer is **503 with `Retry-After: 10`**.
The load carries on, so a retry finds the model ready. The 503 also covers a
request cancelled while waiting.

Headers:

| Header | Direction | Meaning |
| --- | --- | --- |
| `x-session-id` | request | Session id (name set by `sessions.header`), at most 256 characters. |
| `x-priority` | request | Priority class for this request. Default `high`. It decides what the request may unload to make room. |
| `x-session-rebound: true` | response | The session's instance was gone and it was bound to a new one. |
| `x-postrust-ai-node` | response | The node that served the request. |
| `x-postrust-ai-instance` | response | The instance holding the session lock (only on session requests). |
| `x-postrust-ai-forwarded` | request | Set by a node forwarding to another. A request carrying it is served where it lands and never forwarded again. Clients do not set it. |

### Metrics

`GET /metrics` is Prometheus text. Counters: `postrust_ai_result_cache_hits_total`,
`..._misses_total`, `postrust_ai_preemptions_total`,
`postrust_ai_gateway_requests_total`, `..._load_waits_total`,
`..._unavailable_total`, `..._forwarded_total`,
`postrust_ai_session_rebinds_total`, and per queue
`postrust_ai_jobs_finished_total` and `postrust_ai_job_seconds_total`. Gauges:
`postrust_ai_queue_waiting{queue,class}`, `postrust_ai_queue_running`,
`postrust_ai_queue_paused_jobs`, `postrust_ai_instances{model,state}`,
`postrust_ai_instance_measured_mem_mb`, `postrust_ai_instance_measured_gpu_mb`,
`postrust_ai_memory_mb{kind}`, `postrust_ai_gpu_mb{gpu,kind}`,
`postrust_ai_cpu_load{window}` and `postrust_ai_cpus`. Model lifecycle events
(`load_started`, `ready`, `load_failed`, `parked`, `woken`, `stopped`,
`crashed`, `evicted`, `preempted_jobs`, `over_budget`) are rows in
`ai.model_events`.

## Running with postrust-proxy

Add a `[proxy]` section and each node registers itself through the proxy's
admin API:

```toml
[node]
listen = "0.0.0.0:7700"
advertise_url = "http://gpu-1.internal:7700"

[proxy]
admin_url = "http://proxy:9000/admin/api"
upstream_name = "postrust-ai"
route_prefix = "/ai/"
# admin_token = "..."      # sent as a Bearer token if the admin API needs one
```

On startup the node creates the upstream (least-connections, because inference
requests vary widely in length) if it is missing, adds itself as a backend, and
creates a route for `route_prefix` that strips the prefix, if missing. A
request to `https://proxy/ai/v1/infer/qwen2-7b/v1/chat/completions` reaches a
node as `/v1/infer/qwen2-7b/v1/chat/completions`. On shutdown the node removes
only its own backend. If registration fails the node logs the error and serves
without the proxy. The proxy does not know about models or sessions: the nodes
forward session-locked requests to the owning node, and other requests to a
node with the model ready.

Registration goes through the admin API, not the proxy's tables, because the
proxy reads those only at startup. Set `advertise_url` whenever `listen` is not
an address the proxy can reach.

**The route gets the proxy's default 30-second timeout.** The admin API cannot
set a route's timeout, so a synchronous request that takes longer (a long
generation, a large transcription) is answered with a 504 by the proxy while
the node carries on. Either submit long work as a job, or raise the route's
timeout in the proxy's own configuration.

## Configuration reference

All sections are optional, and an unknown key is an error. The example file
lists every key with its default.

| Section | Keys |
| --- | --- |
| `[node]` | `id` (default: hostname), `listen` (`127.0.0.1:7700`), `advertise_url` |
| `[database]` | `url`, `max_connections` (10), `migrate` (true) |
| `[resources]` | `memory_mb`, `memory_reserve_mb` (2048), `gpus` (`[{index, vram_mb}]`), `gpu_reserve_mb` (512), `respect_probe` (true), `max_instances` (16), `port_range` (`[18000, 18999]`) |
| `[cache]` | `dir` (`/var/cache/postrust-ai`), `max_disk_gb` (100), `shared` (false), `result_ttl_secs` (604800) |
| `[sessions]` | `header` (`x-session-id`), `ttl_secs` (600), `max_per_instance` (64) |
| `[gateway]` | `load_wait_secs` (30), `request_timeout_secs` (600), `forward_to_ready_nodes` (true) |
| `[scheduler]` | `lease_secs` (60), `heartbeat_secs` (15, at most half the lease), `poll_secs` (5), `aging_secs` (60), `aging_cap` (99), `archive_after_secs` (86400), `maintenance_secs` (5) |
| `[proxy]` | `admin_url`, `upstream_name` (`postrust-ai`), `route_prefix` (`/ai/`), `admin_token` |
| `[auth]` | `jwt_secret`, `audience`, `admin_roles` |
| `[[queue]]` | `name`, `concurrency` (4), `pause_mode` (`checkpoint`), `pause_grace_secs` (30), `max_attempts` (3), `retry_backoff_secs` (10), `result_cache` (false) |
| `[[model]]` | `id`, `kind`, `flavor`, `runtime`, `version` (`"1"`), `served_name`, `files`, `memory_mb`, `gpu_mb`, `health_path`, `load_timeout_secs` (300), `idle_timeout_secs` (600), `park_timeout_secs` (1800), `pinned`, `min_warm`, `max_replicas` (1), `max_concurrency` (1), `result_cache` |

A `default` queue is added if none is declared. Environment variables override
the file: `DATABASE_URL`, `POSTRUST_AI_NODE_ID`, `POSTRUST_AI_LISTEN`,
`POSTRUST_AI_ADVERTISE_URL`, `POSTRUST_AI_JWT_SECRET`, `POSTRUST_AI_CACHE_DIR`.

The example workflow uses two models the example config does not declare: a
generic HTTP service that extracts audio from a video (its API is yours to
define), and an embedding model. For example:

```toml
[[model]]
id = "audio-extractor"
kind = "video"
flavor = "generic"
[model.runtime]
type = "attached"
base_url = "http://127.0.0.1:9100"

[[model]]
id = "embedder"
kind = "embedding"
flavor = "ollama"
served_name = "nomic-embed-text"
[model.runtime]
type = "attached"
base_url = "http://127.0.0.1:11434"
```

Two models on one attached Ollama server are two instances that happen to
share a server; each is accounted for separately.

## Limitations

- **Proxy routes time out after 30 seconds** (above). Long synchronous
  inference should be a job.
- **Every node must carry the whole model registry.** `POST /v1/jobs` rejects
  models the node it lands on does not declare, and a node claims only jobs for
  models it declares. Use the same `[[model]]` list everywhere; resources
  decide what each node actually loads.
- **`max_concurrency` and session counts are per node.** There is no
  cluster-wide view, and a node can claim a job it cannot fit while another
  node is idle. Multi-GPU models, cluster placement and capacity-aware claims
  are not built.
- **`multipart.file_url` must be `http://` or `https://`.** Local files are
  refused on purpose, so a job submitter cannot make a node upload its own
  files.
- **Result-cache keys hash the input**, so a `file_url` pointing at changed
  content still hits. Include a content hash or version in the URL or `params`.
- **Archived jobs are not checked for idempotency keys.** Re-submitting a key
  after its job was archived (`archive_after_secs`) creates a new job.
- **The model registry is fixed at startup.** Changing a model needs a restart.
- **Inputs are URLs and outputs are inline JSON.** There is no object storage
  for large payloads.
- **No per-tenant limits** and no per-queue permissions.
- **Measured usage is reported, not acted on** (see above).
- **Unverified against real software:** vLLM, Ollama, whisper.cpp, the
  container runtime, Hugging Face downloads, and nodes on separate machines.
- The `ai.*` tables are not exposed through Postrust's REST or GraphQL API.
