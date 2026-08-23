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
and every header including `Content-Range`.

| run | status + body | strict | note |
|-----|---------------|--------|------|
| 12  | 90.3% | 83.0% | before the harness configuration was corrected |
| 13  | —     | —     | reference only; candidate half cut short (see below) |
| 14  | 96.0% | 87.1% | committed binary, corrected reference |

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

## Part 2 — divergences kept on purpose

**These cases fail by choice. Do not "fix" them without deciding to.**

### PostgREST truncates a select at a stray `)`

Probed against the reference directly:

```
/clients?select=id)ZZ,nameQQ   ->  200, and nameQQ never becomes a column
/clients?select=name)))        ->  200
/clients?select=name,,,        ->  400
```

Everything after the `)` is discarded silently. That is exactly the behaviour
commit `f7c7b56` ("stop silently discarding half of a select") removed from
this server, where `select=id, name, billing(address)` had been returning the
id alone. Matching it means reintroducing a bug that was fixed on purpose.

*Cost: 2 cases* (`AggregateFunctionsSpec.hs:34`, `SpreadQueriesSpec.hs:391`).

### Upsert status codes

`POST /articles` with an empty body returns 201 where PostgREST returns 200,
and a `PUT` that replaced an existing row returns 201 where PostgREST returns
200. The evidence is one case each; `mutation_status` carries comments
recording earlier deliberation about exactly these lines; and UpsertSpec passes
58/60. Changing the rule on this evidence risks the 58 to win the 2.

*Cost: 2 cases.*

### Unspecified row order

`RelatedQueriesSpec.hs:183` and `:191` return the same two rows in the opposite
order from PostgREST. Neither request specifies `order=`, so SQL guarantees
nothing and both answers are correct. It shows as a failure only because bodies
are compared exactly. Matching it would mean inventing an ordering PostgREST
does not promise.

*Cost: 2 cases. The harness is over-reporting here, not the server
under-performing.*

## Part 3 — known gaps, in the order worth doing them

Real work, not corrections. Roughly 19 cases across five pieces.

| # | gap | cases | needs | risk if left |
|---|-----|-------|-------|--------------|
| 1 | Ordering across an embed boundary | ~4 | resolve identifiers at the right nesting level | **silently wrong answers** |
| 2 | Composite-key junctions | 4–5 | widen the embed path to tuple keys | missing embeds, loud |
| 3 | Junctions through views | ~2 | derive junctions from views, not just tables | missing embeds, loud |
| 4 | Parser error detail | ~3 | expectation sets in the query parser | poor diagnostics |
| 5 | The OpenAPI document at `/` | 6 | a PostgREST-shaped generator | endpoint absent |

Ordered by consequence rather than size: item 1 is first because it is the only
one that answers wrongly instead of refusing, and item 5 is last because it is
the only one that is a project rather than a change.

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

### 5. The OpenAPI document at `/` — 6 cases

638 KB, 428 paths, 273 definitions, generated from every table, view, column
and function in the schema, with `info` taken from the schema comment. Every
column becomes a `rowFilter.<table>.<column>` parameter; every relation becomes
a definition.

Bodies are compared as exact JSON, so a 95%-correct generator scores **zero on
all six cases**. There is no partial credit to collect, and no way to land it
incrementally against this measurement — which is the argument for treating it
as a project with its own tests rather than as conformance work.

Note the admin surface already generates OpenAPI 3.0 via `utoipa`
(`postrust-server/src/admin.rs`), which describes the admin endpoints and is
not a starting point for this: PostgREST emits Swagger 2.0 describing the data
API.

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
