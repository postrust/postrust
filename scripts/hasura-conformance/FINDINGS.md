# What the Hasura harness found

Measured against `hasura/graphql-engine:v2.50.1`, 464 replayable cases in 61
groups. Three things the commit history does not keep: faults found in the
harness itself, divergences kept on purpose, and the gaps still open.

## Where the number is

| | status | same outcome | same data | full body |
|---|---|---|---|---|
| all (464) | 97.8% | 56.2% | **49.1%** | 19.0% |
| reads (269) | 99.3% | 58.7% | 48.7% | 23.8% |
| writes (195) | 95.9% | 52.8% | 49.7% | 12.3% |

The same-data figure over successive fixes: 34.1 → 41.8 (duplicate-field
panic) → 43.8 (enum arguments read) → 44.2 (relationship predicates, aggregate
result decoding) → 44.6 (scalar naming) → 48.3 (update operators, written-value
casts, root type names) → 49.1 (embed arguments, computed columns).

The third column is the one that matters to a client: the same query came back
with the same rows.

**142 of the 464 cases cannot pass and are counted anyway.** They are the
permission cases — Hasura answers `access-denied` from a rule that lives in
metadata, and there is no metadata here. Excluding them the figure is
165/322 = 51.2%. The gap between the two is small, which is the useful thing
to know: the headline is not being held down by the divergence chosen on
purpose. It is held down by the gaps listed further below.

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

1. **Names Hasura takes from metadata rather than from the schema.** This is
   one item, not the several it looked like, and closing computed relationships
   is what made that clear: the function is `fetch_articles_plain` and the
   field Hasura exposes is `get_articles`, because `add_computed_field` named
   it. Same for a computed field's name (`upper_name` for a function called
   `automatic_comment_in_db_upper_name`, which is why
   `graphql_introspection/descriptions` sits at 0/5 while every description it
   reports is correct), for custom root field names, and for relationship names
   below. Reflection cannot recover a name nobody wrote down. Roughly 25 cases
   across four groups turn on it, and it is one decision about configuration
   rather than four pieces of work.

2. **`sum_float` and one `json` type are not resolved** in
   `graphql_query/computed_fields`, and neither reproduces against a
   hand-written schema with the same shape — a computed column returning
   `json`, and a scalar computed column, both work. Something narrower about
   the fixture is responsible and it has not been found yet.

3. **Enum tables, and why they may stay open.** `colors_enum` in the corpus is
   not a PostgreSQL enum: it is an ordinary table of allowed values, marked
   `set_table_is_enum` in metadata, from which Hasura generates a GraphQL enum
   and types every referencing column as it. 17 cases turn on it. There is no
   reflection-based way to tell that table from any other — a text primary key
   with a comment column is just a table — so this one needs configuration,
   which is the model this project declined. Worth deciding explicitly rather
   than leaving as a to-do.

4. **`on_conflict`.** Upserts are not expressible. Needs unique-constraint
   names, which the schema cache does not collect: `Table` carries `pk_cols`
   and nothing else, though `schema_cache/queries.rs` already reads `conname`
   for foreign keys.

5. **Nested aggregates.** `author { articles_aggregate { aggregate { count } } }`
   — the aggregate exists only as a root field, not as a relationship field.

6. **The comment on a computed field's function** is not carried by the schema
   cache, so a computed field's description is always null.

7. **Ordering across a relationship** is not offered at the root, deliberately:
   a field the schema advertises and the resolver refuses is worse than one
   that was never there. An embedded list now takes its own `order_by`, so what
   is left is ordering a parent by a child's column.

8. **Relationship predicates through a junction** are refused rather than
   resolved: reaching the child means going through a third table, which is
   more than a pair of columns to correlate on. The computed-relationship half
   of this is now resolved, by argument rather than by columns.

## Fixed since the first run

- **Relationship predicates** now resolve as a correlated `EXISTS`, in both
  directions, with each nesting level aliased so a relationship followed twice
  cannot correlate against itself. Column references are qualified for the same
  reason.
- **Aggregates** returned nothing because the result was cast to text and
  `execute_query_on` decodes column zero as JSON, so every row was dropped by
  the `.ok()` that guards it. The selection walk was never the problem, which
  instrumenting it settled in a minute.
- **Scalars carry their PostgreSQL names** — `jsonb`, `numeric`, `timestamptz`,
  `geometry` — because `query ($x: jsonb!)` names a type. 56 cases were failing
  on the spelling alone. Types this server knows nothing about keep their own
  name rather than collapsing to String, which is what makes a PostGIS column
  usable at all.
- **Updates take all seven operators**, and written values are cast to their
  column's type: a bound parameter arrives as text, and PostgreSQL will not
  coerce text to `jsonb`, to an array, or to a user-defined type on assignment.
- **An embedded list takes `where`, `order_by`, `limit` and `offset`**, applied
  inside the child's own subselect so the limit bounds rows per parent and the
  ordering happens before it. `EmbedPlan::embed_expression` already took all
  four for the REST surface; the GraphQL side had no way to say any of them.
- **Computed columns are fields**, at the root and inside an embed.
- **Computed relationships resolve** — a function returning `setof` a table,
  named by its function rather than by the table it returns, filterable and
  pageable like any other embed, and usable in a predicate. They had been in
  the relationship list all along, named after the target table, colliding with
  the foreign key of the same name and dropped as a duplicate; the error even
  suggested the field they had been mistaken for.

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
