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
                    {"name": args.get("name"),
                     "definition": args.get("definition"),
                     "comment": args.get("comment")}
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


def derived_relationship_name(entry, kind, bases):
    """What this server would call it without being told.

    A relationship to many rows takes the target's exposed base name
    pluralised, one to a single row its singular -- the same two rules the
    server applies. `bases` maps `schema.table` to that exposed name, because a
    table given a custom name changes what every relationship *to* it derives
    to: a `Articles`-named table is reached as `Articles`, not as `articles`.

    An entry that matches is left out: writing down a name reflection already
    produces makes the document longer to read and gives it something to go
    stale against.
    """
    using = entry.get("using") or {}
    on = using.get("foreign_key_constraint_on")
    if kind == "array" and isinstance(on, dict):
        schema, table = qualified(on.get("table"))
        if not table:
            return None
        return pluralize(bases.get(f"{schema}.{table}", table))
    if kind == "object":
        # The object side names the table it points at, which the metadata
        # does not carry -- only the column carrying the key. `author_id`
        # pointing at `author` is the ordinary spelling and the only one that
        # can be checked without a database.
        if isinstance(on, str) and on.endswith("_id"):
            target = on[: -len("_id")]
            return singularize(bases.get(f"public.{target}", target))
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
                if derived_relationship_name(relationship, kind, bases) == name:
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
