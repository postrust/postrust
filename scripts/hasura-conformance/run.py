#!/usr/bin/env python3
"""
Replay extracted cases against one server and record raw responses.

The two servers are configured by different means and that asymmetry is
deliberate.

The reference is configured the way the suite intends: each group's fixture
files are POSTed to Hasura's own `/v1/query`, so tracking, relationships and
permissions are established by the engine that owns them. Immediately after
that the database is dumped.

The candidate never sees a fixture file. It gets the dump. Translating
Hasura's metadata commands into something this server understands was the
obvious approach and the wrong one: a translator that got a column type or an
insert order subtly wrong would show up as a divergence in the server, which
is the one failure mode a differential harness exists to rule out. Restoring
the reference's own database removes the translation, and with it the
question.

State is held identical the same way the PostgREST harness holds it: reads
run first as one block from a clean load, and each mutating file gets the
data restored immediately before it. A file's cases are replayed in order --
a sequence file is an insert followed by the select that reads it back, and
splitting it would measure nothing.
"""
import argparse
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

import yaml


def shell(command):
    if not command:
        return
    subprocess.run(command, shell=True, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def post(url, payload, headers, timeout=30):
    body = json.dumps(payload).encode()
    request = urllib.request.Request(url, data=body, method="POST")
    request.add_header("Content-Type", "application/json")
    for name, value in headers.items():
        request.add_header(name, value)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.status, dict(response.headers), response.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as err:
        return err.code, dict(err.headers), err.read().decode("utf-8", "replace")


def flatten(doc):
    """A fixture file is one command, a `bulk` of them, or a bare list."""
    items = doc if isinstance(doc, list) else [doc]
    commands = []
    for item in items:
        if not isinstance(item, dict):
            continue
        if item.get("type") == "bulk":
            commands += [a for a in (item.get("args") or []) if isinstance(a, dict)]
        else:
            commands.append(item)
    return commands


def apply_fixtures(paths, corpus, hasura_url, secret, tolerate_errors=False):
    """Send a group's fixture commands to Hasura, in order.

    The corpus straddles two APIs. `run_sql` and the unprefixed commands are
    `/v1/query`; the source-aware spellings the newer fixtures use --
    `pg_track_table`, `pg_add_computed_field` -- are `/v1/metadata`, and
    posting one to the other's endpoint is rejected outright. Commands are
    sent one at a time rather than as the `bulk` they arrive in, because a
    file that mixes the two has no single endpoint that would accept it.
    """
    headers = {"X-Hasura-Admin-Secret": secret} if secret else {}
    for rel in paths:
        with open(os.path.join(corpus, "queries", rel)) as fh:
            doc = yaml.safe_load(fh)
        if doc is None:
            continue
        for command in flatten(doc):
            kind = command.get("type") or ""
            first = "/v1/metadata" if kind.startswith("pg_") else "/v1/query"
            second = "/v1/query" if first == "/v1/metadata" else "/v1/metadata"

            status, _, body = post(hasura_url + first, command, headers, timeout=120)
            if status >= 400 and "expected tag field" in body:
                status, _, body = post(hasura_url + second, command, headers, timeout=120)
            if status >= 400 and not tolerate_errors:
                return f"{rel} [{kind}]: {status} {body[:300]}"
    return None


def send(base, case, secret):
    """Send one case. Header selection mirrors validate.py's check_query: a
    case that names headers is speaking as that role, and one that names none
    is speaking as admin."""
    headers = dict(case["headers"])
    if not headers and secret:
        headers["X-Hasura-Admin-Secret"] = secret

    record = {k: case[k] for k in ("id", "group", "file", "seq", "url", "mutating")}
    try:
        status, response_headers, body = post(base + case["url"], case["payload"], headers)
        record.update(status=status,
                      headers={k.lower(): v for k, v in response_headers.items()},
                      body=body)
    except Exception as err:                       # connection reset, timeout, ...
        record.update(status=None, headers={}, body="",
                      error=f"{type(err).__name__}: {err}")
    return record


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("cases")
    parser.add_argument("out")
    parser.add_argument("--base", required=True)
    parser.add_argument("--mode", choices=["ref", "cand"], required=True)
    parser.add_argument("--corpus", help="ref mode: where the fixture files are")
    parser.add_argument("--admin-secret", default="")
    # Shell commands. `{group}` is replaced with the group's directory name
    # with slashes flattened, which is what the driver names its dumps after.
    parser.add_argument("--snapshot-cmd", help="ref: capture the loaded database")
    parser.add_argument("--restore-cmd", help="cand: reload a captured database")
    parser.add_argument("--data-reset-cmd", help="both: restore data, no DDL")
    parser.add_argument("--restart-cmd", help="cand: restart, to drop a stale schema cache")
    parser.add_argument("--teardown-cmd", help="ref: drop everything between groups")
    args = parser.parse_args()

    with open(args.cases) as fh:
        extracted = json.load(fh)
    groups = {g["dir"]: g for g in extracted["groups"]}
    by_group = {}
    for case in extracted["cases"]:
        by_group.setdefault(case["group"], []).append(case)

    results, failures = [], {}
    started = time.time()

    for position, name in enumerate(sorted(by_group), 1):
        group = groups.get(name)
        if group is None:
            continue
        slug = name.replace("/", "__")

        if args.mode == "ref":
            error = apply_fixtures(group["setup"], args.corpus, args.base, args.admin_secret)
            if error:
                failures[name] = error
                print(f"  [{position}/{len(by_group)}] {name}: setup failed -- {error}", flush=True)
                continue
            shell((args.snapshot_cmd or "").format(group=slug))
        else:
            try:
                shell((args.restore_cmd or "").format(group=slug))
            except subprocess.CalledProcessError:
                failures[name] = "restore failed"
                print(f"  [{position}/{len(by_group)}] {name}: restore failed", flush=True)
                continue
            # The schema cache is read once at startup and the GraphQL schema
            # is built from a snapshot of it, so new tables are invisible to a
            # process that was already running when they were created. The
            # restart also picks up this group's names, which are per-group for
            # the same reason the database is.
            shell((args.restart_cmd or "").format(group=slug))

        cases = by_group[name]
        reads = [c for c in cases if not c["mutating"]]
        writes = [c for c in cases if c["mutating"]]

        for case in reads:
            results.append(send(args.base, case, args.admin_secret))

        current_file = None
        for case in writes:
            if case["file"] != current_file:
                shell((args.data_reset_cmd or "").format(group=slug))
                current_file = case["file"]
            results.append(send(args.base, case, args.admin_secret))

        if args.mode == "ref":
            # Undo the group the way the suite does, through its own teardown
            # files, then sweep. A table left tracked but dropped makes
            # Hasura's metadata inconsistent and the next group's setup fails
            # for a reason that has nothing to do with the next group.
            apply_fixtures(group["teardown"], args.corpus, args.base,
                           args.admin_secret, tolerate_errors=True)
            shell(args.teardown_cmd)

        print(f"  [{position}/{len(by_group)}] {name}: "
              f"{len(reads)} read, {len(writes)} write  ({time.time() - started:.0f}s)",
              flush=True)

    with open(args.out, "w") as fh:
        json.dump({"results": results, "group_failures": failures}, fh)

    transport = sum(1 for r in results if r["status"] is None)
    print(f"done: {len(results)} requests, {len(failures)} groups unusable, "
          f"{transport} transport failures, {time.time() - started:.0f}s")


main()
