# Hasura conformance harness

Measures how closely Postrust's GraphQL surface matches Hasura v2's, using
Hasura's own fixtures and its own test cases.

```bash
cargo build --release -p postrust-server --features admin-ui
scripts/hasura-conformance/conformance.sh
```

## Why this one is easier than the PostgREST harness, and where it is harder

PostgREST's spec suite cannot be pointed at another server, so
`scripts/conformance/extract.py` lifts request literals out of Haskell.
Hasura's Python suite has no such problem: `conftest.py` takes `--hge-urls`
and drives whatever engine is listening, and every case is declarative YAML
carrying a url, a GraphQL payload, the headers to send and the status to
expect. There is nothing to parse out of source.

The hard part is the other half. PostgREST reflects a schema, so loading
fixtures and starting the server is the whole setup. Hasura is *configured*:
a group's `setup.yaml` is a metadata payload that creates tables with
`run_sql`, tracks them, names every relationship and grants every permission.
None of that vocabulary exists here.

## How the database is made to agree

The two servers are set up by different means, and the asymmetry is the point.

The reference is configured the way the suite intends — each fixture file is
POSTed to Hasura's own API, so tracking, relationships and permissions are
established by the engine that owns them. Immediately afterwards the database
is dumped.

The candidate never sees a fixture file. It gets the dump.

Translating Hasura's metadata commands into something Postrust understands was
the obvious approach and the wrong one. A translator that got a column type or
an insert order subtly wrong would surface as a divergence *in the server* —
which is the one failure mode a differential harness exists to rule out.
Restoring the reference's own database removes the translation, and with it
the question.

One consequence: the candidate phase depends on the reference phase having
run, because it reads the dumps that phase produced. They are cached in
`.work/dumps/` alongside `ref.json` and reused by
`HASURA_CONFORMANCE_REUSE_REF=1`.

## Pieces

| File | Role |
|---|---|
| `conformance.sh` | Driver: fetch, database, both servers, report |
| `extract.py` | Decides which cases are replayable and against which fixtures |
| `run.py` | Replays a case set against one server, records raw responses |
| `report.py` | Diffs two runs and classifies every divergence |

Artifacts land in `scripts/hasura-conformance/.work/`, including `diff.json`
with per-case detail for every divergence.

## What is replayed, and what is not

At v2.50.1 the corpus holds 1,675 YAML files. After dropping the mssql,
bigquery and citus variants, 468 cases in 61 groups address `/v1/graphql` or
`/v1alpha1/graphql` and can be set up from fixtures alone.

The rest are excluded for a reason `extract.py` prints rather than hides:

- **~150 cases need a subsystem this server does not have** — actions, remote
  schemas, remote relationships, event triggers, inherited roles, query
  collections. A case whose setup cannot run measures nothing, so counting it
  as a failure would only make the denominator dishonest.
- **~250 cases address the metadata API** (`/v1/query`, `/v1/metadata`,
  `/v2/query`) rather than the GraphQL endpoint. Postrust reflects its schema
  instead of being configured, so these describe a contract it does not offer.
- **49 Relay cases** (`/v1/relay`) and the REST-endpoint cases (`/api/rest/…`)
  are separate surfaces.

Three details in the layout are worth knowing, because each of them silently
loses cases if missed:

- **A case file is either one case or several.** A file that begins with `-`
  is a sequence — an insert followed by the select that reads it back — and
  its cases are ordered on purpose. Reading only mapping-shaped files finds
  292 cases; reading both finds 468.
- **The fixture family has four members.** `pre_setup`, `schema_setup`,
  `setup` and `values_setup` apply in that order, and a directory with only
  `schema_setup.yaml` is as much a group as one with `setup.yaml`.
  `conftest.py` falls back through the same list.
- **The corpus straddles two APIs.** `run_sql` and the unprefixed commands are
  `/v1/query`; the source-aware spellings — `pg_track_table`,
  `pg_add_computed_field` — are `/v1/metadata`, and posting one to the other's
  endpoint is rejected outright. Commands are sent individually rather than as
  the `bulk` they arrive in, because a file that mixes the two has no single
  endpoint that would accept it.

## How state is kept identical

Every case has to start from the same database, or one divergence cascades
into phantom failures in everything after it.

- **Each group is loaded from scratch**, and the candidate is restarted with
  it: the schema cache is read once at startup and the GraphQL schema is built
  from a snapshot of it, so a table created after the process started is
  invisible to it.
- **Reads run first**, as one block from that clean load.
- **Each mutating file gets the data restored immediately before it**, from a
  data-only dump. No DDL runs, so object OIDs never change and no restart is
  needed between files.

## Roles

Header selection mirrors `validate.py`'s `check_query`, because the corpus
relies on it: the admin secret is attached to every case, and a case that also
names `X-Hasura-Role` is asking to be treated as that role. Both servers
receive identical headers.

The two halves of that are one mechanism, not two. Naming a role is not an
alternative to authenticating — it is something an authenticated caller asks
for, and a role header arriving on its own is an unauthenticated request.
Reading it as an alternative is what this harness did until run 38, and it
cost every permission case in the corpus: the reference answered all 142 with
`"x-hasura-admin-secret" required, but not found` and never reached its
permission layer, while the report called them a difference between two
permission models. `run.py`'s docstring carries the detail.

Postrust has no metadata-defined permissions — it has database roles and row
level security — so the permission cases are expected to diverge. They are
counted rather than excluded, because the size of that gap is one of the
things worth knowing.

## Reading the output

Agreement is reported at four strictness levels. GraphQL answers almost
everything with 200, so status agreement says very little on its own, and a
strict body comparison says too much at first — error text is the last thing
to match and would hide every case that differs in nothing else.

- **HTTP status** — did the transport agree at all?
- **+ same outcome** — did the client branch the same way: data, or errors?
- **+ same data payload** — the number that matters. Did the same query come
  back with the same rows?
- **full body** — strict, error text included.

A case both servers answer with errors counts at the outcome level and again
at the strict level, never in between: agreeing that something failed is real
agreement, but it is not a data match and should not be counted as one.

## Known measurement limits

- **The corpus is not representative traffic.** A test suite concentrates on
  edge cases and error paths by design.
- **Relationship names are Hasura's to choose, not the schema's.** Every
  relationship in the corpus is named by a metadata command a human wrote.
  Postrust derives names from foreign keys, so where the fixture chose
  something other than the convention, the field simply is not there under
  that name. This is a structural limit of reflecting instead of configuring,
  not a bug to be fixed.
- **Some divergences are intentional**, permissions foremost. They are
  counted, not excluded.
