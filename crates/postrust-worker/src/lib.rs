//! Postrust's REST API in a Cloudflare Worker, reaching PostgreSQL through
//! Hyperdrive.
//!
//! The request pipeline is `postrust-engine`, the same one the server runs, and
//! the database driver is `postrust_core::db::PgSocketDb`: the PostgreSQL wire
//! protocol over a `worker::Socket` to Hyperdrive. That driver is held to
//! answering exactly as the server's sqlx driver does. Every request runs in a
//! transaction that sets the token's role and claims, as on the server, so
//! row-level security applies the same way.
//!
//! What is different here, and why:
//!
//! * **One connection per request.** A Worker may not share an I/O object
//!   between requests. Hyperdrive pools connections on its side, so opening
//!   one costs little.
//! * **The schema cache is kept in Workers KV** (`POSTRUST_SCHEMA_CACHE`), so
//!   a cold isolate reads it rather than querying the catalog. It is loaded
//!   live on a miss, refreshed by a Cron Trigger or by an authenticated
//!   `POST /_/reload`, and re-read from KV once `PGRST_WORKER_SCHEMA_TTL`
//!   seconds (default 60) have passed. The KV copy expires after the TTL (at
//!   least 60 seconds), so a migration is seen without a reload too.
//! * **Rows are capped** at `PGRST_DB_MAX_ROWS`, 1000 unless set, because an
//!   isolate has 128 MB. `unlimited`, under that name or `PGRST_MAX_ROWS`,
//!   lifts the cap.
//! * **REST only.** No GraphQL, no subscriptions, no `LISTEN`.
//!
//! The REST API answers at the root, as PostgREST does: `/todos`, `/rpc/f`.
//! Configuration is the server's own variables (`PGRST_DB_SCHEMAS`,
//! `PGRST_DB_ANON_ROLE`, `PGRST_JWT_SECRET`, ...), set as Worker variables or
//! secrets. See docs/workers.md.
//!
//! Its semver covers what a deployment sees, not Rust items: see
//! docs/stability.md.

// The Worker itself is wasm32-only: a Worker's sockets, bindings and futures
// are JavaScript objects. What it decides without them is in `settings`,
// built for every target so that it is tested natively.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod settings;

#[cfg(target_arch = "wasm32")]
mod runtime;
