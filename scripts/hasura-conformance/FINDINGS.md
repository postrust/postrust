# What the Hasura harness found

Measured against `hasura/graphql-engine:v2.50.1`, 464 replayable cases in 61
groups. Three things the commit history does not keep: faults found in the
harness itself, divergences kept on purpose, and the gaps still open.

## Where the number is

| | status | same outcome | same data | full body |
|---|---|---|---|---|
| all (464) | 97.8% | 50.9% | **43.8%** | 13.6% |
| reads (269) | 99.3% | 55.0% | 45.0% | 20.1% |
| writes (195) | 95.9% | 45.1% | 42.1% | 4.6% |

The third column is the one that matters to a client: the same query came back
with the same rows.

**142 of the 464 cases cannot pass and are counted anyway.** They are the
permission cases — Hasura answers `access-denied` from a rule that lives in
metadata, and there is no metadata here. Excluding them the figure is
144/322 = 44.7%, which is barely different, so the headline is not being held
down by them. It is held down by the gaps listed further below.

Reported as measured, at four levels, because a single systemic gap otherwise
sinks everything behind it. Two of those gaps were found exactly that way.

## Faults in the harness, found by their symptoms

A differential harness measures two things at once and only one of them is the
server. Every fault in the instrument either invents work or hides it.

- **Extensions did not survive the sweep between groups.** The sweep drops and
  recreates `public`, which takes any extension living there with it, so every
  group after the first failed with `type "geography" does not exist` — an
  error about a fixture, produced entirely by the harness. Extensions are now
  installed into a schema of their own and the search path carries the
  unqualified type names the fixtures use.

- **The corpus straddles two APIs and the harness knew about one.** `run_sql`
  and the unprefixed commands are `/v1/query`; the source-aware spellings the
  newer fixtures use — `pg_track_table`, `pg_add_computed_field` — are
  `/v1/metadata`, and posting one to the other's endpoint is rejected outright.
  Commands are now sent individually rather than as the `bulk` they arrive in,
  because a file mixing the two has no single endpoint that would accept it.

- **Reading only mapping-shaped case files loses a third of the corpus.** A
  file beginning with `-` is an ordered sequence of cases — an insert followed
  by the select that reads it back. Mapping-shaped files alone give 292 cases;
  both shapes give 468.

## Real faults, found the same way

Both of these were invisible to the unit tests and immediately obvious to the
harness, and both had the same signature: no response at all rather than a
wrong one.

- **A column and a relationship sharing a name killed the process.**
  `create table pizza (crust text references crust)` is an ordinary way to
  write a foreign key. async-graphql panics on a duplicate field rather than
  returning an error, so the server died at startup, before it listened. 106
  of 464 cases recorded no response. Fixed in the object types and then again
  in the generated boolean expressions, which had the same clash one type
  later — along with a second shape of it, two foreign keys to one table
  deriving two relationships of one name.

  Status agreement went from 76.5% to 97.8% on that fix alone. Nothing about
  the query layer changed.

- **Every enum argument arrived as null.** `accessor_to_json` tried boolean,
  integer, float, string, list and object and fell through to null, so
  `order_by: {name: asc}` reached the builder as an empty direction and was
  refused with `"" is not a sort direction`. The direction enum, the ordering
  input and the SQL were each correct on their own; only a real request
  carried a value through all three.

## Kept on purpose

- **Permissions.** Hasura's rules live in metadata and are compiled into every
  query. Here they live in the database as roles and row level security. The
  142 `access-denied` cases are that difference, and they are counted rather
  than excluded — the size of the gap is one of the things worth knowing. What
  transfers is the caller's identity: the `x-hasura-*` claims of a verified
  token become `SET LOCAL` settings, so a policy reading
  `current_setting('hasura.user_id')` sees what the Hasura permission would
  have seen.

- **Session variables come from the token, never from headers.** Hasura reads
  `X-Hasura-User-Id` off the wire because it has an admin secret to gate that
  on. Without one, honouring the header would let any caller name its own
  identity.

- **Relationship names are Hasura's to choose.** Every relationship in the
  corpus is named by a metadata command a human wrote; here they are derived
  from foreign keys. Where the fixture chose something other than the
  convention, the field is simply not there under that name. This is
  structural to reflecting instead of configuring.

## Open, ordered by consequence

1. **Relationship predicates are advertised and not resolved.**
   `where: {articles: {title: {_eq: "x"}}}` needs an `EXISTS` against the
   related table, and the builder has no table context. It refuses rather than
   ignoring — a filter that quietly matches everything is worse than one that
   fails — but the field is in the schema, so a client that type-checks still
   fails at runtime. This is the largest single gap in `boolexp`.

2. **Aggregates return nothing.** The types generate and the field resolves,
   but the selection walk in `resolve_aggregate` finds no children, so
   `aggregate` comes back null and `nodes` empty. Every case in
   `graphql_query/aggregations` fails on it. The corpus cases alias heavily
   (`id_sum: id`, `articles: nodes`), which is the first thing to rule out.

3. **Custom schemas.** `graphql_mutation/custom_schema` and
   `graphql_query/custom_schema` are at zero. Hasura renames root fields
   through `set_table_customization`; here a table outside the default schema
   is prefixed with its schema name instead.

4. **Enum tables.** `set_table_is_enum` turns a table into a GraphQL enum.
   There is no equivalent, and no reflection-based way to know a table was
   meant as one.

5. **Introspection descriptions.** Table and column comments are already in the
   schema cache, so this is closer than its 0/5 suggests.

6. **`_inc`, and the jsonb update operators** (`_append`, `_prepend`,
   `_delete_key`, `_delete_elem`, `_delete_at_path`). `graphql_mutation/update/jsonb`
   is at 14%.

7. **Ordering across a relationship** is not offered at all, deliberately: a
   field the schema advertises and the resolver refuses is worse than one that
   was never there. Item 1 is the same problem with the opposite decision
   already made, and the two should end up consistent.

## Not measured

The GraphQL surface still builds its own SQL by string concatenation, parallel
to the `ReadPlan` → `ReadPlanTree` → `QueryBuilder` path that serves REST.
Items 1 and 7 above are both blocked on that: the plan tree already expresses
relationship predicates and correlated ordering, and the string builder would
have to grow its own version of both. Lowering GraphQL onto the plan was
planned before this work and deferred during it, on the grounds that the
schema shape is what the corpus measures and a refactor with nothing to show
would have delayed every number here. That trade should not be extended
further.
