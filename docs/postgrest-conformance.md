# PostgREST conformance

Postrust follows PostgREST's URL grammar deliberately: `?select=`, `?order=`,
the filter operators and the `Prefer` headers mean the same thing in both. How
closely is not a matter of opinion here — it is measured, against PostgREST
itself, and this page says what the measurement covers and where the two
servers disagree on purpose.

## Where it stands

Run 9, over 1499 replayed cases, against PostgREST v16.1:

| Compared on | All (1499) | Reads (1068) | Writes (431) |
|---|---|---|---|
| Status code | 98.2% | 97.8% | 99.1% |
| Status and body | 96.1% | 95.6% | 97.2% |
| …and headers, except `Content-Range` | 94.5% | 93.9% | 96.1% |
| Full contract | **94.3%** | 93.8% | 95.4% |

Both halves completed with zero transport failures, on a binary the harness
built itself and then verified at runtime. See
[`FINDINGS.md`](../scripts/conformance/FINDINGS.md) for the run history,
including the two runs whose numbers are not publishable and why.

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
[key ordering](configuration.md#compatibility-settings).

**The figures are for the compatibility build**, which is the configuration
the harness runs in every respect: `PGRST_COMPAT_MODE=true`, PostgREST's paths,
verbatim database errors, and `--features admin-ui,compat-key-order`. A
default-mode deployment is not what is measured.

## Where the two disagree on purpose

Some cases fail because PostgREST is wrong, or because neither answer is wrong.
They are listed here so that nobody later "fixes" one without deciding to.
[`FINDINGS.md`](../scripts/conformance/FINDINGS.md) carries the evidence.

**PostgREST truncates a select at a stray `)`.** Probed against the reference
directly, `/clients?select=id)ZZ,nameQQ` returns 200 and `nameQQ` never becomes
a column. Everything after the paren is discarded silently. Postrust rejects
it, because matching this means reintroducing a bug that was fixed on purpose —
`select=id, name, billing(address)` used to return the id alone.

**Two upsert status codes.** `POST` with an empty body and a `PUT` that
replaced an existing row return `201` where PostgREST returns `200`. The
evidence is one case each, against 58 that pass.

**Unspecified row order.** Two cases return the same rows in a different order,
and neither request specifies `order=`. SQL guarantees nothing there, so both
answers are correct and the measurement is over-reporting.

**Clock skew on `nbf` and `iat`, and none on `exp`.** PostgREST checks all
three to the second. Postrust allows thirty seconds on the two that describe a
token not yet valid, and none on the one that describes a token withdrawn. See
[Authentication](authentication.md#how-the-claims-are-checked).

## Known gaps

**The OpenAPI document at `/`.** PostgREST serves a 638 KB Swagger 2.0
document there — 428 paths, 273 definitions, 1035 parameters. Postrust serves
OpenAPI 3.0 for its own surface under `/admin` (behind the `admin-ui` feature)
and does not yet generate PostgREST's. Bodies compare as exact JSON, so this is
all-or-nothing rather than something to land incrementally.

**Parser error detail.** `?or=()` and some JSON-path failures answer generically
where PostgREST names the offending character and what would have been
accepted. The logic tree matches exactly; the select parser does not yet.

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
