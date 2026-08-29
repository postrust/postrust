# PostgREST conformance harness

Measures how closely Postrust's HTTP surface matches PostgREST's, using
PostgREST's own test fixtures and its own test cases.

```bash
cargo build --release -p postrust-server --features admin-ui,compat-key-order
scripts/conformance/conformance.sh
```

Takes about ten minutes, most of it restoring fixture data between mutating
cases.

[FINDINGS.md](FINDINGS.md) records what the runs have turned up: faults found
in this harness itself, divergences kept on purpose, and the gaps still open.
Read it before treating a failing case as a bug — some of them are failing
because PostgREST is wrong and this server is not.

## Why this isn't just "run PostgREST's tests"

PostgREST's spec suite can't be pointed at a different server. `test/spec`
uses `Test.Hspec.Wai`, which drives the WAI `Application` in-process, and the
specs import `PostgREST.Config` and construct an `AppConfig` directly — there
is no HTTP boundary to intercept. (`test/io` does speak HTTP, but it tests
PostgREST's CLI, logging, and config reloading, not the API contract.)

Two parts of the suite are reusable:

- **the fixture database** — `test/spec/fixtures/*.sql`, ~280 tables, plain SQL
- **the request literals** — every example contains a method, path, headers,
  and body written out in full

So the harness lifts the requests out of the Haskell and replays them over
HTTP against **both** stock PostgREST and Postrust, each on an identically
loaded fixture database. Divergence is measured against the reference
implementation's live response, which means we never have to interpret an
hspec expectation — and any parsing mistake shows up as a case both servers
answer the same way, not as a false failure.

## Pieces

| File | Role |
|---|---|
| `conformance.sh` | Driver: fetch, load fixtures, run both servers, report |
| `extract.py` | Lifts request literals out of the `.hs` specs |
| `run.py` | Replays a case file against one server, records raw responses |
| `report.py` | Diffs two runs and classifies every divergence |

Artifacts land in `scripts/conformance/.work/`, including `diff.json` with
per-case detail for every divergence.

## How state is kept identical

Every case has to start from the same database, or one divergence cascades
into phantom failures in everything after it.

- **Reads run first**, as one block from a single clean load. None of them can
  disturb the others.
- **Each mutating case gets the fixture data restored immediately before it.**

The restore truncates every fixture table and reloads a data-only `pg_dump`
snapshot taken after the first full load. No DDL runs, so object OIDs never
change and neither server has to be restarted to drop a stale schema cache.

Two details make the snapshot necessary rather than just re-running
`data.sql`: some fixture rows are inserted by `schema.sql` and are absent from
`data.sql`, and `data.sql` is not idempotent — a few tables (`private.labels`
among them) are inserted into with no preceding `TRUNCATE`, so a second run
duplicates rows and its `(SELECT id FROM labels WHERE ...)` subqueries then
fail.

## Auth coverage

Examples whose header lists call Haskell helpers are resolved rather than
skipped. `extract.py` collects the `let` bindings in each spec file and
handles three forms: a literal bearer token, `generateJWT` over a literal JSON
payload (signed here with the suite's own HS256 secret from `SpecHelper.hs`),
and a plain `("Name", "Value")` pair. 23 of 1,699 request sites still resist
resolution.

## Reading the output

Agreement is reported at four strictness levels, split by reads and writes,
because a single systemic gap would otherwise mask everything behind it:

- **status code only** — would the client's error handling branch the same way?
- **status + body** — does the client get the same data?
- **+ headers, ignoring `Content-Range`** — isolates that one header's effect
- **full contract** — strict

## The raw `>` problem

69 cases in the corpus send a JSON operator (`select=data->>id`) with the `>`
unescaped, which is what `curl` does and what PostgREST's own documentation
shows. warp accepts it; hyper rejects the request line before it reaches any
routing or handler code, so Postrust answers 400 with an empty body.

This is a real drop-in divergence, and it is counted as one — the harness
sends what the spec suite sends. But it is a transport-layer difference with a
single cause, so it masks whatever the query layer does with those cases. To
see past it, re-run just those cases with the operator escaped:

```bash
python3 - .work/cases.json .work/cases-arrow.json <<'EOF'
import json, sys
cs = json.load(open(sys.argv[1]))
json.dump([dict(c, path=c["path"].replace(">", "%3E")) for c in cs if "->" in c["path"]],
          open(sys.argv[2], "w"), indent=1)
EOF
```

## Known measurement limits

- **The corpus is not representative traffic.** A test suite concentrates on
  edge cases and error paths by design. Everyday CRUD scores far higher than
  the suite average — measure both before quoting a number.
- **Some divergences are intentional.** The root endpoint serving server info
  rather than the OpenAPI spec, for one. They are counted, not excluded.
- `Content-Type` is compared without its `charset` parameter.
- Bodies are compared as parsed JSON where both sides parse, so object key
  ordering never affects a result — build with `compat-key-order` if you want
  to check ordering itself.
