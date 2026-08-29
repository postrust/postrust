# What the Hasura harness found

Measured against `hasura/graphql-engine:v2.50.1`, 468 replayable cases in 61
groups. Three things the commit history does not keep: faults found in the
harness itself, divergences kept on purpose, and the gaps still open.

## Where the number is

| | status | same outcome | same data | full body |
|---|---|---|---|---|
| all (468) | 100.0% | 99.6% | **97.4%** | 96.6% |
| reads (271) | 100.0% | 99.6% | 95.9% | 95.6% |
| writes (197) | 100.0% | 99.5% | **99.5%** | 98.0% |

Of the 456 cases the third column counts, **325 agree about data and 131 agree
only because both servers answered with errors.** No case is left where Hasura
refused and this server answered -- down from 115 before the permission layer
existed.

The status column reaching 100% is not a rounding: it was a rule read out of
the corpus wrongly and is described below.

**325 is the figure that tracks the work.** It counts the cases where the same
query came back with the same rows, which is the only thing a client can feel.

Over the permission work, measured at each step: 321/464 (69.2%, 214 real)
before the harness was fixed -> 365/468 (78.0%, 260) once the reference was
actually authenticated and this server could read a session variable ->
402/468 (85.9%, 278) with the permission layer whole -> 403/468 (86.1%, 274)
-> 413/468 (88.2%, 285) -> 414/468 (88.5%, 286) -> 419/468 (89.5%, 290)
once a role could write what it cannot read -> 426/468 (91.0%, 297) with
`_exists` -> 429/468 (91.7%, 300) once a preset reached an update ->
432/468 (92.3%, 303) once a ceiling stopped answering `count` -> 433/468
(92.5%, 304) with the last of the permission model -> 456/468 (**97.4%, 325**)
at run 78, after main was merged in and the shape questions below were closed.

The dip in the fourth of those is worth keeping. Three roles lost their entire
API to one input type that was named and never registered, and the run-to-run
comparison is the only thing that showed it: the headline moved by one case
while eleven real agreements turned into refusals underneath it. That fault is
described below.

Writes are the stronger half now, at 98.0% against 95.6% for reads. They were
the weaker half six runs earlier. Four of the 197 write cases are left, and
every one of them is a shape question rather than a permission one.

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

Where the remaining 35 divergences are:

| | count |
|---|---|
| Hasura answered, this server refused | 16 |
| both answered with data, and the data differs | 17 |
| Hasura refused, this server answered | 2 |
| status differs | 0 |

Each is named in the open list rather than attributed to one missing
subsystem.

The fourth column moved on its own for seven runs after that: what an error
says, and where it says it happened. 311 bodies matched entirely before those
runs and 452 do now. **4 cases are left in the gap between the third column
and the fourth, down from 101 -- and none of the four is a wording question.**
Two are the constraint-order artefact described under the harness, one is a
deliberate divergence, and one is metadata this harness cannot follow.

**16 cases diverge at all, and 10 of them are one thing: introspection, inside
async-graphql.** What is left is described below, item by item, and most of it
is not this server's to change.

Writes are at 97.5% on the full body and 99.0% on data, against 95.6% and
95.9% for reads. Two write cases are left in the whole corpus that do not
agree about data.

The five runs before those moved nineteen cases between them and moved nothing
back.
The first of the five took three attempts to get there, and the two discarded
ones are the record worth keeping: the first read +3 on the headline and hid
seven regressions underneath, four of them in a group with no permissions at
all -- the signature of a change that was supposed to touch only roles. The
second fixed three of the four and left the fourth, because the fix went to
the branch that runs for a role and the fault was in the branch that runs for
everyone. Both were found by the per-case comparison against the previous run,
not by the percentages, which moved the right way each time.

**The permission model is finished as far as this corpus can measure it.**
Nothing left in the open list is a permission question: what remains is schema
shape, library internals, and metadata APIs this server does not offer. Two of
the permission cases that agree do so by both servers refusing, and they refuse
for the same reason now rather than by accident -- `Unknown argument "columns"
on field "count"` against Hasura's `'count' has no argument named 'columns'`.
The wording and the error path are their own open items, below.

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

- **`track_table` takes its argument as a bare name, and the converter read
  it as a document.** `- type: track_table` / `args: employees` is how the
  corpus tracks a table with nothing to configure, and `args` is then a string
  rather than a map. The converter skipped anything whose `args` was not a
  map, so those tables were missing from the names document entirely -- and
  since a missing table means "reflect", the difference was invisible until
  the document started saying which relationships a table has.

- **`pg_dump` restores constraints in a different order than the fixture
  created them, and PostgreSQL names whichever it checks first.** Two cases,
  and neither is a server difference. `transaction_revert_http` inserts a row
  that violates both the primary key and a unique index; the reference answers
  `author_pkey` and this server answers `author_name_key`.

  The fixture is `create table author(id serial primary key, name text
  unique)`, so the reference's database has `author_pkey` first. `pg_dump`
  emits constraints alphabetically, so the candidate's restored database has
  `author_name_key` first -- and PostgreSQL reports the constraint whose index
  it reaches first. Run against one PostgreSQL:

  ```text
  create table fixture(id serial primary key, name text unique);
  -- duplicate key value violates unique constraint "fixture_pkey"

  create table restored(id integer not null, name text);
  alter table restored add constraint restored_name_key unique (name);
  alter table restored add constraint restored_pkey primary key (id);
  -- duplicate key value violates unique constraint "restored_name_key"
  ```

  Same server, same statement, two answers, decided by creation order alone.
  Nothing in the dump records the original order, so the only fix would be to
  build the candidate's database from the fixture SQL rather than from a dump
  of it -- which is the one thing this harness deliberately does not do, for
  the reason at the top of `conformance.sh`. Recorded rather than closed.

- **Metadata commands the Python suite applies between cases are not applied
  here, and four cases agree by both servers lacking them.** A group's setup is
  a `setup.yaml` the harness restores; but Hasura's own suite also calls
  `hge_ctx.v1metadataq_f(...)` from inside a test method, granting a function
  permission before one case and revoking it after. `TestGraphQLMutationFunctions`
  does exactly that for `add_to_score` and `add_to_score_by_user_id`.

  The reference never sees those commands, so it answers `no mutations exist`
  where the corpus file itself expects rows. Since the candidate is configured
  from the same metadata, it now answers the same thing, and the four cases
  count as agreements. They are agreements about a configuration neither server
  was given -- which is a true statement about two servers configured alike,
  and not a true statement about the case. Two more in the same group are
  genuine: `function_without_function_permission` and `address_permission_error`
  expect `no mutations exist` in the corpus and get it from both.

  Closing it means reading the per-test commands out of the Python and
  attaching them to the case they precede, which is the first thing in this
  harness that would parse the suite rather than its data.

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

- **An argument may not name a type nothing registered, and three times it
  did.** The same shape each time, and each time it cost a whole schema rather
  than the one field it was about, because async-graphql refuses to build a
  schema with a dangling reference.

  A nested write named `author_obj_rel_insert_input` for a table the role could
  not insert into. `_inc_input` and the document operator inputs were
  registered from the update permission's columns while the arguments naming
  them were still decided from the object type's fields -- two answers that had
  always agreed by coincidence and stopped agreeing the moment a write input
  began following its own permission. And a select permission granting no
  columns produced a type with no fields and an enum with no members, neither
  of which is legal; `order_by.rs` already skipped such a table with a comment
  predicting exactly this, written back when a table with no columns was
  impossible.

  There is one definition of "which columns a write may name" now, called from
  both the place that registers and the place that declares, so those two
  cannot drift again. The first of the three also showed that one bad role took
  every other role's schema down with it, and the unrestricted one besides: a
  role whose schema cannot be built is now logged and refused alone.

  Found by the run-to-run comparison rather than by any single number: the
  headline moved by one case while eleven real agreements became refusals.

- **A status rule read out of the corpus's own expectations, and read
  wrongly.** Hasura answers 400 for a write refused by a permission's `check`
  -- three cases in the corpus say so, and this server copied them. It answers
  200 for the same refusal five other times, with an identical body and an
  identical code, and the difference is not in the error at all: the three that
  answer 400 were sent to `/v1alpha1/graphql` and the five that answer 200 to
  `/v1/graphql`. Every non-200 in the whole corpus is on the legacy endpoint.

  Reading a declared status as a fact about the error rather than about the
  address cost three cases, all of them otherwise agreeing. The status column
  is 100% with the endpoint passed in, and the lesson is the one the
  admin-secret fault taught: a corpus expectation describes a request, and half
  of a request is where it was sent.

- **A type named twice over, registered for every table that could be
  written.** The set of tables needing a `<t>_mutation_response` was collected
  from the mutation fields' return types without trimming the suffix, so it
  held both `article` and `article_mutation_response` -- and built a
  `article_mutation_response_mutation_response` beside the real one. It was
  harmless only because `insert_one` and `update_by_pk` return the bare type
  and kept the right name in the set. The moment a table had bulk writes and
  neither of those -- which is exactly what a role that cannot read it has --
  the wrong name was registered and the right one was missing, and the schema
  would not build. One junk type per mutable table had been in every
  introspection answer until then.

- **An upsert ignored the update permission.** `ON CONFLICT DO UPDATE`
  overwrites a row that is already there, which is an update, and the update
  permission's row filter was not part of it: a role could reach a row it may
  not update by inserting over it. Two corpus cases show the shape from the
  outside -- one expects `affected_rows: 0` because the filter excludes the
  row, the other expects `missing session variable` because compiling the
  filter is where a session variable the caller does not carry is noticed. Both
  were passing by mutual refusal while the roles in question had no schema at
  all, which is how a hole stays quiet.

- **An enum with no members, reached through an argument.** A role whose update
  permission grants no columns leaves `<t>_update_column` empty, which is not a
  legal enum -- and dropping it drops `<t>_on_conflict` with it, which an
  insert still names. Hasura's answer is a member that names no column,
  `_PLACEHOLDER`, refused with `erroneous column name` where it is used. The
  corpus tests both halves in one file, which is how the shape was found rather
  than guessed.

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

- **An unsecured server trusts no header.** Hasura with no admin secret
  configured treats every caller as an administrator, which also lets any
  caller name its own role and its own identity. Here, with no secret
  configured, `x-hasura-*` headers carry no weight at all and session variables
  come only from a verified token. A policy reading a value the caller chose is
  not a policy, and the failure is silent -- the query succeeds, against the
  wrong rows. It costs nothing measured: every case in the corpus that names a
  role sends the secret beside it, because that is what Hasura's own suite
  does.

- **Introspection is withheld from a role and not from an administrator.**
  `set_graphql_schema_introspection_options` names roles that may not read the
  schema as data, and the corpus expects `__schema` to be refused for one. A
  v2.50.1 reference does not refuse it: given the same fixtures and asked with
  the admin secret beside `X-Hasura-Role`, it answers -- from that role's own
  restricted schema, so the permissions apply and the introspection rule does
  not. The corpus's expectation was written for a role reached without the
  secret, and `check_query` sends the secret.

  This server was built to the corpus's text first and measured against the
  reference second, which cost two cases and is the right way round to find it.
  It now matches what the reference does: reading the schema is an
  administrator's to do, whatever role it then names. The group went 1/3 to
  2/3 on the fix, and the case still outstanding is not about introspection
  being disabled -- it lists the schema's types, and the two schemas differ in
  shape for the reason further down this list.

- **Relationship names are Hasura's to choose.** Every relationship in the
  corpus is named by a metadata command a human wrote; here they are derived
  from foreign keys. Where the fixture chose something other than the
  convention, the field is simply not there under that name. This is
  structural to reflecting instead of configuring.

## Open, ordered by consequence

The 16 cases that diverge, and why each does. Ten of them are one item.

1. **Introspection, inside async-graphql: 10 cases.** Not reachable from
   outside the library, and now traced rather than assumed.
   `Registry::add_system_types` installs five directives -- `skip`, `include`,
   `deprecated`, `specifiedBy`, `oneOf` -- where Hasura publishes three:
   `include`, `skip` and its own `cached`. `create_introspection_types`
   installs `ID` and `__DirectiveLocation`, which Hasura has neither of, and
   answers `__TypeKind`'s members in specification order where Hasura sorts
   them. `SchemaBuilder::finish` builds that registry itself and `Schema`
   keeps it private, so there is no hook: `Registry::directives` is a public
   field on a value this server never holds.

   Every large introspection case needs at least one of those, so none of
   them can be closed by anything here. Three more ingredients would be
   needed even then, and they are open items of their own: the `_stream`
   cursor types (item 2), a function returning one row as a root field (item
   6), and Hasura's own wording for every generated description (item 7).

2. **`_stream` subscriptions.** The cursor-based half of Hasura's subscription
   surface: `article_stream(cursor: {initial_value: {id: 0}}, batch_size: 10)`
   sends rows *after* a cursor rather than the whole answer, which is what a
   client tailing an append-only table wants. The live queries beside it are
   done; this is a second shape with its own cursor types. No case exercises
   it directly -- it shows up as five missing types inside the introspection
   cases above.

3. **An enum value beginning with `null`, `true` or `false`.** One line of
   async-graphql's grammar:

   ```text
   enum_value = ${ !(boolean | null) ~ name }
   ```

   The negative lookahead rejects any name that *starts with* one of the three
   literals, where the specification excludes only the three tokens
   themselves. So `nullPrefixTestTable_pkey` is not lexed as an enum value and
   an upsert naming that constraint is answered with a parse error. Identical
   in 7.2.1, so an upgrade does not carry the fix. One case, and closing it
   means either a patched fork of the parser or rewriting the document text
   and restoring every name in the parsed tree -- machinery that could corrupt
   a document that was valid to begin with.

4. **Four cases that are not this server's to close.** Two are the `pg_dump`
   constraint-order artefact under the harness above: two databases naming
   different constraints for the same violation, proven there with the SQL.
   One is a deliberate divergence -- a nested write into a view that selects
   from more than one table is refused here for having no insert input, where
   Hasura reaches the database and wraps PostgreSQL's complaint in an
   `internal` object this server does not produce. The last is a computed
   field added and dropped through the metadata API inside a single case,
   which a static names document cannot follow.

5. **A function taking a table's row, tracked as a root field.** Hasura lets a
   client write `fetch_articles(args: {search: "Art", author_row: "(1, 'Roger',
   'Chris')"})` -- the row as a literal. Here such a function is a computed
   field and nothing else, on the grounds that a row type is not something a
   client can reasonably send. Offering it would also mean registering the
   table's composite type as a scalar under the table's own name, which is a
   name the object type already has. One case, and the position is deliberate.

6. **A function returning one row, as a mutation.** `add_to_score_by_user_id`
   returns `"user"` rather than `SETOF "user"`; Hasura exposes it as a root
   field yielding one row. Here only a set-returning function becomes a root
   field. No case of its own left -- it is one of the ingredients item 1
   needs.

7. **Every generated description is this server's wording, not Hasura's.**
   `article_bool_exp` is described here as "Filter rows of article. Fields are
   combined with AND unless _or says otherwise." and there as "Boolean
   expression to filter rows from the table \"article\"...". Nothing breaks on
   it -- a description is documentation -- but it is one of the reasons no
   large introspection case reaches agreement, and adopting Hasura's strings
   verbatim is the only way it would. The same goes for the order arguments
   come back in: Hasura sorts them, this server lists them as declared.

8. **Actions and Apollo federation** are subsystems rather than gaps:
   `actions/*` describes handlers Hasura calls out to over HTTP, and
   `apollo_federation` describes the `_service`/`_entities` surface a
   federated gateway composes. Three of the introspection cases in item 1 are
   `actions/*`, and would need the subsystem before the library.

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

  `graphql_query/permissions` 0/34 -> 26/34, `graphql_mutation/insert/
  permissions` 0/39 -> 33/39, `graphql_mutation/delete/permissions` 0/9 -> 7/9,
  and the run-wide figure 260 -> 285 real agreements.

- **Reading and writing are two column sets, not one.** The last piece of the
  permission work, and the one the schema cache had no room for: Hasura grants
  them separately, so a role may set a column it may not see -- ten permissions
  in the corpus do -- and may write a table it cannot read at all, which eight
  more do. Both used to fail closed, which is what this file recorded as the
  largest item left.

  The union goes in the cache and the split is made once, where the object
  type's fields are built. `fields` is what may be read and `writable_fields`
  is what may be written; everything a read is made of -- the type, its
  boolean expression, its ordering, its column enum, its aggregates -- comes
  from the first and narrows with it, and the write inputs come from the second
  and are narrowed again by the permission naming them. That keeps the property
  the whole permission layer rests on: one place to be right, rather than a
  question asked at every place a name is emitted.

  A table with no readable field is the limit of it, and has no GraphQL type at
  all -- a type with no fields is not a legal one. The query roots, `insert_one`,
  the `_by_pk` writes, `returning`, a relationship pointing at it and a function
  returning it all go with the type; what is left is the bulk write, answering
  with `affected_rows`. `insert_account`, `insert_leads` and `insert_author`
  are that shape, and so is a select permission that grants only a computed
  field.

  Four faults came out with it, three older than the change, all described
  above: a mutation response type named twice over, a status rule that belonged
  to the endpoint, an upsert that ignored the update permission, and an enum
  with no members reached through an argument. 286 -> 290 real agreements, and
  the status column to 100%.


- **Which relationships a table has is metadata's to say, and a relationship
  is a join rather than a name.** Two open items that turned out to be one.
  The names document could rename a relationship and not describe one, so a
  **manual** relationship -- Hasura maps it column to column, with no
  constraint to be keyed by -- could not be carried at all; and a rename map
  cannot say what is *absent*, so reflection went on offering one relationship
  per foreign key where Hasura offers the ones its metadata declares.

  `declared_relationships` answers both. An entry is reached either by a key
  reflection already found or by a column mapping no key describes, and where
  the list is present it is what the table has. Absent still means "reflect",
  which is what a document saying nothing about a table has always meant --
  so an empty list is how a converted document says "Hasura tracks this table
  and declares no relationship for it", which is a different statement and
  the one four of these cases turned on.

  Three things the measurement found, none of which reasoning had:

  * A computed relationship is declared by `add_computed_field`, not by
    `create_object_relationship`, so it is not in the list and is not what
    the list is exhaustive about. Six cases lost `author.get_articles` before
    it was exempted.
  * A permission written through a declared relationship has to resolve it
    too: once the declaration carries the name, that is the only place the
    name is written down.
  * An enum table's columns were retyped by walking the relationships the
    schema *exposes*. Declaring a table's relationships away untyped its enum
    columns with them -- eleven cases -- and the two are unrelated questions,
    since a relationship to an enum table is never exposed at all. The
    retyping reads the catalogue's foreign keys now.

- **A mapped join says which row a nested insert writes first.** A key says
  which side holds it -- `article.author_id` references `author`, so the
  author is written first and its key fills the article's column. A column
  mapping says only that two columns are equal, and `author.id =
  author_detail.id` is equal in both directions, so Hasura writes
  `insertion_order` down and this reads it. `after_parent` is spelled here as
  a one-to-one whose parent is this row, which is a shape the writer already
  understood; nothing in the write path needed teaching.

- **A generated enum is a type, not the table's data.** A role granted nothing
  on `colors` still sees `colors_enum` and still has a `favorite_color` typed
  by it; Hasura publishes both and answers `where: {favorite_color: {_eq:
  red}}` for that role. Here the enum went with the table, and a valid query
  came back `Invalid value for argument "where.favorite_color._eq", expected
  type "String"`. The type's name is derived rather than looked up -- a table
  the role cannot read has no entry to look one up in -- and the key that says
  which column is typed by which enum survives the reduction, which costs
  nothing because no field comes of it either way.

- **Seven one-offs, and the last of the error text.** No two shared a cause,
  which is what made them the end of that list.

  A role granted `book_name` and not `id` was still offered `books_by_pk(id:
  ..., book_name: ...)`, because the key columns were read off the table and
  the one that was missing fell back to `text` -- so the field existed and
  typed its argument `String`. A `_by_pk` addresses a row by its key, and half
  a key is not one: Hasura does not publish the field, and neither does this
  now, for the query root and both write roots alike.

  A computed field is granted under **the name the client sees**, and the map
  is keyed by the function. Comparing them directly -- `student_total_marks`
  against `total_marks` -- withheld every computed field a schema had renamed,
  from a permission that had granted it.

  An operator's operand is a value, not a boolean expression. `_st_d_within`
  takes `{distance, from}` and this server read those keys as *columns*: it
  renamed them, wrapped their values in `_eq`, and handed PostGIS something
  that was no longer GeoJSON -- `invalid GeoJson representation`, about a
  shape it had taken apart itself. Session variables inside an operand are
  still resolved, which is the one thing a permission's operand does need.

  `x-hasura-use-backend-only-permissions: random` is not false. The accepted
  texts are a closed set Hasura names in the refusal itself, and `1` is not
  among them; reading anything else as false would send a backend-only write
  down the path meant for everyone else, on a header the client thought it
  had set.

  And a request carrying `extensions.persistedQuery` is an Apollo handshake
  neither server implements. Hasura says `PersistedQueryNotSupported`; this
  server ran whatever document was beside the hash, which is the answer that
  looks like the handshake worked -- after which a client sends hash-only
  requests into silence.

- **A permission's filter is the server's predicate, not the caller's.** Eight
  cases, and the largest thing wrong with reads. A permission written over a
  relationship -- `{"Artist": {"id": "X-Hasura-Artist-Id"}}` on `Track` --
  refused itself: role `Artist` has no permission on `Artist` at all, so that
  relationship is in no schema it is served, and the compiler read the key as
  a column and answered `unsupported comparison "id" on "Artist"`.

  Consulting a table is not exposing it. `_exists` already worked this way and
  said so where it is built; this is the same rule for the relationship a
  permission follows. The permission half of a `where` is now marked on its
  way to the compiler, and compiled with no caller: relationships resolve
  against the unreduced catalogue rather than against the role's schema, and
  the far side's own filter is not applied on top. A client cannot reach a
  hidden table this way -- a key that is not a field of the boolean expression
  is refused before any of this runs, and the only writer of such a key is a
  permission.

  Two more pieces went in with it. **A column compared against another
  column**: `_ceq`, `_cne`, `_cgt`, `_cgte`, `_clt`, `_clte`, written either
  as a bare name -- `{"bid_price": {"$cgt": "price"}}`, two columns of one
  auction -- or as `["$", "name"]`, a column of the table the permission is
  written on. No client can write these, because no client can name the row a
  comparison would be against.

  And `$` names the table the *permission* is written on, not the table the
  query started at. Reading `dml_rbp_tasks` through a project's `where:
  {tasks: {...}}` compared the task's grant against a column of
  `dml_rbp_projects` and answered with rows the role may not see -- the one
  fault in this work that was wrong rather than merely refused, and the reason
  the integration test that pins it checks a row *count* rather than an error.

- **A role granted no mutation has no mutation root.** Hasura answers `no
  mutations exist` at `$`, which is not a field that is missing: the
  `mutation_root` type is absent from that role's schema altogether. Six cases,
  and two separate readings behind them.

  A function with side effects is not granted by permission to read the table
  it returns. Hasura infers a *query* function's permission from the select
  permission on its return table -- which is a rule this server already had,
  since a function returning a table the role cannot read has nowhere to put
  the rows -- and infers nothing at all for a function exposed as a mutation.
  `pg_create_function_permission` is the only thing that grants one. The
  evidence is in the reference rather than in the documentation: `anonymous`
  holds `select` on `user`, `add_to_score` returns `SETOF user`, and the
  reference still has no mutation root for that role.

  And an unrecognised role is, to Hasura, a role that was granted nothing
  rather than an error. This server refused it -- `role "merchant" is not
  defined in the permissions` -- which is the more useful answer for an
  operator and the wrong one for a client. Such a role is now answered from a
  schema with nothing in it, built once at startup rather than one per name a
  caller invents. Both answers refuse everything; only one of them is a
  refusal Hasura's clients know how to read.

- **Three singletons, one shape between them.** Each was this server letting
  something further down answer a question the request had already settled.

  `update_article(_set: {id: 1, author_id: 1} _inc: {id: 2, author_id: 2})`
  asks for two columns to be two things each, in an order the request does not
  say. PostgreSQL refuses the statement -- a column may appear once in a `SET`
  list -- and by then names one column. Hasura names every column written
  twice and reports it against the arguments rather than against either
  operator, since neither is the wrong one: `Column found in multiple
  operators: ['id', 'author_id'].`

  `update_author(where: null)` says nothing about which rows, and `where` is
  required. async-graphql called it `Invalid value for argument "where",
  expected type "author_bool_exp"`, which names the argument and not what was
  wrong with it. Hasura names the type it wanted and the thing it got. Only a
  *required* input object is refused: a nullable one written as `null` is the
  same as leaving it out, which is not this server's business to turn into an
  error. A variable carrying the null keeps its own older answer, `null value
  found for non-nullable type: "author_bool_exp!"`, which is the fault in the
  declaration rather than in the argument -- Hasura distinguishes those too.

  `where: {get_articles: {...}}` is a computed relationship whose function
  also wants a search term: `fetch_articles(search text, author_row author)`.
  A selection writes that as `get_articles(args: {search: "..."})` and a
  filter has nowhere to write it at all, so this server composed
  `fetch_articles(author)` and answered with PostgreSQL's complaint about a
  call it had made up. It is not in the boolean expression now, which is
  Hasura's answer -- `field 'get_articles' not found in type:
  'author_bool_exp'` -- and it stays selectable, where the argument has a
  place. A relationship whose extra arguments are all *optional* stays
  filterable: the call with the row alone is one the function accepts.

- **A request is refused once, however many faults it has.** In 468 replayed
  cases Hasura never answered with a second error. It validates a request the
  way a parser reads one -- it stops where it cannot go on -- so a query
  naming two fields that do not exist is refused for the first of them, and an
  `on_conflict` carrying an unknown key beside a missing required one is
  refused for the key.

  This server reported everything the walk found, which is the more useful
  answer and the wrong one here: a client written against Hasura reads
  `errors[0]`, and the rest of the list is text it will never show. Five cases
  differed in nothing else -- their first error already matched Hasura's
  whole body.

  The walk still runs to the end and the truncation happens where the refusal
  is answered. Stopping the walk itself would put the stopping condition in
  every arm of it, for no gain: the walk is over a document that has already
  parsed.

- **A column PostgreSQL generates always is in no write input.** `author.id`
  is `GENERATED ALWAYS AS IDENTITY`, which means naming it in an insert is an
  error rather than merely unnecessary -- PostgreSQL answers `cannot insert a
  non-DEFAULT value into column "id"`. By then the statement has been built
  and the request forgotten, so the path could only point at the row, not at
  the field: `$.selectionSet.insert_author.args.objects` where Hasura says
  `$.selectionSet.insert_author.args.objects[0].id`.

  The column leaves every write shape instead. It is not a permission -- no
  role may name it -- so the flag lives on the column in the schema cache,
  beside `is_pk`, and is read in the one place the write inputs are built
  from. `_insert_input`, `_set_input`, `_inc_input` and the `_update_column`
  enum all lose it together, the row type keeps it, and the walk over the
  request then produces Hasura's own answer without being told anything:
  `field 'id' not found in type: 'author_insert_input'`, pointed at the field
  it was written on and at whatever depth a nested write put it.

  `GENERATED ALWAYS AS (...) STORED` is the same flag for the same reason. A
  `BY DEFAULT` identity is deliberately not: it has a default, and a value
  given for it is taken.

- **A raster's hex is read before it is sent.** A raster travels as the hex of
  its well-known binary, and PostGIS answers `rt_raster_from_wkb: wkb size
  (14) < min size (61)` for something that is not one -- a complaint about the
  bytes it managed to decode rather than about the text it was given, which in
  the corpus is `this is invalid raster value`. Hasura reads the text first
  and says it is not hexadecimal.

  Only what is certainly not hexadecimal is refused: a character outside
  `0-9A-Fa-f`, or an odd number of them, since a byte takes two. Well-formed
  hex too short to be a raster still goes to PostGIS and comes back in
  PostGIS's words -- the same narrowness the `ltree` rule takes, and for the
  same reason.

- **Fragments that spread each other are named as a cycle.** A fragment that
  spreads its way back to itself describes no finite selection.
  async-graphql notices that something went too deep -- `The recursion depth
  of the query cannot be greater than 32` -- which says a limit was reached
  rather than what was wrong. Hasura names the fragments that went round.

  The walk already held a *set* of fragments to stop itself looping; a stack
  replaces it, because what a cycle is named after is the run of fragments
  from the first occurrence to the repeat, and because a fragment spread twice
  in sibling positions is not a cycle and should be checked at both. A budget
  bounds the walk instead, since fragments that each spread the next twice
  double the work per level without any name repeating.

  The case also settles something the walk had been guessing: **a spread is a
  step in Hasura's path**, named after the fragment --
  `$.selectionSet.author.selectionSet.authorFragment.selectionSet.articles`
  -- and the cycle is reported against the selection set the offending spread
  is in, which by then has no selection set of its own to name.

- **A number that is not an `Int` is refused as one.** GraphQL's `Int` is a
  signed 32-bit integer and `2147483648` is not one. PostgreSQL says `integer
  out of range` when the statement runs; Hasura says so while reading the
  request, and names the value at the column it was written under. An `int8`
  column is a `bigint` scalar rather than an `Int`, so nothing that
  legitimately holds a large number is caught by this.

  Matching the message meant matching how Hasura spells the value:
  `2147483648` comes back as `2.147483648e9`, which is Haskell's `show` for a
  `Double` -- fixed notation while the magnitude is in `[0.1, 10^7)` and
  scientific otherwise, always with a decimal point in the mantissa. Rust's
  two float formats are the same two spellings and both give the shortest
  digits that read back exactly, so choosing between them is the whole of what
  the helper adds.

- **A `limit` is a number of rows, not any integer.** It is typed `Int` in the
  schema either server publishes and is not any `Int`: a page of -1 rows is
  not a page, and Hasura refuses it at the argument before the query is built
  rather than letting `LIMIT must not be negative` come back from the
  database. A string is refused there too, even one that reads as a number.

  `offset` is not the same rule, and the asymmetry is Hasura's rather than an
  omission here -- worth writing down because it reads like a bug. Its own
  corpus sends `offset: "1"` and expects rows back, and sends `offset: -1` and
  expects PostgreSQL's own `OFFSET must not be negative`. So the rule is about
  the argument named `limit` and nothing else.

- **A document that does not parse is not a query.** async-graphql reports a
  parse failure the way a compiler does -- the offending line, a caret under
  the column, and what it expected instead. Hasura says the document is not a
  query and names the document, at `$.query`. Nothing further can run either
  way, so that is the whole of the answer. Five cases, three of them in the
  `ltree` group and about the query text rather than about trees.

  Two came out beside it. A quoted enum value is a mistake about the language
  rather than about the members: `_eq: "red"` is `expected an enum value for
  type 'colors_enum', but found a string` whatever it spells, and the member
  list belongs to the unquoted form and to a variable, where a string is how
  JSON carries an enum. And two codes now come from the SQLSTATE: class 22 is
  `data-exception`, and 22025 -- a `LIKE` pattern ending mid-escape -- is
  `bad-request`, because the pattern came from the request. Only the codes the
  corpus pins are set; the rest is left to the classification that guesses
  from the message and says so, since replacing that guess wholesale would be
  the same guess with fewer places to notice it.

- **An `ltree` path is read before it is sent.** PostgreSQL refuses
  `Tree.Collections.` with `ltree syntax error` and nothing else; Hasura reads
  the path first and says what a path is, which is also what puts the refusal
  at `...where.path._ancestor` rather than at the request as a whole.

  Only an *empty label* is refused, and the narrowness is the finding. What
  counts as a label character is the database's locale to decide: `a-b`,
  `a_b`, `1.2` and `Ünï` are all valid paths on the image the harness runs,
  and in C locale the last of those is not -- checked against
  `postgis/postgis:16-3.4` rather than assumed. Refusing a character this
  server merely doubts would turn a working query into an error, and the
  reward for guessing would have been zero measured cases. What is refused is
  the one thing no locale accepts: a label with nothing in it.

- **An error says what Hasura says it says.** 101 cases agreed about the data
  and not about the whole body, and every one of them differed by the message
  text. 64 do not any more.

  Most were async-graphql's own validation wording -- `Unknown field "x" on
  type "y". Did you mean ...` against `field 'x' not found in type: 'y'`.
  Rewriting the text on the way out was the obvious approach and the wrong
  one: it matches a string this server does not own, and it leaves the path
  wrong besides, since a validation error carries no path at all.

  So the walk that already checks variables now finds what async-graphql would
  have found, and says it first -- it runs before validation, and a document
  with an error in it never reaches validation. It knows the registry and it
  knows where it is, so the message and the path come out right together: a
  field the type does not have, in a selection or inside an input object,
  which is one message in Hasura and two in async-graphql; an argument the
  field does not take, reported against the field; a value the enum does not
  have, listing the members; and a mutation sent to a schema with no mutation
  root, which Hasura answers `no mutations exist` because there is no type for
  the field to have not been in.

  The other half is the database. Hasura names the *kind* of violation before
  PostgreSQL's own words -- `Uniqueness violation. duplicate key value
  violates unique constraint "author_name_key"` -- and the kind is read from
  the SQLSTATE, never from the message: the message is localised and the code
  is not.

- **An error says where in the request it happened.** Hasura answers
  `$.selectionSet.insert_author.args.objects[0].bio` for a write it refused;
  this answered `$` for all of it. The reason is worth keeping: the only path
  available was async-graphql's *response* path, which names fields of the
  answer -- and the answer has no `objects` to name. A place inside an argument
  cannot be reached from the response at all.

  So the path is written where the error is raised, by whoever knows which row
  and which column it is about. Three places know: the variable walk, which
  threads a path as it descends; the insert, whose context carries each row's
  own place and extends it for a nested one; and the `distinct_on` rule, which
  names the field's arguments as a whole because it is about two of them
  disagreeing. The response path is still the fallback, spelled Hasura's way
  now.

  Two details the corpus settled rather than taste. One object written where a
  list is expected is that list's first item -- `objects: {location: $x}` is
  refused at `objects[0].location` -- because input coercion says so. And a
  refused `check` names the argument, `args.objects`, not the row inside it.

  21 cases, all of them already agreeing about the data.

- **A role may be granted "how many" without being granted "which".** A select
  permission naming no columns, with `allow_aggregations`, is how Hasura says
  that: the count is over rows the role may not see, one at a time or at all.
  Such a table used to be dropped here, because a type with no fields is not a
  legal type and there was nothing else keeping it alive.

  It is the same shape a table a role may only write already had -- present in
  the cache, absent from the object types -- so it hangs in the same place. The
  aggregate root is built and the list root is not; `nodes` is not a field of
  `<t>_aggregate`, because there is nothing for it to be a list of; `count`
  takes no `columns`, because there are none to name; the functions that read
  column data go with the columns; and the root's own `distinct_on` and
  `order_by` go too, since they name a column enum that is not built and could
  not be.

  The group agrees entirely now, and the two cases beside this one refuse for
  the reason Hasura refuses rather than because the root was missing.

- **A ceiling bounds the page and not the count.** The aggregate read the same
  rows `nodes` did, so a permission's `limit` became the answer to `count`: a
  role limited to one row of `article` counted one of three, and `max(id)` was
  the maximum of the row it was allowed to see rather than of the rows that are
  there. A ceiling exists to bound how many rows travel, and answering "how
  many are there" with "as many as I would have sent you" is not an answer to
  the question.

  The aggregate reads its own source now -- the same filter, the same order,
  and only the limit and offset the request itself asked for, because
  `article_aggregate(limit: 2)` is a question about two rows. The corpus proves
  this for the permission's ceiling; `PGRST_MAX_ROWS` is treated the same way
  because the reason is the same, and that half is pinned by an integration
  test rather than by the harness.

- **A preset reaches an update, and the update half of an upsert.** They were
  applied on insert and nowhere else, which made an update whose whole
  assignment comes from the permission a no-op: `update_resident(where: ...)`
  with no `_set` at all, under a permission whose `set` names `city`, fell
  through the "an update that changes nothing changes no rows" path and
  answered zero. An `ON CONFLICT DO UPDATE` -- which is an update -- wrote the
  columns the upsert named and none the permission did.

  Presets are written first now and override rather than collide: a request
  naming a preset column is not the duplicate-write error, it is a column the
  permission wins, and on the conflict side that column does not also take its
  value from `EXCLUDED`.

  One thing worth writing down, because it looked like it needed a feature and
  did not. Hasura's corpus presets `last_updated` to the *string* `"NOW()"`
  against a `timestamptz`, which reads as a demand that a preset be spliced in
  as SQL rather than bound as a value -- and splicing metadata into a statement
  is a thing to be sure about before doing. It is not needed: PostgreSQL's
  datetime input reads `'NOW()'` as the current transaction time, parens and
  all. Checked against the image the harness runs rather than assumed, and the
  preset stays a bound value cast to the column's type.

- **`_exists`, the predicate only a permission can write.**
  `{"_exists": {"_table": "user", "_where": {"id": "X-Hasura-User-Id",
  "is_admin": true}}}` asks whether a row exists in *another* table -- one no
  foreign key relates to this one. It is how Hasura writes "the caller is an
  administrator" without asking the caller to say so: the row that decides it
  lives in a table of its own, and whether an account is readable does not
  depend on which account it is.

  So the subselect is uncorrelated, which is the whole difference from the
  `EXISTS` a relationship predicate builds -- if there were key columns to join
  on, the permission would have been written as a relationship. Two things it
  deliberately does not do. The caller's own permissions are not applied inside
  it: the point is to consult a table the role has no access to, and in the
  corpus the role reading `account` through this predicate may not read
  `public.user` at all, so narrowing the subselect would make the predicate
  refuse itself. And `_table` is copied through the permission rewrite
  untouched, because it is a name and not a value -- rewriting it would read a
  table called `x-hasura-anything` as a session variable and wrap it in an
  `_eq` besides.

  Seven cases across all four verbs and a read, and the last of the write-only
  tables came good with it: `insert_account` had a schema after the previous
  change and a predicate it could not compile until this one.

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

**Choosing a role with a token.** A token that allows more than one identity
carries `x-hasura-allowed-roles` beside `x-hasura-default-role`, and a client
selects among them with an `X-Hasura-Role` header. That path is implemented and
unit-tested, and no case here exercises it: `check_query` sends the admin
secret with every request that names a role, because that is how Hasura's own
suite authenticates. So the corpus measures role selection by an administrator
and never role selection by a bearer.

It was found by reading rather than by running, which is worth recording on its
own -- the corpus is a good instrument for what it covers and says nothing
about what it does not, and a gap that no case can reach looks identical to a
gap that does not exist.

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
