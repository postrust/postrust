#!/bin/bash
# Differential Hasura GraphQL conformance run.
#
# Hasura's Python suite, unlike PostgREST's, already speaks HTTP: `conftest.py`
# takes `--hge-urls` and points at whatever engine is listening. The cases are
# declarative YAML carrying a url, a GraphQL payload, headers and a status, so
# there is nothing to lift out of source.
#
# What is not portable is the database each case expects, because Hasura is
# configured rather than reflected: a group's `setup.yaml` is a metadata
# payload that creates tables, tracks them, names relationships and grants
# permissions. Translating those commands into something this server
# understands was the obvious approach and the wrong one -- a translator that
# got a column type or an insert order subtly wrong would surface as a
# divergence in the server, which is the one failure mode a differential
# harness exists to rule out.
#
# So the reference is configured by its own metadata API, and the candidate is
# given a dump of the database that produced. Both then answer the same
# requests and the responses are diffed.
#
# Usage:  scripts/hasura-conformance/conformance.sh [hasura-version]
set -euo pipefail

HGE_VERSION="${1:-v2.50.1}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="${HASURA_CONFORMANCE_WORK:-$HERE/.work}"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"

DB=hge-conf-db
REF=hge-conf-ref
NET=hge-conf-net
DB_PORT="${HASURA_CONFORMANCE_DB_PORT:-55433}"
REF_PORT="${HASURA_CONFORMANCE_REF_PORT:-55003}"
CAND_PORT="${HASURA_CONFORMANCE_CAND_PORT:-55004}"

# The corpus creates postgis, raster and ltree fixtures; the stock postgres
# image cannot load them.
DB_IMAGE="${HASURA_CONFORMANCE_DB_IMAGE:-postgis/postgis:16-3.4}"

# Matches how the suite runs the engine. With a secret set, a case that names
# no headers is admin and one that names them is speaking as that role, which
# is the distinction validate.py's check_query draws.
ADMIN_SECRET=hasuraconformance
# Every schema the corpus creates. Postrust exposes what it is told to; Hasura
# exposes whatever is tracked, so this list is how the two are made to agree.
DB_SCHEMAS="public,hge_tests,custom,test"

DATA_DB=hge_conf
META_DB=hge_conf_meta

mkdir -p "$WORK" "$WORK/dumps"
say() { printf '\n==> %s\n' "$*"; }

cleanup() {
    pkill -f 'target/release/postrust' 2>/dev/null || true
    docker rm -f "$REF" "$DB" >/dev/null 2>&1 || true
    docker network rm "$NET" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# The corpus is YAML and the standard library does not read it. Prefer an
# interpreter that already has PyYAML over installing one, since macOS ships
# a system Python that does.
PYTHON=""
for candidate in python3 /usr/bin/python3 python3.12 python3.11; do
    if command -v "$candidate" >/dev/null 2>&1 && "$candidate" -c 'import yaml' 2>/dev/null; then
        PYTHON="$candidate"
        break
    fi
done
if [ -z "$PYTHON" ]; then
    echo "error: no python3 with PyYAML found. Install it with:" >&2
    echo "       python3 -m pip install --user pyyaml" >&2
    exit 1
fi

if [ ! -x "$REPO_ROOT/target/release/postrust" ]; then
    echo "error: target/release/postrust not found." >&2
    echo "       cargo build --release -p postrust-server --features admin-ui" >&2
    exit 1
fi

say "Fetching the Hasura $HGE_VERSION test corpus"
# A blobless sparse clone of one directory: the full repository is far larger
# than the 13 MB of fixtures actually needed.
if [ ! -d "$WORK/hge/server/tests-py" ]; then
    rm -rf "$WORK/hge"
    git clone --depth 1 --branch "$HGE_VERSION" --filter=blob:none --sparse \
        https://github.com/hasura/graphql-engine.git "$WORK/hge" >/dev/null 2>&1
    git -C "$WORK/hge" sparse-checkout set server/tests-py >/dev/null 2>&1
fi
CORPUS="$WORK/hge/server/tests-py"

say "Starting the fixture database"
cleanup
docker network create "$NET" >/dev/null
docker run -d --name "$DB" --network "$NET" \
    -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=postgres \
    -p "$DB_PORT:5432" "$DB_IMAGE" >/dev/null
for _ in $(seq 1 60); do
    docker exec "$DB" pg_isready -h 127.0.0.1 -U postgres >/dev/null 2>&1 && break
    sleep 1
done

psql_do() {
    docker exec -e PGPASSWORD=postgres "$DB" \
        psql -q -U postgres -d "$1" -v ON_ERROR_STOP=1 "${@:2}"
}

# Hasura keeps its own metadata in the database it serves. Giving it a
# separate one means the data database can be swept between groups without
# taking the engine's source configuration with it.
create_databases() {
    psql_do postgres \
        -c "DROP DATABASE IF EXISTS $DATA_DB WITH (FORCE);" \
        -c "DROP DATABASE IF EXISTS $META_DB WITH (FORCE);" \
        -c "CREATE DATABASE $DATA_DB;" \
        -c "CREATE DATABASE $META_DB;" >/dev/null
    # Fixtures assume these are present rather than creating them, and they
    # are put somewhere the sweep between groups will not take them: an
    # extension installed into `public` is dropped along with it, and the
    # group after the first then fails with `type "geography" does not exist`.
    # The search path carries the unqualified type names the fixtures use.
    psql_do "$DATA_DB" \
        -c "CREATE SCHEMA IF NOT EXISTS extensions;" \
        -c "CREATE EXTENSION IF NOT EXISTS postgis SCHEMA extensions;" \
        -c "CREATE EXTENSION IF NOT EXISTS postgis_raster SCHEMA extensions;" \
        -c "CREATE EXTENSION IF NOT EXISTS citext SCHEMA extensions;" \
        -c "CREATE EXTENSION IF NOT EXISTS ltree SCHEMA extensions;" \
        -c "CREATE EXTENSION IF NOT EXISTS pgcrypto SCHEMA extensions;" >/dev/null
    psql_do postgres \
        -c "ALTER DATABASE $DATA_DB SET search_path = \"\$user\", public, extensions;" >/dev/null
}

# Everything a group created, removed. Extensions and the schemas that hold
# them survive; anything a fixture made does not.
cat > "$WORK/sweep.sql" <<SQL
DO \$\$
DECLARE
    s text;
BEGIN
    FOR s IN
        SELECT nspname FROM pg_namespace
         WHERE nspname NOT IN ('pg_catalog', 'information_schema', 'public',
                               'hdb_catalog', 'hdb_views', 'topology', 'tiger',
                               'tiger_data', 'extensions')
           AND nspname NOT LIKE 'pg\_%'
    LOOP
        EXECUTE format('DROP SCHEMA IF EXISTS %I CASCADE', s);
    END LOOP;
END \$\$;
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
GRANT ALL ON SCHEMA public TO postgres;
SQL
docker cp "$WORK/sweep.sql" "$DB:/sweep.sql" >/dev/null

# Between mutating files, only the rows are replaced. Found by exclusion
# rather than listed: a group may create schemas of its own, and a missed one
# surfaces later as a duplicate-key error on reload.
cat > "$WORK/truncate.sql" <<'SQL'
DO $$
DECLARE tables text;
BEGIN
    SELECT string_agg(format('%I.%I', schemaname, tablename), ', ')
      INTO tables FROM pg_tables
     WHERE schemaname NOT IN ('pg_catalog', 'information_schema', 'hdb_catalog',
                              'hdb_views', 'topology', 'tiger', 'tiger_data',
                              'extensions')
       AND schemaname NOT LIKE 'pg\_%';
    IF tables IS NOT NULL THEN
        EXECUTE 'TRUNCATE TABLE ' || tables || ' CASCADE';
    END IF;
END $$;
SQL
docker cp "$WORK/truncate.sql" "$DB:/truncate.sql" >/dev/null

say "Extracting cases from the corpus"
"$PYTHON" "$HERE/extract.py" "$CORPUS" "$WORK/cases.json"

# A group's fixtures name its relationships and computed fields, and those
# names are the largest single divergence measured here -- not because anything
# is unimplemented, but because reflection cannot recover a name nobody wrote
# down. A migration converts them; so does this, from the same metadata, with
# scripts/hasura-names.py.
#
# This makes the run measure a *configured* server rather than a bare one.
# That is the fair comparison -- it is what migrating actually involves -- but
# it is a different measurement, and the report says which one it is.
say "Converting each group's names"
mkdir -p "$WORK/names"
"$PYTHON" - "$WORK/cases.json" "$CORPUS/queries" "$WORK/names" "$REPO_ROOT/scripts/hasura-names.py" <<'PY'
import json, os, subprocess, sys
cases, queries, out_dir, converter = sys.argv[1:5]
with open(cases) as fh:
    groups = json.load(fh)["groups"]
named = 0
for group in groups:
    slug = group["dir"].replace("/", "__")
    setup = [os.path.join(queries, rel) for rel in group["setup"]]
    if not setup:
        continue
    result = subprocess.run([sys.executable, converter, "--commands", *setup],
                            capture_output=True, text=True)
    if result.returncode != 0:
        print(f"  {group['dir']}: {result.stderr.strip()[:120]}", flush=True)
        continue
    document = result.stdout.strip() or "{}"
    with open(os.path.join(out_dir, f"{slug}.json"), "w") as fh:
        fh.write(document)
    if document != "{}":
        named += 1
print(f"{named} of {len(groups)} groups name something the schema does not carry")
PY

# --- reference -------------------------------------------------------------
#
# The expensive half, and its answers only change when Hasura's version, the
# corpus, or the harness itself does -- so a previous run can be reused while
# iterating on the candidate. Set HASURA_CONFORMANCE_REUSE_REF=1 to do that --
# it reuses the per-group dumps too, which is what the candidate phase reads.
#
# "or the harness itself" is not a caveat, it is the thing that went wrong.
# `run.py` was changed to attach the admin secret to every case; every run
# after that reused a reference recorded before it, in which 142 cases had
# answered `access-denied` because the request was never authenticated. The
# harness went on reporting a permission-model difference that had been fixed.
# So the reference is stamped with what produced it, and a stamp that does not
# match is replayed rather than reused.
STAMP="$WORK/ref.stamp"
harness_stamp() {
    cat "$HERE/run.py" "$HERE/extract.py" "$WORK/cases.json" 2>/dev/null |
        shasum -a 256 | cut -d' ' -f1
}

SNAPSHOT_CMD="docker exec -e PGPASSWORD=postgres $DB pg_dump -U postgres -d $DATA_DB \
    --no-owner --no-acl -N hdb_catalog -N hdb_views -N topology -N tiger -N tiger_data -N extensions \
    -f /dump-{group}.sql && docker cp $DB:/dump-{group}.sql $WORK/dumps/{group}.sql"

# Between mutating files: the same dump with the schema left alone, so no DDL
# runs, object OIDs never change and neither server has to be restarted to
# drop a stale schema cache.
DATA_SNAPSHOT_CMD="docker exec -e PGPASSWORD=postgres $DB pg_dump -U postgres -d $DATA_DB \
    --data-only --disable-triggers --no-owner \
    -N hdb_catalog -N hdb_views -N topology -N tiger -N tiger_data -N extensions \
    -f /data-{group}.sql && docker cp $DB:/data-{group}.sql $WORK/dumps/{group}.data.sql"

WANTED_STAMP="$(harness_stamp)"
if [ "${HASURA_CONFORMANCE_REUSE_REF:-}" = "1" ] && [ -s "$WORK/ref.json" ] &&
   [ "$(cat "$STAMP" 2>/dev/null)" = "$WANTED_STAMP" ]; then
    say "Reusing the recorded Hasura $HGE_VERSION responses"
else
    if [ "${HASURA_CONFORMANCE_REUSE_REF:-}" = "1" ] && [ -s "$WORK/ref.json" ]; then
        say "The recorded responses were produced by a different harness -- replaying"
    fi
    say "Replaying against Hasura $HGE_VERSION (reference)"
    create_databases
    docker rm -f "$REF" >/dev/null 2>&1 || true
    docker run -d --name "$REF" --network "$NET" -p "$REF_PORT:8080" \
        -e HASURA_GRAPHQL_DATABASE_URL="postgres://postgres:postgres@$DB:5432/$DATA_DB" \
        -e HASURA_GRAPHQL_METADATA_DATABASE_URL="postgres://postgres:postgres@$DB:5432/$META_DB" \
        -e HASURA_GRAPHQL_ADMIN_SECRET="$ADMIN_SECRET" \
        -e HASURA_GRAPHQL_ENABLE_CONSOLE=false \
        "hasura/graphql-engine:$HGE_VERSION" >/dev/null
    for _ in $(seq 1 90); do
        curl -sf "http://localhost:$REF_PORT/healthz" >/dev/null 2>&1 && break
        sleep 1
    done

    "$PYTHON" "$HERE/run.py" "$WORK/cases.json" "$WORK/ref.json" \
        --base "http://localhost:$REF_PORT" --mode ref \
        --corpus "$CORPUS" --admin-secret "$ADMIN_SECRET" \
        --snapshot-cmd "$SNAPSHOT_CMD && $DATA_SNAPSHOT_CMD" \
        --data-reset-cmd "docker exec -e PGPASSWORD=postgres $DB psql -q -U postgres -d $DATA_DB -v ON_ERROR_STOP=1 -f /truncate.sql -f /data-{group}.sql" \
        --teardown-cmd "docker exec -e PGPASSWORD=postgres $DB psql -q -U postgres -d $DATA_DB -f /sweep.sql"

    # Written only after a replay that finished, so an interrupted one is not
    # mistaken for a reusable reference.
    printf '%s' "$WANTED_STAMP" > "$STAMP"
fi

# --- candidate -------------------------------------------------------------

say "Replaying against Postrust (candidate)"
create_databases

# Written out rather than exported: run.py restarts the server through
# `sh -c`, which does not inherit shell functions.
cat > "$WORK/restart.sh" <<SH
#!/bin/bash
# \$1 is the group, which selects the names converted from its own fixtures.
pkill -f 'target/release/postrust' 2>/dev/null || true
NAMES="$WORK/names/\$1.json"
[ -f "\$NAMES" ] || NAMES=""
PGRST_GRAPHQL_NAMES="\$NAMES" \
DATABASE_URL="postgres://postgres:postgres@localhost:$DB_PORT/$DATA_DB" \
PGRST_DB_SCHEMAS="$DB_SCHEMAS" PGRST_DB_ANON_ROLE=postgres \
PGRST_HASURA_ADMIN_SECRET="$ADMIN_SECRET" \
PGRST_DB_EXTRA_SEARCH_PATH="public,extensions" \
PGRST_SERVER_PORT="$CAND_PORT" PGRST_SERVER_HOST=127.0.0.1 \
PGRST_DB_AGGREGATES_ENABLED=true PGRST_LOG_LEVEL=warn \
    "$REPO_ROOT/target/release/postrust" >>"$WORK/postrust.log" 2>&1 &
for _ in \$(seq 1 60); do
    curl -sf "http://localhost:$CAND_PORT/" >/dev/null 2>&1 && exit 0
    sleep 0.25
done
exit 0
SH
chmod +x "$WORK/restart.sh"

start_postrust() {
    pkill -f 'target/release/postrust' 2>/dev/null || true
    DATABASE_URL="postgres://postgres:postgres@localhost:$DB_PORT/$DATA_DB" \
    PGRST_DB_SCHEMAS="$DB_SCHEMAS" PGRST_DB_ANON_ROLE=postgres \
    PGRST_HASURA_ADMIN_SECRET="$ADMIN_SECRET" \
    PGRST_SERVER_PORT="$CAND_PORT" PGRST_SERVER_HOST=127.0.0.1 \
    PGRST_DB_AGGREGATES_ENABLED=true PGRST_LOG_LEVEL=warn \
        "$REPO_ROOT/target/release/postrust" >>"$WORK/postrust.log" 2>&1 &
    for _ in $(seq 1 40); do
        curl -sf "http://localhost:$CAND_PORT/" >/dev/null 2>&1 && return 0
        sleep 0.25
    done
    return 0
}

RESTORE_CMD="docker exec -e PGPASSWORD=postgres $DB psql -q -U postgres -d $DATA_DB -f /sweep.sql >/dev/null 2>&1; \
    docker cp $WORK/dumps/{group}.sql $DB:/restore.sql >/dev/null && \
    docker exec -e PGPASSWORD=postgres $DB psql -q -U postgres -d $DATA_DB -f /restore.sql >/dev/null 2>&1 && \
    docker cp $WORK/dumps/{group}.data.sql $DB:/data-{group}.sql >/dev/null"

"$WORK/restart.sh"
"$PYTHON" "$HERE/run.py" "$WORK/cases.json" "$WORK/cand.json" \
    --base "http://localhost:$CAND_PORT" --mode cand \
    --admin-secret "$ADMIN_SECRET" \
    --restore-cmd "$RESTORE_CMD" \
    --restart-cmd "$WORK/restart.sh {group}" \
    --data-reset-cmd "docker exec -e PGPASSWORD=postgres $DB psql -q -U postgres -d $DATA_DB -v ON_ERROR_STOP=1 -f /truncate.sql -f /data-{group}.sql"

say "Report"
{
    echo "Names converted from each group's own metadata and given to the candidate"
    echo "(scripts/hasura-names.py). This measures a configured server, which is what"
    echo "migrating involves -- not a bare one."
    echo
    "$PYTHON" "$HERE/report.py" "$WORK/ref.json" "$WORK/cand.json" "$WORK/diff.json"
} | tee "$WORK/report.txt"
