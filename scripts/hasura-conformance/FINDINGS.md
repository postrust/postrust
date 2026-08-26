# What the Hasura harness found

Measured against `hasura/graphql-engine:v2.50.1`, 468 replayable cases in 61
groups. Three things the commit history does not keep: faults found in the
harness itself, divergences kept on purpose, and the gaps still open.

## Where the number is

| | status | same outcome | same data | full body |
|---|---|---|---|---|
| all (468) | 99.4% | 90.8% | **85.9%** | 60.7% |

Of the 402 cases the third column counts, **278 agree about data and 124 agree
only because both servers answered with errors.** A further 4 are cases where
Hasura refused and this server answered -- down from 26, and from 115 before
the permission layer existed.

**278 is the figure that tracks the work.** It counts the cases where the same
query came back with the same rows, which is the only thing a client can feel.

Over the six phases of the permission work, measured at each: 321/464 (69.2%,
214 real) before the harness was fixed -> 365/468 (78.0%, 260) once the
reference was actually authenticated and this server could read a session
variable -> 402/468 (**85.9%, 278**) with the permission layer whole.

**These numbers are not comparable with the ones this file carried before the
admin-secret fix, and the reason is in the instrument.** Every run up to 37 had
reused a reference in which 142 cases answered `access-denied` because the
request was never authenticated at all -- the harness attached the admin secret
only to a case that named no headers, where Hasura's own suite attaches it
whenever one is configured. Those 142 were counted as a difference between two
permission models. They were an unauthenticated request. The old headline said
69.2% of 464 with a "permission exclusion" of 94.4%; both halves of that were
measuring the instrument. The guard against it recurring is below.

The corpus is 468 cases rather than 464 because a batched request -- a body
that is a JSON array of operations -- is a case the extractor now reads.

Where the remaining 66 divergences are:

| | count |
|---|---|
| both answered with data, and the data differs | 23 |
| Hasura answered, this server refused | 39 |
| Hasura refused, this server answered | 4 |

The second row is where the work is, and it is no longer one thing. Reading
the run: a role that may write a table it cannot read has no schema here and
so no mutation (6 permissions in the corpus do this); `_exists`, which is a
predicate only a permission can write, is not compiled; and a handful are
faults of their own -- a preset binding that produces an invalid UTF-8
sequence, an upsert that reports a check failure where Hasura reports that it
wrote nothing. Each is named in the open list.

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

- **A reference recorded by a different harness was reused for eleven runs.**
  `run.py` attaches the admin secret to every case, because a case that names
  `X-Hasura-Role: user` and nothing else is an *admin-authenticated* caller
  asking to be treated as another role -- without the secret the engine never
  reaches its permission layer. That was fixed; what was not fixed is that
  `HASURA_CONFORMANCE_REUSE_REF=1` went on reusing a `ref.json` recorded before
  it, in which all 142 role-naming cases had answered `access-denied`. The
  harness reported a permission-model difference that no longer existed, and
  the number it printed was built on it for eleven runs.

  The instrument now stamps the reference with a hash of what produced it --
  `run.py`, `extract.py`, and the extracted cases -- and replays rather than
  reuses when the stamp does not match. This is the same class as the two
  faults below and the worst of the three: those produced no answer, which is
  visible, and this produced a confident wrong one.

- **Reading only mapping-shaped case files loses a third of the corpus.** A
  file beginning with `-` is an ordered sequence of cases — an insert followed
  by the select that reads it back. Mapping-shaped files alone give 292 cases;
  both shapes give 468.

## Real faults, found the same way

- **A comparison naming a type no column has broke the schema, three commits
  running.** The raster comparisons name `geometry`; a cast from a geometry
  names `geography`. A type the schema mentions and never registers makes the
  whole schema unbuildable, so a table with a raster column and no geometry
  column took its whole group down -- and the fix, applied by hand, broke again
  one operator later, that time across 95 of 464 cases.

  Patched twice, then fixed: `build_inputs` reports the scalars it actually
  named, and those are what get registered. There is no second list to keep in
  step, which is the only version of this that cannot drift. Both times the
  symptom was cases with no response rather than a wrong one, and both times
  the server said what had failed and kept serving -- which is why it was five
  404s and then 95, rather than a dead process.

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

- **A permission is narrowed, never widened.** Hasura's rules and this
  server's are the same rules now -- read from the same metadata, compiled into
  the same queries -- with one asymmetry kept on purpose. Reading is what makes
  a table exist here, so a role that Hasura lets insert into a table it cannot
  read loses the insert rather than gaining a readable column, and the same for
  a column. Six permissions in the corpus write a table they cannot read and 27
  write a column they cannot read; every one of them fails closed. Reproducing
  Hasura needs two column sets per table where a schema cache has one, and
  until that exists the direction to be wrong in is the one that withholds.

- **An unsecured server trusts no header.** Hasura with no admin secret
  configured treats every caller as an administrator, which also lets any
  caller name its own role and its own identity. Here, with no secret
  configured, `x-hasura-*` headers carry no weight at all and session variables
  come only from a verified token. A policy reading a value the caller chose is
  not a policy, and the failure is silent -- the query succeeds, against the
  wrong rows. It costs nothing measured: every case in the corpus that names a
  role sends the secret beside it, because that is what Hasura's own suite
  does.

- **Relationship names are Hasura's to choose.** Every relationship in the
  corpus is named by a metadata command a human wrote; here they are derived
  from foreign keys. Where the fixture chose something other than the
  convention, the field is simply not there under that name. This is
  structural to reflecting instead of configuring.

## Open, ordered by consequence

1. **A role that may write a table it cannot read.** Six permissions in the
   corpus, and the largest single item left. Hasura exposes `insert_x` to a
   role with an insert permission and no select permission; here reading is
   what makes a table exist, so such a role gets no schema for it and no
   mutation. Closing it means two column sets per table -- one for the type,
   one for the input -- where a schema cache has one, and a mutation response
   that carries `affected_rows` without a `returning` to hang rows on. The
   direction it is wrong in is the one that withholds, which is why it is an
   open item rather than a fault.

2. **`_exists`.** A predicate only a permission can write:
   `{"_exists": {"_table": {...}, "_where": {...}}}` asks whether a row exists
   in *another* table, unrelated to this one by any foreign key. Two cases in
   `graphql_mutation/delete/permissions`. The machinery is there -- it is the
   correlated subselect a relationship predicate already builds -- but the
   correlation is written by hand in the `_where` rather than derived from a
   key, which is a different shape to read.

3. **A preset that produces an invalid UTF-8 sequence.** `resident_user` in the
   insert permissions group answers `invalid byte sequence for encoding
   "UTF8"`, which is this server binding a preset value as something it is not.
   One case, and a real fault rather than a gap.

4. **An upsert that reports a check failure where Hasura reports it wrote
   nothing.** `resident_5_modifies_resident_6_upsert` expects `affected_rows:
   0` from an `ON CONFLICT` that changes nothing; here the check runs on a row
   the statement did not write. One case, and the same shape of fault.

5. **The path in an error is `$` where Hasura writes the selection.** Hasura
   answers `$.selectionSet.insert_computer.args.objects` for a refused write;
   this answers `$`, because the refusal is raised where the rows come back
   rather than where the argument was read. Costs nothing at the data level and
   is why those cases do not reach full-body agreement.

6. **`_stream` subscriptions.** The cursor-based half of Hasura's subscription
   surface: `article_stream(cursor: {initial_value: {id: 0}}, batch_size: 10)`
   sends rows *after* a cursor rather than the whole answer, which is what a
   client tailing an append-only table wants. The live queries beside it are
   done; this is a second shape with its own cursor types, and nothing in the
   corpus exercises it -- it shows up in introspection only.

7. **What is left of the enum tables.** They work: a marked table's rows are a
   generated enum, referencing columns are typed as it, and no relationship
   points at one. What remains is the metadata API around them —
   `v1/set_table_is_enum` is four cases of turning the flag on and off through
   `/v1/query`, which is the contract this server does not offer.

8. **A manual relationship** -- one Hasura maps column by column rather than by
   a foreign key -- has no constraint to key a name by, so
   `PGRST_GRAPHQL_NAMES` cannot carry its name and the converter says so rather
   than guessing. In the corpus it is also a *second* name for a foreign key
   that already has one, which reflection can only produce once. Two cases, one
   of them the only remaining insert this server refuses and Hasura performs.

9. **Which relationships exist is metadata's to say.** Hasura exposes the
   relationships its metadata declares; this server exposes one per foreign
   key. Where a fixture tracks a table without naming all of its keys, the
   extra fields are here and not there. No query breaks on a field it does not
   ask for, so this shows up only where a schema is compared field by field --
   `graphql_introspection/nullable_object_relationship` is the case. Closing it
   would mean letting the names document say which relationships exist, not
   just what they are called, which is a different kind of directive.

10. **A function taking a table's row, tracked as a root field.** Hasura lets a
   client write `fetch_articles(args: {search: "Art", author_row: "(1, 'Roger',
   'Chris')"})` -- the row as a literal. Here such a function is a computed
   field and nothing else, on the grounds that a row type is not something a
   client can reasonably send. Offering it would also mean registering the
   table's composite type as a scalar under the table's own name, which is a
   name the object type already has. One case, and the position is deliberate.

11. **A function returning one row, as a mutation.** `add_to_score_by_user_id`
   returns `"user"` rather than `SETOF "user"`; Hasura exposes it as a root
   field yielding one row. Here only a set-returning function becomes a root
   field.

12. **Every generated description is this server's wording, not Hasura's.**
    `article_bool_exp` is described here as "Filter rows of article. Fields are
    combined with AND unless _or says otherwise." and there as "Boolean
    expression to filter rows from the table \"article\"...". Nothing breaks
    on it -- a description is documentation -- but it is why no large
    introspection case reaches full-body agreement, and adopting Hasura's
    strings verbatim is the only way it would. The same goes for the order
    arguments come back in: Hasura sorts them, this server lists them as
    declared.

13. **Introspection shape, inside async-graphql.** It publishes five directives
    where Hasura publishes three, registers `ID` and `__DirectiveLocation`
    where Hasura has neither, and answers `__TypeKind`'s members in
    specification order rather than alphabetically. Six cases, and none of them
    is reachable without forking the library.

14. **An enum value beginning with `null`, `true` or `false`.** The parser
    mis-lexes `nullPrefixTestTable_pkey` as the literal `null` followed by
    something it cannot read, so an upsert naming that constraint is answered
    with a parse error. This is async-graphql's lexer, not this server. One
    case.

15. **Actions and Apollo federation** are subsystems rather than gaps:
    `actions/*` describes handlers Hasura calls out to over HTTP, and
    `apollo_federation` describes the `_service`/`_entities` surface a
    federated gateway composes. Five cases between them.

## Fixed since the first run

- **Permissions, in full.** The largest single piece of work this file records,
  and it began as a harness fault rather than a feature: the 142 cases this
  file used to call "the permission gap" were unauthenticated requests, and
  fixing that is what made the gap measurable at all.

  What went in, in order. `PGRST_HASURA_ADMIN_SECRET` and the reading that a
  role header is something an *authenticated* caller sends, so `x-hasura-*`
  headers become session variables and a Hasura role is held apart from the
  database role the transaction runs as. Permissions in the same document as
  the names, converted from the same metadata by the same script, because a
  permission is the same kind of thing as a name -- something a person wrote
  down that no schema remembers -- with the one asymmetry that a missing name
  means "derive it" and a missing permission means "no access". A schema per
  role, built from a schema cache first reduced to what that role can see, so
  the builders need to know nothing: a table this role cannot read is one the
  code generating root fields cannot see either. The row filter, which is the
  same language a `where` argument is written in and so is rewritten into one
  and handed to the compiler that already exists -- applied at the root read,
  the by-key read, the aggregate, inside an embed, and inside the `EXISTS` a
  relationship predicate becomes, because a row you may not read is one you may
  not learn the existence of by filtering on it. And the writes: a `filter` on
  update and delete, a `check` evaluated in the write's own `RETURNING` against
  the row as written, presets applied over the request rather than under it,
  and `backend_only` as a second schema rather than a check, since the flag
  decides whether a field exists.

  Two faults it found on its first run, both invisible to the unit tests: a
  nested write naming an `_obj_rel_insert_input` for a table the role could not
  insert into, which async-graphql refuses to build a schema around -- and the
  fact that one such role took every other role's schema down with it, and the
  unrestricted one besides. A role whose schema cannot be built is now refused
  alone.

  `graphql_query/permissions` 0/34 -> 21/34, `graphql_mutation/insert/
  permissions` 0/39 -> 27/39, and the run-wide figure 260 -> 278 real
  agreements.


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
- **A reserved word in a returned type.** `pg_catalog.format_type` quotes what
  needs quoting, so a function returning rows of `user` reports `SETOF "user"`
  -- which matches no table called `user`. Not one of the corpus's six
  functions was exposed and a hand-written stand-in was, which is what let it
  hide for a round.
- **Functions that answer with rows of a table.** `search_articles(args: {term:
  "rust"}, where: …, order_by: …, limit: 5)` -- the rows are that table's rows,
  so everything built for reading them applies unchanged, and the function's
  own arguments go under `args` where a parameter called `limit` cannot shadow
  the one that pages the result. VOLATILE puts it on the mutation root and
  STABLE on the query root, which is what PostgreSQL says about it rather than
  what its name suggests.
- **A comparison against a null.** `where: {id: {_eq: $id}}` with a null
  answered `operator does not exist: integer = text`: a bound parameter is
  untyped and PostgreSQL infers what it is from the operator, and a null gives
  it nothing to infer from. Only a null is cast -- casting every operand broke
  every numeric comparison, because a cast makes PostgreSQL infer the parameter
  feeding it as text.
- **Shapes are GeoJSON in both directions.** A geometry column was written by
  casting -- and `'{"type":"Point",…}'::geometry` is not a cast PostgreSQL has
  -- and read back as WKB hex, which no client parses. Both ends speak GeoJSON
  now, with the CRS member Hasura includes, applied everywhere a row becomes
  JSON rather than only where the tests looked.
- **Tree comparisons.** `_ancestor`, `_descendant`, `_matches` and their `_any`
  forms on an `ltree` column, each naming what its operand casts to -- `?` is
  "any of these labels" for a path and "has this key" for a jsonb, and
  PostgreSQL tells them apart by the operand's type alone. `boolexp/ltree` went
  5/12 to 12/12.
- **A list is bound as an array.** `_has_keys_any` never worked: one parameter
  carrying `["a","b"]` is a JSON array, not an array literal, and PostgreSQL
  said so. Elements are bound one at a time into an `ARRAY[...]`.
- **Typed mutation inputs.** `<table>_insert_input`, `_set_input` and
  `_inc_input`, with `obj_rel`/`arr_rel` inputs carrying nested writes and
  their own `on_conflict` -- so a nested row is upserted exactly as a top-level
  one is. Every field optional, because which columns the database insists on
  is the database's answer.
- **`_cast`.** A geometry column compared as a geography, and back -- the
  question a sphere answers and a plane does not. It changes what is being
  compared rather than comparing, so it recurses into the comparison builder
  with a different column expression and a different type.
- **PostGIS comparisons.** The spatial relations on the columns they apply to,
  with GeoJSON operands parsed by `ST_GeomFromGeoJSON` and cast to the column's
  own type -- `ST_DWithin` takes a geometry or a geography and picking the
  wrong one is not an overload PostGIS has. `boolexp/raster` went 1/5 to 5/5.
- **Ordering by a related row's column, or by an aggregate of a row's
  children.** `order_by: {author: {name: asc}}` and
  `order_by: {articles_aggregate: {count: desc}}`, both as correlated
  subselects, both nesting. `graphql_query/order_by` went 3/14 to 9/14 and
  `order_by_nulls` to 7/7.
- **Enum tables.** A table marked as a set of allowed values becomes a GraphQL
  enum built from its rows, with a `comment` column as each member's
  description, and every column with a foreign key to it is typed as that enum.
  The values are read once at startup, since rows are not schema. This was the
  item filed as "needs a decision"; the decision was that the flag belongs
  beside the names, in the one document a migration already converts.
- **Upserts.** `on_conflict: {constraint: author_name_key, update_columns:
  [bio]}` writes the row or updates the one already there, and an empty
  `update_columns` is `DO NOTHING` -- `affected_rows: 0` and an empty
  `returning`. The blocker was never the SQL: the schema cache carried
  `pk_cols` and nothing else, so there was no way to name which uniqueness was
  being resolved against. Unique constraints are loaded now, each a member of a
  generated enum with the columns it covers as its description.
- **A row's children can be counted without fetching them**:
  `articles_aggregate { aggregate { count } nodes { … } }` is the same embed
  with an aggregate select list, one row per parent, both halves in one pass.
  Worth noting what this did *not* move: the cases in the corpus that ask for a
  nested aggregate are permission cases, or ask for a tracked function as a
  root field beside it, so the field existing was necessary and not sufficient.
- **A parent and its children are written in one mutation**, in either
  direction, in one transaction, with `affected_rows` counting every row
  written rather than every row returned.
- **A mutation's `returning` carries relationships and computed fields**, by
  reading the written rows again through the projection a query uses. Before
  this, a non-null list field asked for beside the columns had no value and the
  mutation answered as though it had failed -- after writing its rows
  correctly, which is worse than failing.
- **Names can be given** where the schema cannot carry them, through
  `PGRST_GRAPHQL_NAMES`: table base names, relationship names (keyed by
  constraint, or by function for a computed relationship), and computed field
  names. Only names -- no permissions, no tracking.
- **Computed relationships resolve** — a function returning `setof` a table,
  named by its function rather than by the table it returns, filterable and
  pageable like any other embed, and usable in a predicate. They had been in
  the relationship list all along, named after the target table, colliding with
  the foreign key of the same name and dropped as a duplicate; the error even
  suggested the field they had been mistaken for.
- **An aggregate's `nodes` are rows like any other.**
  `article_aggregate { nodes { author { name } } }` answered null for the
  author and `author_aggregate { nodes { articles_aggregate { … } } }` answered
  an empty list, because `nodes` was read as a list of column names and nothing
  else. It gets the projection the plain root field gets: computed fields, and
  relationships as correlated subselects. A nested aggregate also takes the
  four arguments its list takes -- `where` on an aggregate decides which rows
  are *counted*, so the rows are read as a subquery and the aggregate reads
  from that. `graphql_query/aggregations` 0/5 → 5/5, including the deeply
  nested case, which alternates the two all the way down.
- **The jsonb comparisons a client actually writes.** `_contains: "latest"`
  bound the bare text `latest`, which is not JSON; a containment operand is a
  whole document and `"latest"` is one. `_cast: {String: {_like: …}}`, because
  a document has no `LIKE` and the text it renders as does. `_jsonb_path_exists`
  and `_jsonb_path_match`, which query the document rather than compare against
  it. `graphql_query/boolexp/jsonb` 5/10 → 10/10.
- **A variable declared and never used.** The specification calls that document
  invalid and async-graphql refused it; Hasura executes it, and a client whose
  filter was edited years ago has been getting answers ever since. The unused
  declarations are dropped before validation sees them -- one specific refusal,
  not validation in general: an unknown field and an *undefined* variable are
  still refused. Seven cases across the mutation groups.
- **Four ways an ordering was read wrong.** A direction given as null is no
  direction, in all three places an ordering is read. A computed field is a
  column as far as ordering is concerned. `distinct_on` with an ordering that
  does not begin with it is refused rather than answered -- prepending the
  distinct columns produced an answer in the opposite order to the one asked
  for. And a `citext` column compares as one: a parameter arrives as text and
  `citext = text` resolves by taking the citext *down* to text, so `_eq:
  "clarke"` answered case-sensitively against a case-insensitive column.
  `graphql_query/order_by` 10/14 → 13/14.
- **A row that points at nothing has no related row.** A to-one relationship
  with a null key answered `{"name": null}` -- an object every field of which
  is null -- because the null was handed to the field resolvers instead of
  being the answer.
- **A shape sent as a variable arrives as a shape.** Every coordinate in
  `insert/geojson` came back as zero, and it had nothing to do with geometry:
  this crate reads `serde_json` with `arbitrary_precision` so a `numeric` keeps
  its digits, and the cost is that a *fractional* number in a variable
  deserializes into async-graphql's value type as a one-key object holding the
  text. Integers were unaffected, which is why it survived every case that
  filters by id. Beside it, a statement that writes a table names that table's
  row in `RETURNING`, and a bare table name there is read as a *column* first
  -- `to_jsonb("area")` on a table `area` with a geography column `area` was
  the shape, not the row. And GeoJSON is checked before PostGIS sees it, which
  will otherwise build a "Polygon" from three points that do not meet.
  `graphql_mutation/insert/geojson` 5/13 → 13/13.
- **What a write can be told, and what it can answer with.** A delete's
  `returning` carries relationships, by putting the delete in a CTE and reading
  the projection from it while the rows it points at are still there. An
  update's and a delete's `where` can follow a relationship. An update that
  changes nothing changes no rows rather than being refused. `_delete_at_path`
  binds its path one key at a time, since one parameter carrying
  `["name","last"]` is a JSON array and PostgreSQL reads it as an array
  literal. A written value names the type it is compared against, not only
  where it is null -- `uuid = text` is not an operator at all. And
  `_st_d_within` on a geography takes `use_spheroid`.
- **A mutation is one transaction.** Several root fields in one mutation each
  opened and committed a transaction of their own, so a mutation whose second
  root field violated a constraint left the first one's rows behind and
  reported failure. They share one now, opened by the first write and settled
  by whoever answers the request -- committed when the response carries no
  errors, rolled back otherwise.
- **Default descriptions are Hasura's.** `fetch data from the table: "x"`,
  `columns and relationships of "x"`, `A computed field, executes function
  "f"`. A schema with no comments in it documents itself the way the one it
  replaces did.

- **The names Hasura keeps that a schema cannot**, in full: per-root names for
  the cases a base name cannot reach (`select_by_pk: Article` beside `select:
  Articles`), and column renaming. The second reaches everywhere -- a renamed
  column appears in the projection, in `where`, in `order_by`, in
  `distinct_on`, in the key arguments, in both mutation inputs, in
  `on_conflict.update_columns` and in every embed and aggregate -- and each of
  those is now a place that asks what column a field name means. The rename
  goes in the projection *over* the subquery, never inside it, so the row a
  computed field is passed is still the table's own composite; `RETURNING`
  keeps the table's names for the same reason, since a nested insert reads the
  parent's key out of it. `graphql_query/custom_schema` 0/2 → 2/2 and
  `graphql_mutation/custom_schema` 0/4 → 4/4, and nothing regressed: 8 gained,
  0 lost.
- **The descriptions Hasura keeps in metadata** -- for the table, each column,
  each root field and each computed field. An empty comment is not silence: it
  means the field has no description, which is how the corpus hides one the
  database has. Beside it, a converter bug this found: a custom root field may
  be written as a bare name or as `{name, comment}`, and only the first shape
  was read.
- **`update_x_many`**, which applies several updates each with its own filter,
  in one transaction. And root fields introspect in name order, which is
  Hasura's order and does not depend on which table this server read first.
  `_schema` -- a field returning the string "Postrust GraphQL Schema", on every
  query root and in every generated client's types -- is gone; the empty-schema
  placeholder it was holding up is now called `no_queries_available`, which is
  what Hasura calls its own.

- **A function that asks who is asking.** `add_to_score(hasura_session json,
  search text, increment integer)` is Hasura's convention for a function that
  needs the caller's identity, and the session document is the server's to
  supply -- a caller that could write it could name any identity it liked. It
  is dropped from the exposed arguments and filled from the verified token,
  under the names a function body indexes into. The same for a computed field,
  which may now take the session beside the row, at either position -- which
  means the call is written in named notation and the row argument's name is
  carried for it. The REST surface skips those: it has no session document to
  give them.
- **Filtering on a computed field.** `where: {sum_float_offset: {_gt: 10}}` is
  a question a client can write, because the field is on the type, and it
  answered `column float_test.sum_float_offset does not exist`. It reads as the
  call that produces it now, the same way it does in a projection.
- **An array column takes an array**, in both directions. The insert and `_set`
  inputs were typed by the leaf scalar, so a `text[]` column said `String` and
  a client offering `[String]` was told it had the wrong type; and once past
  that, `["a","b"]` is JSON where PostgreSQL wanted `{a,b}`.

- **A variable used where its type does not fit is refused.** The
  specification's "All Variable Usages Are Allowed" rule, made here because
  async-graphql carries exactly that rule and it does not fire -- verified
  against 7.0.17 with a static schema, with a dynamic one, and on a built-in
  directive, none of which report. It was the only gap that was a missing
  *refusal*: a client whose variable was mistyped got rows instead of an
  error, and found out from the rows. Written with the two exemptions that
  make the rule usable -- a nullable variable fits a non-null place when
  either it or the place has a default -- and with a default *of null* not
  counting, which is the difference between the two cases the corpus has.
  Alongside it, a non-null variable given an explicit null. Every message is
  Hasura's, word for word.

- **A computed relationship that takes arguments.** `fetch_articles(search
  text, author_row author)` returns rows of another table, so it is an embed
  rather than a field -- and it takes something from the caller, which an embed
  had nowhere to put. It takes `args` now, the way a function root field does;
  the row is passed by name, since it is no longer the only argument; and the
  session is filled by the server. The aggregate over one takes the same
  arguments, because counting the rows a function answers with means calling
  it.
- **Which row a nested insert writes first** follows from which side holds the
  key, not from how many rows there are. `author_detail.id` referencing
  `author.id` is one row in either direction, and the detail was written first
  and failed on a null key. `Cardinality::O2O` already recorded which side is
  the parent; the code was reading cardinality where it needed direction.
- **A table read at the root is aliased.** A whole-row reference -- what a
  computed field is passed -- can only be written as a bare name, and
  `"public"."author"` is not one: PostgreSQL reads it as a column of a table
  called `public`. An alias no column can share is a name that works in both
  positions.
- **A computed column that takes arguments.** The symmetric half of the
  relationship above. `locations_distance("from" json, locations_row
  locations)` is a field of `locations` taking `args`, and the projection that
  writes its call now carries the query's parameter list, which is what it was
  missing.
- **`path` into a document column.** `c32_json(path: "objs[0]['你好']")` reads
  one part of a `json` or `jsonb` column, in the spelling Hasura accepts --
  optional `$`, keys bare or after a dot, indices and quoted keys in brackets,
  and a dot before a bracket meaning nothing. Answered where the value is
  rather than in SQL, because the same column may be asked for under several
  aliases and one projection cannot carry both.
- **A key over more than one column can be named.** Hasura names a
  relationship by its columns, not by its constraint, and the converter could
  key only a single-column one -- so `article_multi.author` and
  `author_multi.articles` kept their derived names and every query naming them
  failed. Both sides now write the columns, sorted, as one key.
- **The coercions Hasura performs on a written value.** `offset: "1"` and
  `{c1_smallint: "32767", c20_boolean: "true"}` are a string where a number or
  a boolean is declared, and Hasura reads them: a column's value goes through
  PostgreSQL's own reading of a literal, which takes either spelling. The walk
  is type-directed -- a `String` column keeps its digits -- and `limit` is left
  strict, because the corpus refuses `limit: "3"` in the same breath that it
  answers the offset.
- **A null written into a comparison** is refused: `where: {id: {_eq: null}}`
  reads as `id = NULL`, which is never true, so a client that wrote it meant
  something the query cannot mean. A variable standing for a null counts; a
  variable that was not given does not, because an absent variable makes the
  comparison itself absent.
- **`distinct_on` inside an embed**, under the same rule the root field
  applies: the ordering has to begin with the distinct columns, or the row that
  survives depends on the plan.
- **The document operators are typed, and only where there is a document.**
  `_append`, `_prepend`, `_delete_key`, `_delete_elem` and `_delete_at_path`
  took an untyped `JSON`; each is an input object over the table's `jsonb`
  columns now, and a table with none is not offered them at all. `_in` and
  `_nin` take a list of non-null items, since a null is not a value a column
  could equal. A bulk delete's `where` is non-null, which is what the resolver
  was refusing at execution. A boolean column is out of `min` and `max`, which
  PostgreSQL has no aggregate for.
- **A predicate over an aggregate of a related set.** `where:
  {articles_aggregate: {count: {predicate: {_gt: 2}}}}` -- the authors with
  more than two articles, which no filter on one article can say. `count`
  takes the same `columns` and `distinct` the field does and a `filter`
  narrowing what is counted; `bool_and` and `bool_or` fold one boolean column.
  A scalar subselect rather than an `EXISTS`, because over no related rows at
  all `count` is zero and a fold is null -- which is how "no articles" is
  written, and an `EXISTS` cannot say it.
- **`count(columns:, distinct:)` was declared and ignored**, so every count was
  `count(*)` and a client asking for the distinct values of a column got the
  number of rows. It is written now, and each occurrence is answered under the
  name it was asked for: `count` beside `authors: count(columns: [author_id],
  distinct: true)` is two different numbers in one selection, and they were one.
  The same went for two selections of one other aggregate -- `sum { id }`
  beside `totals: sum { views }` built the object twice under one key, and
  builds it once over the union of their columns now.
- **Where metadata says a function goes.** `track_function` with
  `configuration: {exposed_as: query}` puts a VOLATILE function on the query
  root; reflection places it by volatility, which is the only thing the
  catalogue records. The names document grew a `functions` section for it, and
  the converter reads it out of the same metadata. The document's flat shape is
  still read, so nothing written against the old one has to change.
- **A batched request.** A body that is a JSON array of operations is answered
  with an array of responses, in the order they were sent -- the shape Apollo's
  batching client posts. Each entry is its own operation with its own
  transaction: one failing answers for itself and the array still comes back
  whole. Three cases in `graphql_query/basic` use it, and the harness could not
  see them: `extract.py` skipped any case whose payload was not a mapping, and
  `report.py` read every body as one response. Both read an array now, so the
  corpus is 467 cases rather than 464.
- **A subscription is a live query.** It was a change stream: one row per
  `LISTEN`/`NOTIFY` payload, no arguments, no initial answer. It is now what a
  Hasura client expects -- `subscription_root` mirrors the query root field for
  field, `orders(where:, order_by:, distinct_on:, limit:, offset:)` yields
  `[orders!]!`, and `orders_by_pk` and `orders_aggregate` sit beside it -- and
  every message is the whole current answer.

  The mechanism is this server's own rather than Hasura's. Hasura polls every
  subscriber on an interval and pays for it whether or not anything happened;
  here the trigger on the table wakes the query, which is instant and costs
  nothing while nothing is written, and one `LISTEN` connection per instance
  serves any number of subscribers. A slow refresh runs beside it --
  `PGRST_SUBSCRIPTION_REFRESH`, 30 seconds -- because a trigger cannot see
  everything a query can: a view has none, an embedded row may live in a table
  that carries none, and `where: {expires_at: {_lt: "now()"}}` changes with no
  write at all. A wake is not a message: the answer is compared with the one
  last sent, so a write outside the subscription sends nothing.

  Row-level security now applies to a subscription, which it could not before:
  each re-read runs as the subscriber's own role, where a notification carried
  whatever the trigger published.

- **`/v1alpha1/graphql`** is served. Ten cases in the corpus post there -- it is
  the address Hasura answered on before `/v1`, and a client old enough to have
  been pointed at it is exactly the one that cannot be repointed.

- **`money` arithmetic.** `_inc: {price: -1.1}` failed with `operator does not
  exist: money + double precision`: the operand was bound uncast, and a JSON
  number arrives as a double. `_inc` now casts to the column's type the way
  `_set` does, and `money` is reached through `numeric`, which is the only cast
  PostgreSQL has to it -- while an amount written as text keeps going straight
  to `money`, so `"$12,344.57"` is still read as one.

- **An aggregate over a function.** Every table root has an `_aggregate` and now
  every function root does too: `search_tracks_aggregate(args: {…})` counts what
  `search_tracks(args: {…})` would have answered with, which means calling the
  function and taking the same arguments.

## Not measured

The GraphQL surface still builds its own SQL by string concatenation, parallel
to the `ReadPlan` → `ReadPlanTree` → `QueryBuilder` path that serves REST. It
has now grown its own relationship predicates, its own correlated ordering, its
own aggregates and its own nested writes -- each of which the plan tree either
expresses already or would express better, and each of which is a second
implementation to keep in step with the first. Column renaming (open item 1) is
where that bill comes due: translating a name at every boundary is exactly the
sort of thing a plan has one place for and a string builder has eleven.

Lowering GraphQL onto the plan was planned before this work and deferred during
it, on the grounds that the schema shape is what the corpus measures and a
refactor with nothing to show would have delayed every number here. That trade
has now been extended about as far as it goes.
