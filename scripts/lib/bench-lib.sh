#!/usr/bin/env bash
# Shared helpers for the benchmark harnesses.
#
# Sourced by scripts/bench.sh (Postrust on its own) and
# scripts/bench-compare.sh (Postrust against PostgREST, Hasura and
# PostGraphile). Not executable on its own.
#
# Callers are expected to have set: REQUESTS, CONCURRENCY, LOAD_TOOL.

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

log()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

require() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"
}

# ---------------------------------------------------------------------------
# Waiting and measuring
# ---------------------------------------------------------------------------

# Poll until a command succeeds or a wall-clock deadline passes.
#
# The deadline is measured in seconds rather than loop iterations so the wait is
# correct even where `sleep` returns early (sandboxes, some CI runners).
wait_until() {
    local timeout="$1"
    shift
    local deadline=$(($(date +%s) + timeout))

    until "$@" >/dev/null 2>&1; do
        if (($(date +%s) >= deadline)); then
            return 1
        fi
        sleep 0.5 || true
    done
}

# Resident set size of a pid, in KB. Works on both macOS and Linux.
rss_kb() {
    ps -o rss= -p "$1" 2>/dev/null | tr -d ' ' || echo 0
}

# Resident set size of a running container, in KB.
#
# `docker stats` reports a human string ("13.31MiB / 7.653GiB"), so the unit is
# parsed rather than assumed -- the value crosses from MiB to GiB under load and
# a hardcoded divisor would silently report a 1000x error.
container_rss_kb() {
    local name="$1" raw unit value
    raw="$(docker stats --no-stream --format '{{.MemUsage}}' "$name" 2>/dev/null | awk '{print $1}')"
    [[ -z "$raw" ]] && { echo 0; return; }

    unit="$(sed 's/[0-9.]//g' <<<"$raw")"
    value="$(sed 's/[^0-9.]//g' <<<"$raw")"

    case "$unit" in
        B)         awk -v v="$value" 'BEGIN { printf "%.0f", v / 1024 }' ;;
        KiB|kB|KB) awk -v v="$value" 'BEGIN { printf "%.0f", v }' ;;
        MiB|MB)    awk -v v="$value" 'BEGIN { printf "%.0f", v * 1024 }' ;;
        GiB|GB)    awk -v v="$value" 'BEGIN { printf "%.0f", v * 1024 * 1024 }' ;;
        *)         echo 0 ;;
    esac
}

# Image size, reported two ways because neither number alone is unambiguous.
#
# `docker image inspect .Size` means different things depending on the storage
# driver: with the containerd snapshotter it is the compressed (download) size,
# with the classic overlay2 store it is the uncompressed size. Verified here by
# calibrating against alpine:3.22, whose published compressed size is ~4 MB and
# whose uncompressed size is ~13 MB. So both are captured and labelled, and
# neither is silently presented as "the" image size.
image_layer_bytes() {
    docker image inspect "$1" --format '{{.Size}}' 2>/dev/null || echo 0
}

# Uncompressed on-disk size as docker reports it in `docker images`. Consistent
# across storage drivers, which is why it is the headline figure.
image_disk_size() {
    docker images "$1" --format '{{.Size}}' 2>/dev/null | head -1 || echo "n/a"
}

human_kb() {
    awk -v kb="$1" 'BEGIN { printf "%.1f MB", kb / 1024 }'
}

human_bytes() {
    awk -v b="$1" 'BEGIN { printf "%.1f MB", b / 1048576 }'
}

pick_load_generator() {
    # oha first: it reports percentiles as machine-readable JSON. `hey` is not
    # supported -- its text output was parsed from memory once and produced
    # silently wrong numbers, so it is better to require a tool whose output
    # this script has actually been checked against.
    for tool in oha ab; do
        if command -v "$tool" >/dev/null 2>&1; then
            echo "$tool"
            return 0
        fi
    done
    die "no load generator found -- install oha (brew install oha) or use ab"
}

# ---------------------------------------------------------------------------
# Load generators
#
# Each takes: <url> <header> [body]
# and prints:  "<requests_per_second> <p50_ms> <p95_ms> <p99_ms>"
#
# A non-empty body switches the request to POST with a JSON content type, which
# is how the GraphQL targets are driven.
# ---------------------------------------------------------------------------

run_oha() {
    local url="$1" header="$2" body="${3:-}"
    # `--output-format json`, not `--json`: the latter is not an oha flag and
    # makes it exit with a usage error.
    local args=(-n "$REQUESTS" -c "$CONCURRENCY" --no-tui --output-format json)
    [[ -n "$header" ]] && args+=(-H "$header")
    if [[ -n "$body" ]]; then
        args+=(-m POST -H "Content-Type: application/json" -d "$body")
    fi

    local output
    if ! output="$(oha "${args[@]}" "$url" 2>&1)"; then
        echo "ERROR oha-failed"
        return 0
    fi

    # Percentiles are reported in seconds. Any non-2xx response is surfaced
    # rather than averaged into a throughput figure.
    jq -r '
        (.statusCodeDistribution // {}) as $codes
        | ([$codes | to_entries[] | select(.key | startswith("2") | not) | .value]
           | add // 0) as $bad
        | if $bad > 0 then
              "ERROR non-2xx=\($bad)"
          else
              "\(.summary.requestsPerSec | floor) " +
              "\(((.latencyPercentiles.p50 * 10000) | floor) / 10) " +
              "\(((.latencyPercentiles.p95 * 10000) | floor) / 10) " +
              "\(((.latencyPercentiles.p99 * 10000) | floor) / 10)"
          end
    ' <<<"$output" 2>/dev/null || echo "ERROR oha-parse-failed"
}

run_ab() {
    local url="$1" header="$2" body="${3:-}"
    local args=(-q -n "$REQUESTS" -c "$CONCURRENCY")
    [[ -n "$header" ]] && args+=(-H "$header")

    local body_file=""
    if [[ -n "$body" ]]; then
        body_file="$(mktemp)"
        printf '%s' "$body" > "$body_file"
        args+=(-p "$body_file" -T "application/json")
    fi

    # ab reports "Failed requests: 0" for uniform-length error bodies, so
    # non-2xx responses are checked separately and reported as an error.
    local output
    output="$(ab "${args[@]}" "$url" 2>&1)"
    [[ -n "$body_file" ]] && rm -f "$body_file"

    local non_2xx
    non_2xx="$(awk '/Non-2xx responses:/ { print $3 }' <<<"$output")"
    if [[ -n "$non_2xx" && "$non_2xx" != "0" ]]; then
        echo "ERROR non-2xx=$non_2xx"
        return 0
    fi

    awk '
        /Requests per second:/ { rps = $4 }
        /^  50%/               { p50 = $2 }
        /^  95%/               { p95 = $2 }
        /^  99%/               { p99 = $2 }
        END { printf "%.0f %.1f %.1f %.1f\n", rps, p50, p95, p99 }
    ' <<<"$output"
}

run_load() {
    case "$LOAD_TOOL" in
        oha) run_oha "$1" "$2" "${3:-}" ;;
        ab)  run_ab  "$1" "$2" "${3:-}" ;;
    esac
}
