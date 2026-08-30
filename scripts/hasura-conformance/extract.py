#!/usr/bin/env python3
"""
Collect replayable GraphQL cases out of Hasura's Python integration suite.

Nothing needs lifting out of source here, unlike the PostgREST harness: the
cases in `server/tests-py/queries` are already declarative YAML carrying a
url, a GraphQL payload, the headers to send and the status to expect. What
this does is decide which of them can be replayed, and against which database
state.

Three things the layout does not say out loud:

  * A case file is either one case (a mapping) or several (a sequence). A
    sequence is ordered on purpose -- an insert followed by the select that
    reads it back -- so the cases in one file are never reordered or split.

  * The database a case expects is built by the setup files in the nearest
    ancestor directory that has any, and the family has four members that
    apply in order (`pre_setup`, `schema_setup`, `setup`, `values_setup`).
    A directory with only `schema_setup.yaml` is as much a group as one with
    `setup.yaml`; conftest.py falls back through the same list.

  * A group whose setup creates actions, remote schemas, event triggers or
    inherited roles is describing a subsystem this server does not have.
    Those groups are reported and skipped rather than counted as failures,
    because a case that cannot be set up measures nothing.
"""
import json
import os
import re
import sys

import yaml

CORPUS, OUT = sys.argv[1], sys.argv[2]

# Backends other than PostgreSQL. Their fixtures never load here.
OTHER_BACKEND = re.compile(r"mssql|bigquery|citus|mysql|sqlserver", re.I)

# Applied in this order to build a group's database.
SETUP_ORDER = ["pre_setup.yaml", "schema_setup.yaml", "setup.yaml", "values_setup.yaml"]
# Undone in this order.
TEARDOWN_ORDER = [
    "values_teardown.yaml",
    "teardown.yaml",
    "schema_teardown.yaml",
    "post_teardown.yaml",
]
IS_FIXTURE = re.compile(
    r"^(pre_setup|schema_setup|setup|values_setup"
    r"|teardown|schema_teardown|values_teardown|post_teardown)"
)

# Metadata commands that configure a subsystem rather than a table.
SUBSYSTEM_OPS = {
    "create_action", "drop_action", "set_custom_types",
    "add_remote_schema", "remove_remote_schema", "create_remote_relationship",
    "create_event_trigger", "delete_event_trigger",
    "create_cron_trigger", "create_scheduled_event",
    "add_inherited_role", "drop_inherited_role",
    "add_collection_to_allowlist", "create_query_collection",
    "create_rest_endpoint",
}

REPLAYABLE_URLS = {"/v1/graphql", "/v1alpha1/graphql"}


def load(path):
    with open(path) as fh:
        return yaml.safe_load(fh)


def fixture_files(directory, order):
    return [
        os.path.join(directory, name)
        for name in order
        if os.path.exists(os.path.join(directory, name))
    ]


def op_types(paths):
    """Every metadata command named by a group's setup files."""
    found = []
    for path in paths:
        try:
            doc = load(path)
        except Exception:
            continue
        for item in doc if isinstance(doc, list) else [doc]:
            if not isinstance(item, dict):
                continue
            if item.get("type") == "bulk":
                found += [
                    a.get("type") for a in (item.get("args") or []) if isinstance(a, dict)
                ]
            elif item.get("type"):
                found.append(item["type"])
    return [t for t in found if t]


def group_for(directory, root):
    """The nearest ancestor that carries fixtures, which is what builds the
    database this directory's cases expect."""
    current = directory
    while current.startswith(root):
        if fixture_files(current, SETUP_ORDER):
            return current
        current = os.path.dirname(current)
    return None


def is_mutating(payload):
    """Whether replaying this case leaves the database changed.

    Read off the operation keyword rather than the field names: a query named
    `insert_author` is still a query, and a mutation that only reads back is
    still run inside one. A batch is mutating if any operation in it is.
    """
    if isinstance(payload, list):
        return any(is_mutating(one) for one in payload if isinstance(one, dict))
    text = payload.get("query") or ""
    return re.search(r"(^|[\s{])mutation[\s({]", text) is not None


def main():
    root = os.path.join(CORPUS, "queries")
    cases, skipped, groups = [], {}, {}

    for directory, _, filenames in os.walk(root):
        for filename in sorted(filenames):
            if not filename.endswith((".yaml", ".yml")) or IS_FIXTURE.match(filename):
                continue
            path = os.path.join(directory, filename)
            if OTHER_BACKEND.search(path):
                continue
            try:
                doc = load(path)
            except Exception:
                continue

            entries = doc if isinstance(doc, list) else [doc]
            entries = [e for e in entries if isinstance(e, dict) and e.get("url")]
            entries = [e for e in entries if e["url"] in REPLAYABLE_URLS]
            if not entries:
                continue

            group = group_for(directory, root)
            rel_group = os.path.relpath(group, root) if group else None
            rel_file = os.path.relpath(path, root)

            if group is None:
                skipped.setdefault("no fixtures found", []).append(rel_file)
                continue
            if rel_group not in groups:
                subsystem = sorted(set(op_types(fixture_files(group, SETUP_ORDER))) & SUBSYSTEM_OPS)
                groups[rel_group] = {
                    "dir": rel_group,
                    "setup": [os.path.relpath(p, root) for p in fixture_files(group, SETUP_ORDER)],
                    "teardown": [
                        os.path.relpath(p, root) for p in fixture_files(group, TEARDOWN_ORDER)
                    ],
                    "subsystem_ops": subsystem,
                }
            if groups[rel_group]["subsystem_ops"]:
                reason = "needs " + ", ".join(groups[rel_group]["subsystem_ops"])
                skipped.setdefault(reason, []).append(rel_file)
                continue

            for index, entry in enumerate(entries):
                payload = entry.get("query") or {}
                # A batch: the body is a JSON array of operations and the
                # answer is an array of responses. One case, replayed as it
                # was written -- splitting it would measure something else.
                if isinstance(payload, list):
                    payload = [one for one in payload if isinstance(one, dict)]
                    if not payload:
                        continue
                elif not isinstance(payload, dict):
                    continue
                cases.append({
                    "id": rel_file if len(entries) == 1 else f"{rel_file}#{index}",
                    "group": rel_group,
                    "file": rel_file,
                    "seq": index,
                    "url": entry["url"],
                    "method": (entry.get("method") or "POST").upper(),
                    "headers": {k: str(v) for k, v in (entry.get("headers") or {}).items()},
                    "payload": payload,
                    "expected_status": entry.get("status"),
                    "mutating": is_mutating(payload),
                })

    # A file's cases are a sequence; the file is the smallest replayable unit.
    files = {}
    for case in cases:
        files.setdefault(case["file"], []).append(case)
    for entries in files.values():
        if any(c["mutating"] for c in entries):
            for c in entries:
                c["mutating"] = True

    out = {
        "groups": [groups[g] for g in sorted(groups) if not groups[g]["subsystem_ops"]],
        "cases": cases,
        "skipped": {k: sorted(v) for k, v in sorted(skipped.items())},
    }
    with open(OUT, "w") as fh:
        json.dump(out, fh, indent=1)

    mutating = sum(1 for c in cases if c["mutating"])
    print(f"{len(cases)} cases in {len(out['groups'])} groups "
          f"({len(cases) - mutating} read, {mutating} write), {len(files)} files")
    for reason, paths in out["skipped"].items():
        print(f"  skipped {len(paths):>4}  {reason}")


main()
