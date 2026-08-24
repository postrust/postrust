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

What is deliberately not converted: permissions, tracked-table lists, actions,
remote schemas and event triggers. Those are the metadata model rather than
names, and this server does not have one -- permissions live in the database as
roles and row level security. The document this writes is a lookup table.
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
    """The CLI keeps one file per table under databases/<source>/tables/."""
    tables = []
    for root, _, filenames in os.walk(directory):
        if os.path.basename(root) != "tables":
            continue
        for filename in sorted(filenames):
            if not filename.endswith((".yaml", ".yml")):
                continue
            document = load_yaml(os.path.join(root, filename))
            # `tables.yaml` is a list of includes or of tables; a per-table file
            # is one mapping.
            for entry in document if isinstance(document, list) else [document]:
                if isinstance(entry, dict) and "table" in entry:
                    tables.append(entry)
    return tables


def tables_from_document(document):
    """Walk an exported document, whichever version it is."""
    tables = []
    if not isinstance(document, dict):
        return tables

    # v3: {metadata: {sources: [{tables: [...]}]}} or the bare {sources: [...]}
    metadata = document.get("metadata", document)
    for source in metadata.get("sources", []) or []:
        tables.extend(source.get("tables", []) or [])

    # v2: {tables: [...]} at the top level.
    if not tables:
        tables.extend(metadata.get("tables", []) or [])

    return tables


def tables_from_commands(paths):
    """Fold a sequence of metadata commands into table entries.

    A schema is not always an exported document. Migrations, a test suite's
    fixtures and anything scripted against `/v1/query` are a list of commands
    instead -- `create_array_relationship`, `add_computed_field`,
    `set_table_customization` -- applied in order. Folding them gives the same
    shape an export would have had.
    """
    entries = {}

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
            if not isinstance(args, dict) or "table" not in args:
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
                    {"name": args.get("name"), "definition": args.get("definition")}
                )
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

    return list(entries.values())


def qualified(table):
    """Hasura writes a table as a name or as {schema, name}."""
    if isinstance(table, str):
        return "public", table
    if isinstance(table, dict):
        return table.get("schema") or "public", table.get("name") or ""
    return "public", ""


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
        return on[0] if len(on) == 1 else None
    if isinstance(on, dict):
        _, table = qualified(on.get("table"))
        columns = on.get("columns") or on.get("column")
        if isinstance(columns, list):
            columns = columns[0] if len(columns) == 1 else None
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


def pluralize(word):
    """The server's own rule, in schema/relationship.rs."""
    if word.endswith("s") and not word.endswith("ss"):
        return word
    if word.endswith(("x", "ch", "sh", "ss")):
        return word + "es"
    if word.endswith("y") and not word.endswith(("ey", "ay", "oy")):
        return word[:-1] + "ies"
    return word + "s"


def singularize(word):
    """The server's own rule, in schema/relationship.rs."""
    if word.endswith("ies"):
        return word[:-3] + "y"
    if word.endswith(("ses", "xes", "ches", "shes")):
        return word[:-2]
    if word.endswith("s") and not word.endswith("ss"):
        return word[:-1]
    return word


def derived_relationship_name(entry, kind):
    """What this server would call it without being told.

    A relationship to many rows takes the target table's name pluralised, one
    to a single row its singular -- the same two rules the server applies. An
    entry that matches is left out: writing down a name reflection already
    produces makes the document longer to read and gives it something to go
    stale against.
    """
    using = entry.get("using") or {}
    on = using.get("foreign_key_constraint_on")
    if kind == "array" and isinstance(on, dict):
        _, table = qualified(on.get("table"))
        return pluralize(table) if table else None
    if kind == "object":
        # The object side names the table it points at, which the metadata
        # does not carry -- only the column carrying the key. `author_id`
        # pointing at `author` is the ordinary spelling and the only one that
        # can be checked without a database.
        if isinstance(on, str) and on.endswith("_id"):
            return singularize(on[: -len("_id")])
    return None


def convert(tables):
    names = {}

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
        custom_name = configuration.get("custom_name")
        if custom_name and custom_name != table:
            given["name"] = custom_name

        # Hasura names each root separately -- `select: Authors`,
        # `select_by_pk: Author`, `select_aggregate: AuthorAgg`. This server
        # derives all of them from one base name, so a set that agrees on one
        # converts and a set that does not is reported rather than guessed at.
        roots = configuration.get("custom_root_fields") or {}
        if roots and "name" not in given:
            def strip_suffix(value, suffix):
                return value[: -len(suffix)] if value.endswith(suffix) else None

            def strip_prefix(value, prefix):
                return value[len(prefix):] if value.startswith(prefix) else None

            implied = {
                "select": lambda v: v,
                "select_by_pk": lambda v: strip_suffix(v, "_by_pk"),
                "select_aggregate": lambda v: strip_suffix(v, "_aggregate"),
                "insert": lambda v: strip_prefix(v, "insert_"),
                "update": lambda v: strip_prefix(v, "update_"),
                "delete": lambda v: strip_prefix(v, "delete_"),
            }
            bases = {implied[root](value) for root, value in roots.items()
                     if root in implied and isinstance(value, str)}
            bases.discard(None)
            if len(bases) == 1:
                base = bases.pop()
                if base != table:
                    given["name"] = base
            elif bases:
                print(
                    f"note: {key} names its roots separately ({', '.join(sorted(roots))}); "
                    f"this server derives them from one base name, so they are left alone",
                    file=sys.stderr,
                )

        if configuration.get("custom_column_names"):
            print(
                f"note: {key} renames columns, which this server does not; "
                f"those fields keep their database names",
                file=sys.stderr,
            )

        relationships = {}
        for kind, field in (("object", "object_relationships"),
                            ("array", "array_relationships")):
            for relationship in entry.get(field) or []:
                name = relationship.get("name")
                if not name:
                    continue
                lookup = relationship_key(relationship, kind)
                if lookup is None:
                    print(
                        f"note: {key}.{name} is a manual relationship this cannot key; "
                        f"name it by its constraint if it needs one",
                        file=sys.stderr,
                    )
                    continue
                if derived_relationship_name(relationship, kind) == name:
                    continue          # reflection already produces this name
                if lookup in relationships and relationships[lookup] != name:
                    print(
                        f"note: {key} names one foreign key twice, as "
                        f"\"{relationships[lookup]}\" and \"{name}\"; keeping the first",
                        file=sys.stderr,
                    )
                    continue
                relationships[lookup] = name
        if relationships:
            given["relationships"] = relationships

        computed = {}
        for field in entry.get("computed_fields") or []:
            name = field.get("name")
            function = (field.get("definition") or {}).get("function")
            _, function_name = qualified(function)
            if name and function_name and name != function_name:
                computed[function_name] = name
        if computed:
            given["computed_fields"] = computed

        if given:
            names[key] = given

    return names


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

    if args.url:
        tables = tables_from_document(export_from_engine(args.url, args.admin_secret))
    elif args.commands:
        tables = tables_from_commands(args.commands)
    elif args.metadata_dir:
        tables = tables_from_metadata_dir(args.metadata_dir)
    else:
        with open(args.file) as fh:
            head = fh.read()
        try:
            document = json.loads(head)
        except json.JSONDecodeError:
            document = load_yaml(args.file)
        tables = tables_from_document(document)

    if not tables:
        # An empty document is a legitimate answer for a schema that renames
        # nothing, and a caller generating one per group should not have to
        # special-case it.
        print("{}")
        return

    names = convert(tables)
    print(json.dumps(names, indent=2, sort_keys=True))
    print(
        f"note: {len(names)} of {len(tables)} tables need a name written down; "
        f"the rest are what this server derives anyway",
        file=sys.stderr,
    )


main()
