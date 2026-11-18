# What the conformance harness has found

Running notes on the differential runs against PostgREST v16.1. See
[README.md](README.md) for what the harness is and how to run it.

Three things are recorded here that the commit history does not hold:

- **faults in the instrument itself**, which matter more than the score —
  a harness fault either invents work or hides it, and the ones that hide it
  are indistinguishable from success;
- **divergences kept on purpose**, so that nobody later "fixes" a case that is
  failing because PostgREST is wrong;
- **known gaps**, with the reason each is a piece of work rather than a
  correction.

> **Provenance.** Part of an earlier revision of this file was lost when the
> machine filled its disk (fault 10). Instrument faults 1–8 are not fully
> recovered; the two that were inflating the score are restated below because
> they change how the number reads. Everything from run 14 onward is first-hand.
> The per-fix narrative lives in the commit messages, which is the more durable
> place for it.

## Results

Measured over 1499 replayed cases. "Strict" is the full contract: status, body,
and the six headers that are part of the answer rather than of the transport —
`Content-Type`, `Content-Range`, `Location`, `Preference-Applied`, `Allow`,
`WWW-Authenticate`. Not *every* header: `Date`, `Server` and `Connection`
differ between any two servers and say nothing about conformance.

| run | status + body | strict | note |
|-----|---------------|--------|------|
| 12  | 90.3% | 83.0% | before the harness configuration was corrected |
| 13  | —     | —     | reference only; candidate half cut short (see below) |
| 14  | 96.0% | 87.1% | committed binary, corrected reference |
| 4   | 95.9% | 93.1% | **not publishable** — measured without `compat-key-order` |
| 8   | —     | —     | died at `writes 300/431`; the machine had filled its disk |
| 9   | 96.1% | 94.3% | clean: both halves complete, 0 transport failures, features verified at runtime |
| 10  | 96.7% | 94.9% | computed relationships; 1 unanswered request, counted against us |
| 11  | 96.9% | 94.3% | 3.3.0; 11 cases lost to `Allow` on a 405, added in 2.4.0 and never recorded here |
| 15  | 99.3% | **99.1%** | 3.4.0; 0 status differences; the 13 left are listed under Part 2 and Part 3 |

Run 15 (3.4.0, commit `730b801`) by group: reads 99.1% / 98.9%, writes 99.8% /
99.8%, and the status code agrees in all 1499 cases. It reused the reference
recorded for run 11, which reused run 9's. It closed 72 cases, of two kinds:

- **Bugs, fixed in both modes.** Many-to-many embeds with the
  junction's columns in scope (`42702`) and through views counted twice
  (`PGRST201`); composite-key junctions whose parent key was not selected;
  inherited partition foreign keys; views without the primary key of the table
  they select from (`Location`, `Allow`); domain defaults; the count total
  after a write; `Content-Range` on a 416; negative offsets; a handler's own
  `Content-Type`; `Accept` lists; representations and casts inside embeds;
  `RETURNS TABLE` functions ignoring `select`; upsert statuses on views and
  partitions; `Range` on methods other than GET; `PGRST128`.
- **PostgREST behaviour that compatibility mode now matches and default mode
  keeps away from.** Listed in Part 2, with what each costs default mode.

**Run 11 is the 3.3.0 measurement, and its 10-case drop from run 10 was a
bookkeeping gap rather than a regression.** Bisected, it is one commit:
`ddf7530` (2.4.0) gave every 405 an `Allow` header, as RFC 9110 asks.
PostgREST sends none on those responses. The change was deliberate. The miss
was that it was never entered in Part 2, so the published figure went stale
when 2.4.0 shipped. Run 10's code replayed against the same reference still
scores 1425 (one more than run 10 itself, the socket fault), which is how the
instrument was cleared. The 3.4.0 run is numbered 15 because 12–14 were
already taken.

Run 10 by group: reads 96.3% / 94.5%, writes 97.9% / 96.1%. It reused run 9's
reference, which is what `CONFORMANCE_REUSE_REF=1` is for: nothing that shapes
a request had changed, only the candidate. Run 9's own reference had to be
re-recorded, because `run.py`'s `encode_path` had changed and a reference
records the answers to requests *as they were sent*.

Run 9 by group: reads 95.6% / 93.8%, writes 97.2% / 95.4%. It is the first run
whose provenance was recorded rather than remembered — see below; run 10 is
the first where the harness wrote that record itself.

**One request in run 10 got no answer at all.** `Query/InsertSpec.hs:604`, a
`POST` carrying multibyte UTF-8, failed with `BadStatusLine` naming its own
request line back — the signature of a socket carrying somebody else's
leftovers. It did not reproduce in 440 attempts of the identical request, the
server logged no error, and the code path is untouched by anything in that
run; 400 requests leave 440 sockets in `TIME_WAIT` on this host, which is how
a reused ephemeral port becomes possible over 1499 of them. Treated as
environmental, and counted as a failure regardless, so the figures understate
by one case. `report.py` now names such a case rather than letting it dissolve
into the totals: a flaky socket must not read as a conformance gap, and a real
one must not read as a flaky socket.

Run 14 confirmed the three specs `0d8ef2f` claimed: CustomMedia 19/50 → 50/50,
PostGIS 1/13 → 13/13, Plan 4/29 → 29/29, ExtraSearchPath 8/9 → 9/9.

**Run 13 was stopped once its reference was written.** The reference is the
expensive half and the only half that had changed; its candidate half would
have measured a binary the probe runs had already characterised. Set
`CONFORMANCE_REUSE_REF=1` to replay a candidate against a recorded reference —
the reference only changes when PostgREST's version, the fixtures, or the
harness configuration change.

## Part 1 — faults in the instrument

**`db-extra-search-path` was never set.** Both servers answered `42883`,
function does not exist, to every request touching PostGIS or `isn`. They
agreed perfectly and agreement scored as a pass, so an entire category was
reporting success while measuring nothing. Fixed in `0d8ef2f`.

**Header literals kept their Haskell escapes**, so eight cases were replayed
with a header the spec never wrote, and eight results described a request
nobody had made. Also `0d8ef2f`.

**The harness built without `compat-key-order`.** PostgREST returns object keys
in select order; `serde_json::Map` is a `BTreeMap` unless `preserve_order` is
on, so this server returns them alphabetically. The workspace already had the
feature, and the server already logged a warning when compatibility mode was
enabled without it — so it was announcing a known incompatibility to anyone
reading its logs while the instrument measured the incompatible build. It is
invisible in JSON, because bodies are compared as parsed JSON and key order
does not survive parsing. A CSV response puts its columns in key order, and
there it is the whole answer.

> **The conformance number is for the compatibility build.** That is the
> configuration the harness runs in every other respect too:
> `PGRST_COMPAT_MODE=true`, PostgREST's paths, verbatim database errors.

**The machine ran out of disk mid-session.** `cargo test --workspace` failed
with `ENOSPC`, and then so did every subsequent command — including `df`,
because each command's output is written to a file before being read. A full
disk takes the tools away at the same moment it takes the build away. Docker
died with it. `target/debug` was 1.2 GB and had no reason to exist during a
release-binary session.

The lesson is scheduling, not cleanup: this project's builds are large, the
harness keeps a fixture database and two containers alive, and a session that
runs both has to leave room for both. Recorded because the failure mode is
silent until it is total — the first symptom was an unrelated-looking test
failure.

**`cargo test --workspace` silently strips the candidate's features.** It
rebuilds the same `target/release/postrust` with default features and
overwrites it, so *build, then test, then measure* measured a binary with
neither `admin-ui` nor `compat-key-order`. Nothing said so. The only visible
sign was the file shrinking by 1.1 MB:

    built with features                    5,717,136
    after cargo test --release --workspace 4,559,968

This is the same class as the `db-extra-search-path` fault: the instrument
quietly measuring something other than what it claims. The harness now builds
the binary itself rather than requiring one to exist, and asserts against the
server's own startup warning once it is up, so neither half can drift again.

**The replay client dropped everything after a `#`.** `run.py` builds requests
with `urllib.request.Request`, which reads `#` as beginning a fragment and
strips it and the rest of the URL:

    Request('http://h/items?select=data->!@#$%^&*_d').full_url
    -> 'http://h/items?select=data->!@'

A fragment is a client-side notion that never appears in a request target, so
in a PostgREST query the character is a literal — and `lenient_uri.rs` carries
a paragraph about handling exactly that. It was never exercised. Both servers
received the truncated URL, agreed, and the case scored as a pass on a request
neither spec wrote. `encode_path` now sends `%23`, which both servers decode
back to `#` at the parser. What that still does not exercise is a *raw* `#` on
the wire; that needs a client that writes its own request line.

**A case the candidate never answered was dropped from the denominator.**
`report.py` skipped any reference case with no matching candidate record, which
can only move every percentage up. It counts and names them now. The two halves
replay the same file, so the count should always be zero — which is the point:
a silent zero and a silent skip look identical until one of them isn't.

**A run did not record what it had measured.** Run 4's figures were quoted for
weeks before anyone could say which features its binary carried, and the answer
turned out to be "not the ones that matter". The harness now writes
`run-meta.json` beside `diff.json` — reference version, build features, commit,
date — and `scripts/gen-conformance.mjs` refuses to publish a run without it,
or one whose features do not include `compat-key-order`. A number that cannot
say what produced it is not a measurement.

**Non-compatibility mode is not measured at all.** The harness runs
`PGRST_COMPAT_MODE=true`, where the REST surface answers at the root. In the
default mode it is mounted under `/api`, and a bug that only shows there is
invisible here however green the report is — as one was: every `OPTIONS`
preflight outside compatibility mode answered `404 PGRST125`, because the
`Allow` middleware read the path before the mount prefix was stripped off it.
Found by review, not by the harness.

## Part 2 — divergences kept on purpose

Two kinds are recorded here. Some answers differ in default mode by choice and
match in compatibility mode, which is the configuration the harness measures,
so they cost nothing in the figures. Others are cases the harness cannot score
fairly. **Do not "fix" any of them without deciding to.**

### Matched in compatibility mode only

Each of these is PostgREST behaviour that a client written against PostgREST
may depend on, and that default mode answers differently on purpose. Since
3.4.0 compatibility mode matches all of them; before that each one was a
failure in the figures. [`docs/compat-mode.md`](../../docs/compat-mode.md)
lists them for users.

- **A select cut short at a stray character.** Probed against the reference:

  ```
  /clients?select=id)ZZ,nameQQ   ->  200, and nameQQ never becomes a column
  /clients?select=name)))        ->  200
  /clients?select=name,,,        ->  400
  ```

  PostgREST's parser stops at the first character it cannot use and discards
  the rest silently. Commit `f7c7b56` ("stop silently discarding half of a
  select") removed exactly that from this server, where `select=id, name,
  billing(address)` had been returning the id alone. Default mode still refuses
  with `PGRST100`. Compatibility mode drops the tail, except a tail beginning
  with `,`, which PostgREST refuses too (`AggregateFunctionsSpec.hs:34`,
  `SpreadQueriesSpec.hs:391`).
- **`Allow` on a 405** (11 cases, `UpsertSpec` and `RpcSpec`). RFC 9110 requires
  it, and default mode sends it (`ddf7530`, 2.4.0). PostgREST sends none.
- **`Allow` on a CORS preflight** (3 cases, `CorsSpec`). PostgREST's CORS
  middleware answers a preflight before the application sees it. A plain
  `OPTIONS` carries `Allow` in both servers.
- **`Content-Type` for `application/vnd.pgrst.array+json`**
  (`NullsStripSpec.hs:117`). PostgREST answers `application/json` unless
  `nulls=stripped` was asked for. Default mode names the type the client
  negotiated.
- **An unknown column in `select`** (`QuerySpec.hs:1557`). PostgREST does not
  check the column, so PostgreSQL's `42703` comes back. Default mode refuses it
  before the query with `PGRST204`, which names the table.
- **`GET /` with an `Accept` the root cannot produce** (`OpenApiSpec.hs:36`).
  PostgREST refuses with 406 `PGRST107`. Default mode's `/` is a directory of
  endpoints, which is JSON whatever was asked.

### What the harness cannot score

*6 cases. The harness is over-reporting here, not the server under-performing.*

- **Unspecified row order** (`RelatedQueriesSpec.hs:183`, `:191`, `:218`, and
  `QuerySpec.hs:995`, whose `order=` leaves two rows tied). Both servers return
  the same rows in a different order. SQL guarantees nothing there, so both
  answers are correct; the body comparison is exact. Matching would mean
  shaping the SQL after PostgREST's to chase its plan, which no release of
  either server promises to keep.
- **A spec configured differently from the harness** (`QuerySpec.hs:1700`).
  The spec runs with `url-use-legacy-target-names = false`. The harness runs
  one PostgREST for every spec, with the default `true`, under which PostgREST
  applies every filter to `work_visits`. Neither server gives the answer the
  spec expects; the setting itself is a gap, in Part 3.
- **A test PostgREST does not run** (`InsertSpec.hs:634`). The spec marks it
  `pendingWith "Decide on what to do with CSV insert"`, and the extractor lost
  its body, which is built with `<>`, so the harness sends an empty `text/csv`
  `POST`. Both answer 400 `PGRST102`; only cassava's wording differs.

### Clock skew on `nbf` and `iat`, and none on `exp`

PostgREST checks all three to the second. This server allows thirty seconds of
slack on `nbf` and `iat` and none on `exp`, which is a deliberate asymmetry
rather than an oversight in either direction.

`nbf` and `iat` describe a token that is not valid *yet*. A client whose clock
runs a little fast mints one a few seconds in the future through no fault of
anyone's, and refusing it makes a working deployment fail intermittently and
unreproducibly. Slack there costs nothing an attacker can use.

`exp` is the other way round. Leniency about an expiry is leniency about a
token that has been *withdrawn*: it keeps a session alive past the second its
issuer said it ended, and hands anyone holding a stale token a window for free.
The RFC permits leeway there; this does not take it.

*Cost: 0 cases — no case in the suite mints a token near either boundary,
which is why this is recorded here rather than left to be inferred from a
green report.*

## Part 3 — known gaps, in the order worth doing them

| # | gap | cases | state |
|---|-----|-------|-------|
| 1 | Ordering across an embed boundary | ~4 | **done** |
| 2 | Composite-key junctions | 4–5 | **done** |
| 3 | Junctions through views | ~2 | **done** |
| 4 | Parser error detail | ~3 | **done** for the logic tree and for a JSON operand in `select` (3.4.0); other select errors still answer generically |
| 5 | The OpenAPI document at `/` | 7 | open — see below |
| 6 | `url-use-legacy-target-names = false` | 1 | open — not configurable, and a filter on `visits.` also reaches an embed aliased from `visits` |

### 1. Ordering and filtering across an embed boundary — ~4 cases

Four failures, one shape: an identifier resolved at the wrong nesting level.

- Ordering an embed by a column of *its* embed — `tasks.order=projects(id).desc`
  returns the wrong row. **This is the one to fix first:** status 200, a
  well-formed body, and a different row than asked for. Nothing signals it.
- Ordering a spread by a column from a *deeper* spread —
  `...processes(name,...process_costs(cost))&processes.order=process_costs(cost)`
  → `column "cost" does not exist`. Single-level spread ordering works
  (`SpreadQueriesSpec` `:302` and `:563` pass), so it is specifically the extra
  level.
- A two-level filter path — `child_entities.grandchild_entities=not.is.null`
  → `PGRST204 Column not found`, when the last segment names an embedded
  resource being existence-tested rather than a column. The one-level form
  works.

### 2. Composite-key junctions — 4–5 cases

Every failing many-to-many junction has composite foreign keys: `touched_files`
joins `files(project_id, filename)` to `users_tasks(user_id, task_id)`, and
`car_models_car_dealers` joins two two-column keys. PostgREST embeds through
both.

The derivation was written and then **reverted deliberately**. `EmbedPlan`
carries one `local_column` and one `foreign_column`, `EmbedJunction` one column
per side, and `parent_keys` matches children to parents on a single scalar.
Deriving the relationship without widening all of that joins a composite
junction on the first column of each key and returns rows that are **wrong
rather than absent** — and much harder to notice than the error it replaces.

The reasoning is recorded in place at `add_junction_relationships`, and the
column plumbing there is already generalised behind a `len() == 1` gate, so the
remaining work is the embed layer:

1. `EmbedPlan::{local_column, foreign_column}` → column lists
2. `EmbedJunction::{parent_column, child_column}` → column lists
3. the three correlated-subquery join sites in `embed.rs` (mechanical: AND the
   conditions)
4. `parent_keys` / `key_to_text` → tuple keys, for the batched path that matches
   children to parents in Rust
5. the same fields in `postrust-graphql`'s handler

This comes before item 3: a view used as a junction over a composite key needs
the widened path anyway.

Note the same single-column assumption sits on the ordinary foreign-key embed
path (`columns.first()` in `EmbedPlan::resolve`). That is latent rather than
observed: no failing case pins it, but a composite-key embed is joining on one
column today.

### 3. Junctions through views — ~2 cases

`sites` embeds `big_projects` through `jobs` and also through `main_jobs`, a
view over it. We derive the first and not the second, so a hinted
`big_projects!main_jobs` finds no relationship, and the ambiguity list
`PGRST201` returns is short by one candidate.

`substitute_hidden_junctions` already routes a many-to-many through the exposed
view of a hidden junction; this is the other direction — a view that is *itself*
usable as a junction alongside the table it selects from.

### 4. Parser error detail — ~3 cases

`?or=()` and some JSON-path failures answer
`{"message":"Invalid request","details":null}` where PostgREST names the
character and what would have been accepted:

```
"failed to parse logic tree (())" (line 1, column 4)
unexpected ")" expecting field name (* or [a..z0..9_$]), negation operator (not)
  or logic operator (and, or)
```

The machinery exists — `Error::UnparsableQuery`, built by `a5b6c94` for filters
and used again for `?columns=`. What is left is carrying expectation sets
through the select and logic-tree parsers so the message can be assembled.
Matching PostgREST exactly means reproducing its parser combinator's
expectation sets, which is why this is worth doing for its own sake rather than
for the three cases.

### 5. The OpenAPI document at `/` — 7 cases

Still open, and the only item here that is a project rather than a change.
What the reference actually serves, measured:

    638 KB      428 paths      273 definitions      1035 parameters

The shape is regular, which is the argument for doing it and also the reason
it cannot be done by halves:

- **paths** — `/`, one per table and view with a method per operation the
  relation supports, one per function under `/rpc/`;
- **parameters** — 11 shared (`select`, `order`, `range`, `limit`,
  `preferCount`, …), one `rowFilter.<table>.<column>` for every column of
  every relation, one `body.<table>` for every writable one;
- **definitions** — one per relation, a property per column.

The detail is where the work is. A column carries PostgREST's own conventions,
not just its type:

    "id": {
      "description": "Note:\nThis is a Primary Key.<pk/>",
      "format": "int64",
      "type": "integer"
    }

so matching means reproducing its type-to-(type, format) mapping, its
`<pk/>`/`<fk/>` annotations, how a column comment is folded into the
description, which columns count as `required`, and the same again for every
function signature.

**Bodies are compared as exact JSON, so a 95%-correct generator scores zero on
all five `GET /` cases.** The other two are `HEAD /`, whose `Content-Type`
(`application/openapi+json`) waits for the document: claiming the type for the
directory served today would be wrong. The 406 refusals beside them were
cheap, and closed in 3.4.0. There is no partial credit to collect and no way to land it
incrementally against this measurement — which is why it belongs in its own
change, with its own tests, rather than being started here and left half-built.

The admin surface already emits OpenAPI 3.0 through `utoipa`
(`postrust-server/src/admin.rs`). It is not a starting point: it describes the
admin endpoints, and PostgREST emits Swagger 2.0 describing the data API.

## Part 4 — a note on where bugs hide

Two patterns recurred often enough to be worth naming.

**A permissive fallback turns a syntax error into a plausible success.**
`?columns=` produced a column named `""` because `"".split(',')` yields one
empty piece rather than none, and that name travelled to the schema cache and
came back as "Could not find the '' column" — describing a schema problem when
the URL named no field. `data->>--34` became a key literally named `--34` and
answered 200 with nulls. Neither failed loudly.

**A fix can be silently absorbed by the grammar it is fixing.** Rejecting
`--34` with a recoverable nom `Error` did nothing visible: `alt` backtracked
from `->>` to `->`, and the rest was re-read as a key beginning with `>` —
still 200, now under a different name. It took `Err::Failure` to stop it, which
is also the semantically correct signal: once `->>` is consumed, no other rule
should get to reinterpret it. The test asserting `is_err()` mattered more than
the code change did.

**A measurement only covers the configuration it runs in.** Every number here
is for compatibility mode. The `OPTIONS` bug above was a whole class of request
answered `404` in the *default* configuration while the report said the `Allow`
work had landed — because in compatibility mode the surface is at the root, and
the middleware's mistake was reading a path that still had `/api` on the front
of it. A 93% that does not say what it is 93% *of* invites exactly this.

The general shape: `Router::layer` wraps each endpoint, and `nest` puts its
prefix-stripping **inside** that endpoint. Anything added with `layer` sees the
path the client wrote, not the path the handler reads. Middleware that parses
the path has to account for the mount it is running above.
