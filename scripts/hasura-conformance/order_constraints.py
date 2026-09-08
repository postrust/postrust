#!/usr/bin/env python3
"""Put PRIMARY KEY constraints before UNIQUE ones in a pg_dump file.

Why this exists
---------------

PostgreSQL reports *whichever* unique index it reaches first when a row
violates more than one, and "first" means creation order. That makes the
constraint named in a uniqueness error a property of how the database was
built, not of the server under test.

The reference's database is built by Hasura's metadata from the corpus
fixture -- `create table author(id serial primary key, name text unique)` --
so its primary key index exists first. The candidate's database is built by
restoring a `pg_dump` of that same database, and pg_dump emits constraints
alphabetically, so `author_name_key` lands before `author_pkey`. Insert a row
violating both and the two servers name different constraints while behaving
identically:

    create table fixture(id serial primary key, name text unique);
    -- duplicate key value violates unique constraint "fixture_pkey"

    create table restored(id integer not null, name text);
    alter table restored add constraint restored_name_key unique (name);
    alter table restored add constraint restored_pkey primary key (id);
    -- duplicate key value violates unique constraint "restored_name_key"

Same server, same statement, two answers, decided by creation order alone.

Reordering the dump puts both databases in the same state, so a difference
that remains afterwards is the server's. The alternative -- building the
candidate's database from the fixture SQL instead of from a dump -- is the one
thing this harness deliberately refuses, because a translator that got a type
or an order subtly wrong would surface as a server divergence, which is the
failure a differential harness exists to rule out.

This is a change to the instrument, not to a result: it removes a difference
in the environment rather than hiding one in the answer.

Limits
------

An inline `PRIMARY KEY` is created before an inline `UNIQUE` in the same
`CREATE TABLE`, which is the shape every fixture in the corpus uses. A fixture
that added a unique constraint first and its primary key later would be
reordered *away* from its original order by this; nothing in a dump records
which came first, so no transformation can be right for both. Idempotent, and
a no-op on a dump with no constraints or only one kind.
"""

import re
import sys

# Each constraint is its own statement in a dump:
#
#     ALTER TABLE ONLY public.author
#         ADD CONSTRAINT author_pkey PRIMARY KEY (id);
CONSTRAINT = re.compile(
    r"ALTER TABLE (?:ONLY )?[^\n;]+\n\s+ADD CONSTRAINT [^\n;]+;\n",
    re.M,
)


def reorder(sql: str) -> str:
    """Move PRIMARY KEY statements ahead of the other constraints."""
    statements = CONSTRAINT.findall(sql)
    primary = [s for s in statements if "PRIMARY KEY" in s]
    others = [s for s in statements if "PRIMARY KEY" not in s]

    # Nothing to separate. Also the common case, so it costs one scan.
    if not primary or not others:
        return sql

    # Already in the wanted order.
    if statements == primary + others:
        return sql

    at = sql.index(statements[0])
    for statement in statements:
        sql = sql.replace(statement, "", 1)
    return sql[:at] + "".join(primary + others) + sql[at:]


def main() -> None:
    if len(sys.argv) != 2:
        print("usage: order_constraints.py <dump.sql>", file=sys.stderr)
        raise SystemExit(2)

    path = sys.argv[1]
    with open(path, encoding="utf-8") as handle:
        original = handle.read()

    reordered = reorder(original)
    if reordered != original:
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(reordered)


if __name__ == "__main__":
    main()
