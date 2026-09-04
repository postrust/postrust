# Hasura conformance

Postrust speaks Hasura's GraphQL dialect deliberately: the field names, the
`where` grammar, `order_by`, the aggregate shapes, `on_conflict`, and the error
envelope mean the same thing in both. How closely is not a matter of opinion
here — it is measured, against `hasura/graphql-engine` itself, and this page
says what the measurement covers and where the two servers disagree on purpose.

## Where it stands

Run 78, over 468 replayed cases in 59 groups, against
`hasura/graphql-engine:v2.50.1`:

| Compared on | All (468) | Reads (271) | Writes (197) |
|---|---|---|---|
| HTTP status | 100.0% | 100.0% | 100.0% |
| …and the same outcome | 99.6% | 99.6% | 99.5% |
| …and the same data | **97.4%** | 95.9% | **99.5%** |
| Whole body, wording included | 96.6% | 95.6% | 98.0% |

Of the 456 cases the third level counts, 325 agree about data and 131 agree
because both servers refuse. Twelve cases diverge there — two on the outcome,
ten on the data — and at the strictest level sixteen do.

Not one case differs on status.

Measured on a binary the harness built itself, with the reference replayed
rather than reused; `run-meta.json` beside the results records both.

The run history — including the runs whose numbers are not publishable, and
why — is in [`FINDINGS.md`](../scripts/hasura-conformance/FINDINGS.md).

## How it is measured

Hasura's test corpus is YAML: each case spells out a GraphQL document, the
variables and headers it is sent with, and the response the suite expects. The
harness extracts those cases and replays each one over HTTP against **both**
stock `hasura/graphql-engine` and Postrust, each on an identically loaded
fixture database, and diffs the live responses.

The reference implementation is the oracle. No expectation written in the
corpus is ever interpreted, which means a mistake in the extractor shows up as
a case both servers answer the same way rather than as a false failure. This
matters more here than it would elsewhere: two of the divergences below are the
corpus disagreeing with the engine it was written for.

What Hasura keeps in metadata rather than in the database — relationship names,
and what each role may do — is converted from each group's own metadata and
given to the candidate. So this measures a *configured* server, which is what
migrating actually involves, rather than a bare one.

```bash
scripts/hasura-conformance/conformance.sh
node scripts/gen-hasura-conformance.mjs scripts/hasura-conformance/.work/diff.json
```

See [`scripts/hasura-conformance/README.md`](../scripts/hasura-conformance/README.md)
for the mechanics.

### What produced a number is recorded with it

The harness builds its own candidate rather than requiring one to exist,
because which features it was built with is part of what is being measured and
cannot be read off the file. `admin-ui` is what mounts the GraphQL routes at
all: without it every case answers 404, which reads as total failure rather
than as the misconfiguration it is.

It then writes `run-meta.json` beside the results — the Hasura version, the
features built, whether the reference was replayed or a recording reused, the
commit, and the date. The generator reads that file rather than its own
arguments, and refuses to publish a run that cannot account for itself. A run
measured with the wrong binary produces a number that looks exactly like a good
one, and a command-line flag would only stamp it as correct.

## What "conformance" counts

Agreement is reported at four strictness levels, because one systemic gap would
otherwise sink every case and hide the hundreds that differ in nothing else.

1. **HTTP status.**
2. **…and the same outcome** — both answered with data, or both with errors.
3. **…and the same data.**
4. **The whole body**, errors included, wording and all.

The third is the one that matters, and the one that needs explaining. Two
servers agree about data when they return the same rows — and they also agree
when both refuse. Counting only the first would score a case where Hasura
itself raises an error as a failure of this server to match it, which is
backwards.

Bodies are compared as parsed JSON.

## Where the two disagree on purpose

**An unsecured server trusts no header.** Hasura with no admin secret
configured treats every caller as an administrator, which also lets any caller
name its own role and its own identity. Here, with no secret configured,
`x-hasura-*` headers carry no weight at all and session variables come only
from a verified token. A policy reading a value the caller chose is not a
policy, and the failure is silent — the query succeeds, against the wrong rows.
It costs nothing measured: every case in the corpus that names a role sends the
secret beside it, because that is what Hasura's own suite does.

**Relationship names are derived, not configured.** Every relationship in the
corpus is named by a metadata command a human wrote; here they come from
foreign keys. Where a fixture chose something other than the convention, the
field is not there under that name. This is structural to reflecting a database
instead of configuring one.

**Two databases naming different constraints.** `pg_dump` restores constraints
in a different order than the fixture created them, so PostgreSQL reports a
different constraint name for the same violation. Two cases, proven against one
PostgreSQL — neither server is answering wrongly.

**Introspection belongs to an administrator.** The corpus expects `__schema` to
be refused for a role that has introspection disabled. A v2.50.1 reference does
not refuse it: asked with the admin secret beside `X-Hasura-Role`, it answers
from that role's own restricted schema, so the permissions apply and the
introspection rule does not. This server was built to the corpus's text first
and measured against the reference second, which cost two cases and is the
right way round to find that.

## Known gaps

**Introspection, inside async-graphql.** Not reachable from here:
`SchemaBuilder::finish` builds the registry itself and `Schema` keeps it
private, so the directives it installs, the types it adds, and the order it
lists them in cannot be changed from outside the library. It publishes five
directives where Hasura publishes three, adds `ID` and `__DirectiveLocation`
which Hasura has neither of, and answers `__TypeKind`'s members in
specification order where Hasura sorts them. Every large introspection case
needs at least one of those.

**This one stays even if the library opens up**, and it is worth being plain
about why, because ten of the sixteen remaining cases are here and closing
them would take the measured figure from 96.6% to about 99.4%.

The response could be rewritten on the way out — there is one place every
GraphQL answer passes through. But matching Hasura means *removing*
`deprecated`, `specifiedBy` and `oneOf` from the directive list, and `ID` and
`__DirectiveLocation` from the type list, when this server genuinely has them.
Introspection is the contract a client or a code generator reads to learn what
a server supports; a schema that omits `@deprecated` while `@deprecated` keeps
working is a server misreporting itself. It runs the other way too: Hasura
publishes a `cached` directive, and advertising that would claim a
response-caching feature this server does not implement.

Sorting types, arguments and enum members is harmless, and so is wording a
description differently — order and prose carry no capability claim. Those are
worth doing. The suppression is not, and since every large case needs at least
one suppressed item, the cases stay open. A conformance percentage is a
measurement, not a target to be reached by making the server describe itself
falsely.

**`_stream` subscriptions.** The cursor-based half of Hasura's subscription
surface. The live queries beside it are done; this is a second shape with its
own cursor types.

**An enum value beginning with `null`, `true` or `false`.** One line of
async-graphql's grammar excludes any name that *starts with* one of the three
literals, where the specification excludes only the tokens themselves. So an
upsert naming a constraint called `nullPrefixTestTable_pkey` is answered with a
parse error.

A fix is written and tested against the rule directly —
[`patches/async-graphql-parser-enum-value.patch`](../patches/async-graphql-parser-enum-value.patch)
— and belongs upstream rather than here. The local alternative is rewriting the
document text and restoring every name in the parsed tree, machinery that could
corrupt a document that was valid to begin with. This is a real defect beyond
conformance: any schema with a constraint or enum member named `nullable_x` or
`truestore` hits it.

**Generated descriptions** are this server's wording rather than Hasura's, and
arguments come back in declaration order where Hasura sorts them. Nothing
breaks on it — a description is documentation — but it is one reason no large
introspection case reaches agreement.

**Actions and Apollo federation** are subsystems rather than gaps.

## Related

- [Configuration](./configuration.md#hasura-authentication) — the admin secret,
  roles, and session variables
- [API Reference](./api-reference.md) — the GraphQL surface itself
- [PostgREST Conformance](./postgrest-conformance.md) — the same treatment for
  the REST surface
