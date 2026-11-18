# PostgREST conformance

Postrust follows PostgREST's URL grammar deliberately: `?select=`, `?order=`,
the filter operators and the `Prefer` headers mean the same thing in both. How
closely is not a matter of opinion here — it is measured, against PostgREST
itself, and this page says what the measurement covers and where the two
servers disagree on purpose.

## Where it stands

Run 15, over 1499 replayed cases, against PostgREST v16.1 (Postrust 3.4.0):

| Compared on | All (1499) | Reads (1068) | Writes (431) |
|---|---|---|---|
| Status code | 100.0% | 100.0% | 100.0% |
| Status and body | 99.3% | 99.1% | 99.8% |
| …and headers, except `Content-Range` | 99.1% | 98.9% | 99.8% |
| Full contract | **99.1%** | 98.9% | 99.8% |

That is 1486 of 1499 cases agreeing on everything compared. Of the 13 that do
not, seven are [the OpenAPI document at `/`](#known-gaps) and six are cases the
harness cannot score fairly: rows in an order no request asked for, one spec
that needs a setting the harness does not apply, and one test PostgREST itself
does not run.

Measured on a binary the harness built itself and then verified at runtime.
See [`FINDINGS.md`](../scripts/conformance/FINDINGS.md) for the run history,
including the runs whose numbers are not publishable and why.

## How it is measured

PostgREST's own spec suite cannot be pointed at another server: it drives the
WAI application in-process and imports `PostgREST.Config` directly, so there is
no HTTP boundary to intercept. Two parts of it are reusable — the fixture
database (~280 tables of plain SQL) and the request literals inside the
examples, each of which spells out a method, path, headers and body.

So the harness lifts those requests out of the Haskell and replays them over
HTTP against **both** stock PostgREST and Postrust, each on an identically
loaded fixture database, and diffs the live responses.

The reference implementation is the oracle. No hspec expectation is ever
interpreted, which means a mistake in the extractor shows up as a case both
servers answer the same way rather than as a false failure.

```bash
scripts/conformance/conformance.sh
```

Takes about ten minutes per server, most of it restoring fixture data between
mutating cases. See [`scripts/conformance/README.md`](../scripts/conformance/README.md)
for the mechanics.

## Through other drivers and hosts

The same suite, with the same reference responses, against the REST pipeline
on its other driver and in its other host (issue #50):

| Candidate | Full contract | Status and body | What differs from the server |
|---|---|---|---|
| The server (sqlx) | 1424/1499 | 1451 | — |
| The server, REST through the socket driver (`CONFORMANCE_EXTRA_FEATURES=pg-socket`) | 1423/1499 | 1450 | One case, below |
| The Cloudflare Worker in workerd (`CONFORMANCE_WORKER_BUILD=...`) | 1422/1499 | 1449 | Two cases, below |

The socket driver answers the suite as sqlx does, except in
`PreparedStatementsSpec`. That case asks `pg_prepared_statements` whether the
session holds named statements. sqlx caches them, so the answer is yes. The
socket driver sends every statement unnamed, in one round trip, because its
connection never outlives the request that opened it.

The Worker runs on the socket driver, so it differs in that case too, and in
one more: the `CorsSpec` preflight that sends `Accept-Encoding: gzip`. workerd
compresses the empty response body. The bytes are a valid gzip stream of
nothing, so a client that decodes it sees the empty body the server sends;
the harness compares raw bytes.

The Worker's first run scored 1405. It found a real bug: a request whose body
the pipeline never read, such as a write refused for its token, left the
request stream to throw after the response. That restarted the isolate and
failed the next request. The Worker now reads the body itself before the
pipeline sees it. `scripts/serverless/worker.sh` checks that sequence, and
also checks the Worker's log for uncaught exceptions.

## What "conformance" counts

Agreement is reported at four strictness levels, because one systemic gap — a
single header never emitted — would otherwise sink every case and hide the
hundreds that differ in nothing else.

The strictest is **status, body, and six headers**: `Content-Type`,
`Content-Range`, `Location`, `Preference-Applied`, `Allow`, and
`WWW-Authenticate`. Those are part of the answer. `Date`, `Server` and
`Connection` differ between any two servers and say nothing about conformance,
so they are not compared.

Bodies are compared as parsed JSON, so whitespace and formatting do not count
against either server — but **object key order does not survive parsing**, and
a CSV response puts its columns in key order. See
[key ordering](configuration.md#key-ordering-a-build-time-choice).

**The figures are for the compatibility build**, which is the configuration
the harness runs in every respect: `PGRST_COMPAT_MODE=true`, PostgREST's paths,
verbatim database errors, and `--features admin-ui,compat-key-order`. A
default-mode deployment is not what is measured.

## Where the two disagree on purpose

Some cases fail because PostgREST is wrong, or because neither answer is wrong.
They are listed here so that nobody later "fixes" one without deciding to.
[`FINDINGS.md`](../scripts/conformance/FINDINGS.md) carries the evidence.

**Default mode, by choice.** Six PostgREST behaviours are matched in
compatibility mode and avoided by default:
- a `select` cut short at a stray character and the rest discarded;
- no `Allow` on a 405;
- no `Allow` on a CORS preflight;
- `application/json` as the type of an `application/vnd.pgrst.array+json`
  response;
- `42703` from PostgreSQL for an unknown `select` column;
- a 406 for `GET /` with an `Accept` it cannot produce.

The figures above measure compatibility mode, so these cost nothing in them.
[PostgREST compatibility mode](compat-mode.md) lists them with what default
mode does instead.

The first matters most. PostgREST answers
`/clients?select=id)ZZ,nameQQ` with 200 and `nameQQ` never becomes a column.
Postrust used to do the same: `select=id, name, billing(address)` returned the
id alone. It was fixed on purpose, so default mode refuses.

**Unspecified row order.** Four cases return the same rows in a different
order. Three requests specify no `order=`, and in the fourth two rows tie on
the column named. SQL guarantees nothing there, so both answers are correct
and the measurement is over-reporting.

**Clock skew on `nbf` and `iat`, and none on `exp`.** PostgREST checks all
three to the second. Postrust allows thirty seconds on the two that describe a
token not yet valid, and none on the one that describes a token withdrawn. See
[Authentication](authentication.md#how-the-claims-are-checked).

## Known gaps

**The OpenAPI document at `/`** (7 cases). PostgREST serves a 638 KB Swagger 2.0
document there: 428 paths, 273 definitions, 1035 parameters. Postrust serves
OpenAPI 3.0 for its own surface under `/admin` (behind the `admin-ui` feature)
and does not yet generate PostgREST's. Bodies compare as exact JSON, so this is
all-or-nothing rather than something to land incrementally. `HEAD /` waits for
it too, since its `Content-Type` names the document.

**`url-use-legacy-target-names = false`** (1 case). The setting is not read,
and a filter on `visits.` also reaches an embed aliased from `visits`.

**Parser error detail.** A logic tree and a JSON operand in `select` report the
offending character and what would have been accepted, as PostgREST does.
Other `select` errors still answer generically.

**Not every config knob.** `Prefer: tx=rollback` in particular is not
implemented, and is no longer reported as applied.

## Reading a failing case

`scripts/conformance/.work/diff.json` carries per-case detail for every
divergence: the request, both statuses, both bodies, and which headers differ.

Before treating one as a bug, read
[`FINDINGS.md`](../scripts/conformance/FINDINGS.md). It records the faults
found in the harness itself, which matter more than the score — a harness fault
either invents work or hides it, and the ones that hide it are indistinguishable
from success. Several have been found so far, including one that had an entire
category reporting success while measuring nothing.
