#!/usr/bin/env python3
"""
Turn Hasura's metadata into a PGRST_GRAPHQL_NAMES document.

Every name Hasura exposes that a database schema cannot supply is written down
in its metadata: `create_array_relationship` says the relationship is `posts`,
`add_computed_field` says the function `fetch_articles_plain` is exposed as
`get_articles`, `set_table_customization` renames the root fields. This reads
those and writes the equivalent document, so a migration does not begin by
copying names out of one file into another by hand.

    # from a running engine
    scripts/hasura-names.py --url http://localhost:8080 --admin-secret SECRET \\
        > graphql-names.json

    # from a metadata directory the CLI manages
    scripts/hasura-names.py --metadata-dir ./metadata > graphql-names.json

    # from an exported document
    scripts/hasura-names.py --file metadata.json > graphql-names.json

    PGRST_GRAPHQL_NAMES=./graphql-names.json postrust

Only names that actually differ from what this server would derive on its own
are emitted. A relationship Hasura called `articles` on a table called
`article` is the name reflection already produces, so writing it down would
only make the document harder to read and easier to leave stale.

Permissions are the exception to that rule, and are always written: nothing
derives a permission, so one that is not in the document does not exist. That
asymmetry is the point of them -- a role the document says nothing about has
no access, where a table the document says nothing about keeps every name
reflection gives it.

What is still not converted: tracked-table lists, actions, remote schemas and
event triggers. Those are the metadata model rather than a lookup table, and
this server does not have one.
"""
import argparse
import json
import os
import sys
import urllib.request


def die(message):
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def load_yaml(path):
    try:
        import yaml
    except ImportError:
        die("reading a metadata directory needs PyYAML: python3 -m pip install --user pyyaml")
    with open(path) as fh:
        return yaml.safe_load(fh)


def export_from_engine(url, secret):
    """`export_metadata` returns the whole document, whatever version it is."""
    headers = {"Content-Type": "application/json"}
    if secret:
        headers["X-Hasura-Admin-Secret"] = secret
    request = urllib.request.Request(
        url.rstrip("/") + "/v1/metadata",
        data=json.dumps({"type": "export_metadata", "args": {}}).encode(),
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.loads(response.read())


def tables_from_metadata_dir(directory):
    """The CLI keeps one file per table under databases/<source>/tables/.

    Functions sit beside them under `functions/`, in the same shape.
    """
    tables = []
    functions = []
    for root, _, filenames in os.walk(directory):
        holds = os.path.basename(root)
        if holds not in ("tables", "functions"):
            continue
        for filename in sorted(filenames):
            if not filename.endswith((".yaml", ".yml")):
                continue
            document = load_yaml(os.path.join(root, filename))
            # `tables.yaml` is a list of includes or of tables; a per-table file
            # is one mapping.
            for entry in document if isinstance(document, list) else [document]:
                if not isinstance(entry, dict):
                    continue
                if holds == "tables" and "table" in entry:
                    tables.append(entry)
                elif holds == "functions" and "function" in entry:
                    functions.append(entry)
    return tables, functions


def tables_from_document(document):
    """Walk an exported document, whichever version it is."""
    tables = []
    functions = []
    if not isinstance(document, dict):
        return tables, functions

    # v3: {metadata: {sources: [{tables: [...]}]}} or the bare {sources: [...]}
    metadata = document.get("metadata", document)
    for source in metadata.get("sources", []) or []:
        tables.extend(source.get("tables", []) or [])
        functions.extend(source.get("functions", []) or [])

    # v2: {tables: [...]} at the top level.
    if not tables:
        tables.extend(metadata.get("tables", []) or [])
    if not functions:
        functions.extend(metadata.get("functions", []) or [])

    return tables, functions


def tables_from_commands(paths):
    """Fold a sequence of metadata commands into table and function entries.

    A schema is not always an exported document. Migrations, a test suite's
    fixtures and anything scripted against `/v1/query` are a list of commands
    instead -- `create_array_relationship`, `add_computed_field`,
    `set_table_customization` -- applied in order. Folding them gives the same
    shape an export would have had.
    """
    entries = {}
    functions = []
    introspection_disabled = []

    def entry_for(table):
        schema, name = qualified(table)
        return entries.setdefault(
            (schema, name),
            {"table": {"schema": schema, "name": name}},
        )

    for path in paths:
        document = load_yaml(path)
        if document is None:
            continue
        commands = []
        for item in document if isinstance(document, list) else [document]:
            if not isinstance(item, dict):
                continue
            if item.get("type") == "bulk":
                commands += [a for a in (item.get("args") or []) if isinstance(a, dict)]
            else:
                commands.append(item)

        for command in commands:
            # The source-aware spellings mean the same thing here.
            kind = (command.get("type") or "")
            if kind.startswith("pg_"):
                kind = kind[len("pg_"):]
            args = command.get("args") or {}
            # `track_table` and `track_function` accept the bare name as their
            # whole argument -- `args: employees` -- and the corpus writes it
            # that way. Read as a dict it is nothing at all, which is how a
            # tracked table came to be missing from a converted document.
            if isinstance(args, str):
                args = {"function" if kind.endswith("_function") else "table": args}
            if not isinstance(args, dict):
                continue
            # A tracked function is not about a table, so it is collected
            # before the table entries are.
            if kind == "track_function":
                functions.append({
                    "function": args.get("function") or args.get("name"),
                    "configuration": args.get("configuration"),
                })
                continue
            # A function permission is about a function and a role, and is
            # the only thing that grants a mutation-exposed function to one.
            # Folded in order, since the suite grants and revokes.
            if kind in ("create_function_permission", "drop_function_permission"):
                functions.append({
                    "function": args.get("function") or args.get("name"),
                    ("permissions" if kind.startswith("create_") else "revoked"):
                        [{"role": args.get("role")}],
                })
                continue
            # Nor is this: it names roles and nothing else.
            if kind == "set_graphql_schema_introspection_options":
                introspection_disabled.extend(args.get("disabled_for_roles") or [])
                continue
            if "table" not in args:
                continue
            entry = entry_for(args["table"])

            if kind in ("create_object_relationship", "create_array_relationship"):
                field = ("object_relationships" if kind.endswith("object_relationship")
                         else "array_relationships")
                entry.setdefault(field, []).append(
                    {"name": args.get("name"), "using": args.get("using")}
                )
            elif kind == "add_computed_field":
                entry.setdefault("computed_fields", []).append(
                    {"name": args.get("name"),
                     "definition": args.get("definition"),
                     "comment": args.get("comment")}
                )
            elif kind.startswith("create_") and kind.endswith("_permission"):
                # `create_select_permission` -> `select_permissions`, which is
                # the key an exported document uses, so both inputs reach
                # `convert` in one shape.
                verb = kind[len("create_"):-len("_permission")]
                entry.setdefault(f"{verb}_permissions", []).append(
                    {"role": args.get("role"), "permission": args.get("permission") or {}}
                )
            elif kind.startswith("drop_") and kind.endswith("_permission"):
                verb = kind[len("drop_"):-len("_permission")]
                granted = entry.get(f"{verb}_permissions") or []
                entry[f"{verb}_permissions"] = [
                    p for p in granted if p.get("role") != args.get("role")
                ]
            elif kind == "set_table_is_enum":
                entry["is_enum"] = bool(args.get("is_enum", True))
            elif kind in ("track_table", "set_table_customization", "add_existing_table_or_view"):
                configuration = args.get("configuration")
                if isinstance(configuration, dict):
                    # A later customization replaces an earlier one, which is
                    # what applying them in order means.
                    entry["configuration"] = configuration
                if "is_enum" in args:
                    entry["is_enum"] = bool(args["is_enum"])

    return list(entries.values()), functions, introspection_disabled


def qualified(table):
    """Hasura writes a table as a name or as {schema, name}."""
    if isinstance(table, str):
        return "public", table
    if isinstance(table, dict):
        return table.get("schema") or "public", table.get("name") or ""
    return "public", ""


def join_columns(columns):
    """The key a key over more than one column is written under.

    A composite foreign key has no single column to be named by, and Hasura
    names one by its columns rather than by its constraint. Sorted, because
    the order the two sides list the same columns in is not something either
    of them promises -- the server sorts them the same way.
    """
    if not columns:
        return None
    if len(columns) == 1:
        return columns[0]
    return ",".join(sorted(columns))


def relationship_key(entry, kind):
    """The key this server will look the relationship up under.

    Hasura identifies a relationship by the column carrying the foreign key,
    and which side that column is on depends on the direction: an object
    relationship points out through a column of its own table, an array
    relationship is pointed at through a column of the other one. Both
    spellings are keys this server accepts, so no database is needed to turn
    either into the constraint that carries it.
    """
    using = entry.get("using") or {}
    on = using.get("foreign_key_constraint_on")

    if isinstance(on, str):
        return on
    if isinstance(on, list):
        return join_columns(on)
    if isinstance(on, dict):
        _, table = qualified(on.get("table"))
        columns = on.get("columns") or on.get("column")
        if isinstance(columns, list):
            columns = join_columns(columns)
        if table and columns:
            return f"{table}.{columns}"
        if columns:
            return columns
        return None

    # `manual_configuration` maps columns without naming a key. Which column
    # belongs to which side is not recoverable from the mapping alone for an
    # array relationship, so only the unambiguous direction is converted.
    manual = using.get("manual_configuration") or {}
    mapping = manual.get("column_mapping") or {}
    if kind == "object" and len(mapping) == 1:
        return next(iter(mapping))
    return None


# What each root field would be called, given a base name. The server derives
# all of them from one word; Hasura names them one at a time.
DERIVED_ROOTS = {
    "select": lambda base: base,
    "select_by_pk": lambda base: f"{base}_by_pk",
    "select_aggregate": lambda base: f"{base}_aggregate",
    "insert": lambda base: f"insert_{base}",
    "insert_one": lambda base: f"insert_{base}_one",
    "update": lambda base: f"update_{base}",
    "update_by_pk": lambda base: f"update_{base}_by_pk",
    "update_many": lambda base: f"update_{base}_many",
    "delete": lambda base: f"delete_{base}",
    "delete_by_pk": lambda base: f"delete_{base}_by_pk",
}


def custom_roots(entry):
    """The root names Hasura was told.

    A root may be written as a bare name or as `{name, comment}`, and in the
    second shape the name may be null -- which says "keep the derived name and
    only change the comment". Both spellings appear in the same corpus.
    """
    configuration = entry.get("configuration") or {}
    named = {}
    for root, value in (configuration.get("custom_root_fields") or {}).items():
        if isinstance(value, str):
            name = value
        elif isinstance(value, dict):
            name = value.get("name")
        else:
            continue
        if isinstance(name, str) and name:
            named[root] = name
    return named


def custom_root_comments(entry):
    """The comments Hasura was told to put on the root fields."""
    configuration = entry.get("configuration") or {}
    comments = {}
    for root, value in (configuration.get("custom_root_fields") or {}).items():
        if isinstance(value, dict) and isinstance(value.get("comment"), str):
            comments[root] = value["comment"]
    return comments


def exposed_base(entry, table):
    """The one word this server derives every name for this table from.

    `custom_name`, or the table's own name. Deliberately *not* inferred from
    the custom root fields, however neatly they share a stem: in Hasura those
    rename the root fields and nothing else, while a base name here renames the
    generated types too. A table whose roots are `AutomaticNoCommentInDb` and
    whose type is still `automatic_no_comment_in_db` is the case that says so,
    and it is in the corpus.

    Computed once and used twice: it decides this table's own names, and it
    decides what a relationship *to* this table derives to.
    """
    configuration = entry.get("configuration") or {}
    custom = configuration.get("custom_name")
    if isinstance(custom, str) and custom:
        return custom
    return table


def convert(tables):
    names = {}

    # What each table is exposed as, which a relationship to it derives from.
    bases = {}
    for entry in tables:
        schema, table = qualified(entry.get("table"))
        if not table:
            continue
        bases[f"{schema}.{table}"] = exposed_base(entry, table)

    for entry in tables:
        schema, table = qualified(entry.get("table"))
        if not table:
            continue
        key = f"{schema}.{table}"
        given = {}

        # Root field names. Hasura's customization names each root separately;
        # this server derives all of them from one base name, so a set that
        # shares a stem is converted and anything else is reported.
        # Not a name, and here for the same reason the names are: nothing in
        # the schema says a table is a set of allowed values rather than a set
        # of rows.
        if entry.get("is_enum"):
            given["enum"] = True

        configuration = entry.get("configuration") or {}
        base = bases[key]
        if base != table:
            given["name"] = base

        # Whatever the base name does not reproduce is written down root by
        # root: `select_by_pk: Article` beside `select: Articles` needs an
        # entry, because one base name derives `Articles_by_pk`.
        # `select_stream` and `update_many` name surfaces this server does not
        # generate, so they are dropped rather than named.
        roots = {root: value for root, value in custom_roots(entry).items()
                 if root in DERIVED_ROOTS and DERIVED_ROOTS[root](base) != value}
        if roots:
            given["roots"] = roots

        # Hasura keeps a column's exposed name in two places depending on its
        # version: `custom_column_names` maps column to name directly, and
        # `column_config` wraps it in an object beside other per-column
        # settings.
        columns = dict(configuration.get("custom_column_names") or {})
        for column, config in (configuration.get("column_config") or {}).items():
            if isinstance(config, dict) and isinstance(config.get("custom_name"), str):
                columns[column] = config["custom_name"]
        columns = {column: name for column, name in columns.items()
                   if isinstance(name, str) and name and name != column}
        if columns:
            given["columns"] = columns

        # Comments. Hasura keeps a description in metadata as readily as a
        # name, and where it does the database's own comment is not what a
        # client sees -- an empty string is "no description", not "no opinion".
        comments = {}
        if isinstance(configuration.get("comment"), str):
            comments["table"] = configuration["comment"]
        column_comments = {
            column: config["comment"]
            for column, config in (configuration.get("column_config") or {}).items()
            if isinstance(config, dict) and isinstance(config.get("comment"), str)
        }
        if column_comments:
            comments["columns"] = column_comments
        root_comments = {root: text for root, text in custom_root_comments(entry).items()
                         if root in DERIVED_ROOTS}
        if root_comments:
            comments["roots"] = root_comments
        computed_comments = {}
        for field in entry.get("computed_fields") or []:
            function = (field.get("definition") or {}).get("function")
            _, function_name = qualified(function)
            if function_name and isinstance(field.get("comment"), str):
                computed_comments[function_name] = field["comment"]
        if computed_comments:
            comments["computed_fields"] = computed_comments
        if comments:
            given["comments"] = comments

        # Which relationships the table has, and how each is reached. Written
        # whenever metadata declares any, because the declaration is the whole
        # list: Hasura offers the relationships it was told about, where
        # reflection offers one per foreign key. A table nothing is declared
        # for is left to reflection, which is what a document that says
        # nothing about it has always meant.
        declared = []
        seen = set()
        for kind, field in (("object", "object_relationships"),
                            ("array", "array_relationships")):
            for relationship in entry.get(field) or []:
                name = relationship.get("name")
                if not name or name in seen:
                    if name in seen:
                        print(
                            f"note: {key} declares \"{name}\" twice; keeping the first",
                            file=sys.stderr,
                        )
                    continue
                seen.add(name)
                using = relationship.get("using") or {}
                manual = using.get("manual_configuration") or {}
                mapping = manual.get("column_mapping") or {}
                if mapping:
                    schema, table = qualified(manual.get("remote_table"))
                    if not table:
                        print(
                            f"note: {key}.{name} maps columns to a table this cannot "
                            f"name; it is left out",
                            file=sys.stderr,
                        )
                        continue
                    # Which row a nested insert writes first. A mapping says
                    # only that two columns are equal, which is equal in both
                    # directions, so Hasura writes the order down.
                    declared.append({
                        "name": name,
                        "table": f"{schema}.{table}",
                        "columns": dict(mapping),
                        "to_one": kind == "object",
                        "after_parent":
                            manual.get("insertion_order") == "after_parent",
                    })
                    continue
                lookup = relationship_key(relationship, kind)
                if lookup is None:
                    print(
                        f"note: {key}.{name} is reached in a way this cannot describe; "
                        f"it is left out",
                        file=sys.stderr,
                    )
                    continue
                declared.append({"name": name, "using": lookup,
                                 "to_one": kind == "object"})
        # Written even when empty, and that is the point: metadata mentioning a
        # table and declaring no relationship for it is Hasura saying the table
        # has none. Absent means "reflect", which is what a hand-written
        # document that says nothing about a table has always meant -- so the
        # empty list is the only way a converted document can say the
        # difference.
        given["declared_relationships"] = declared

        computed = {}
        for field in entry.get("computed_fields") or []:
            name = field.get("name")
            function = (field.get("definition") or {}).get("function")
            _, function_name = qualified(function)
            if name and function_name and name != function_name:
                computed[function_name] = name
        if computed:
            given["computed_fields"] = computed

        permissions = convert_permissions(entry)
        if permissions:
            given["permissions"] = permissions

        if given:
            names[key] = given

    return names


# Which keys of a Hasura permission this server reads, per verb. Everything
# else Hasura accepts there -- `allow_upsert`, `query_root_fields`,
# `subscription_root_fields`, `validate_input` -- is reported rather than
# dropped silently, because a permission that converts to something weaker
# than it was is worse than one that fails to convert.
PERMISSION_KEYS = {
    "select": {"columns", "filter", "limit", "allow_aggregations", "computed_fields"},
    "insert": {"columns", "check", "set", "backend_only"},
    "update": {"columns", "filter", "check", "set"},
    "delete": {"filter"},
}


def convert_permissions(entry):
    """What each role may do with one table, keyed by role.

    Unlike the names, a permission is converted whether or not it differs from
    something derived, because nothing derives it: a permission that is not
    written down does not exist, and the absence of one is itself the
    statement that the role has no access.
    """
    _, table = qualified(entry.get("table"))
    by_role = {}

    for verb, wanted in PERMISSION_KEYS.items():
        for granted in entry.get(f"{verb}_permissions") or []:
            role = granted.get("role")
            permission = granted.get("permission")
            if not role or not isinstance(permission, dict):
                continue

            unread = sorted(set(permission) - wanted)
            if unread:
                print(
                    f"note: {table}.{verb} for \"{role}\" says "
                    f"{', '.join(unread)}, which this server does not read",
                    file=sys.stderr,
                )

            kept = {k: v for k, v in permission.items() if k in wanted}
            # A write permission that names no columns covers all of them.
            # Worth stating rather than leaving to a default, because getting
            # it the other way round refuses every insert the permission was
            # written to allow: `author` for `user` in
            # `graphql_mutation/insert/permissions` says only `check`, and the
            # corpus expects an insert naming three columns to succeed.
            if "columns" in wanted and "columns" not in kept:
                kept["columns"] = "*"
            by_role.setdefault(role, {})[verb] = kept

    return by_role


def convert_functions(functions):
    """What metadata says about a function that reflection cannot derive.

    Two things. Which root it is exposed on: this server places a function by
    its volatility, which is what the catalogue records; Hasura lets
    `track_function` override it, and a VOLATILE function tracked with
    `exposed_as: query` is a decision no schema remembers.

    And which roles may call it. Hasura infers a *query* function's permission
    from the select permission on the table it returns -- a role that may read
    the rows may ask the function for them, which reflection can derive. A
    function exposed as a **mutation** is not inferred from anything: it has
    side effects, and permission to read a table is not permission to change
    it. `pg_create_function_permission` is the only thing that says so.

    A function whose placement matches what volatility would give is left out,
    for the same reason a derived name is -- but that cannot be checked here
    without a database, so what is written down is every `exposed_as` metadata
    actually carries.
    """
    given = {}
    for entry in functions:
        schema, name = qualified(entry.get("function"))
        if not name:
            continue
        configuration = entry.get("configuration")
        exposed = None
        if isinstance(configuration, dict):
            exposed = configuration.get("exposed_as")
        # Folded rather than replaced: a document carries a function once,
        # but a sequence of commands tracks it and then grants it, and may
        # revoke afterwards.
        placed = given.setdefault(f"{schema}.{name}", {})
        if exposed in ("query", "mutation"):
            placed["exposed_as"] = exposed
        roles = placed.get("roles") or []
        for granted in entry.get("permissions") or []:
            if isinstance(granted, dict) and granted.get("role") not in (None, *roles):
                roles.append(granted["role"])
        for revoked in entry.get("revoked") or []:
            if isinstance(revoked, dict):
                roles = [role for role in roles if role != revoked.get("role")]
        if roles:
            placed["roles"] = roles
        else:
            placed.pop("roles", None)
    # A function nothing was actually said about is left out, for the same
    # reason a derived name is.
    return {key: placed for key, placed in given.items() if placed}


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--url", help="a running graphql-engine")
    source.add_argument("--metadata-dir", help="a metadata directory the CLI manages")
    source.add_argument("--file", help="an exported metadata document (JSON or YAML)")
    source.add_argument("--commands", nargs="+", metavar="FILE",
                        help="YAML files of metadata commands, applied in order")
    parser.add_argument("--admin-secret", default=os.environ.get("HASURA_GRAPHQL_ADMIN_SECRET"))
    args = parser.parse_args()

    introspection_disabled = []
    if args.url:
        tables, functions = tables_from_document(
            export_from_engine(args.url, args.admin_secret)
        )
    elif args.commands:
        tables, functions, introspection_disabled = tables_from_commands(args.commands)
    elif args.metadata_dir:
        tables, functions = tables_from_metadata_dir(args.metadata_dir)
    else:
        with open(args.file) as fh:
            head = fh.read()
        try:
            document = json.loads(head)
        except json.JSONDecodeError:
            document = load_yaml(args.file)
        tables, functions = tables_from_document(document)

    if not tables and not functions and not introspection_disabled:
        # An empty document is a legitimate answer for a schema that renames
        # nothing, and a caller generating one per group should not have to
        # special-case it.
        print("{}")
        return

    names = convert(tables)
    placed = convert_functions(functions)
    # The sectioned shape only where there is a second section to write. The
    # flat one came first, is still read, and is shorter for the schemas that
    # need nothing but names.
    document = names
    if placed or introspection_disabled:
        document = {"tables": names}
        if placed:
            document["functions"] = placed
        if introspection_disabled:
            document["introspection_disabled_for"] = sorted(set(introspection_disabled))
    print(json.dumps(document, indent=2, sort_keys=True))
    print(
        f"note: {len(names)} of {len(tables)} tables need a name written down; "
        f"the rest are what this server derives anyway",
        file=sys.stderr,
    )
    if placed:
        print(
            f"note: {len(placed)} function(s) are exposed on a root their "
            f"volatility would not put them on",
            file=sys.stderr,
        )


main()
