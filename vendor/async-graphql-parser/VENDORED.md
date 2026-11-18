# Vendored async-graphql-parser

This is `async-graphql-parser` 7.0.17 from crates.io, byte-identical except for
one change to `src/graphql.pest`, plus the regenerated `src/parse/generated.rs` it implies (the same change is in
`patches/async-graphql-parser-enum-value.patch` at the repository root).

## What changed

The `boolean`, `null` and `enum_value` rules rejected any name that merely *starts with* `true`, `false` or
`null`. The GraphQL specification says an EnumValue is "Name but not `true`,
`false` or `null`" -- only those three exact names. `boolean` and `null` now match
their literal only when no other name character follows, so
`nullPrefixTestTable_pkey`, `trueish` and `falsey` parse while `null`, `true`
and `false` are still refused.

## Why

An upsert naming a constraint called `nullPrefixTestTable_pkey` was answered
with a parse error (Hasura conformance case
`graphql_mutation/insert/nullprefixcolumn`), as was any enum member such as
`truestore`.

## When to remove

When async-graphql-parser releases a version with the fix (the defect is also
present in 7.2.1): delete this directory, the `[patch.crates-io]` section in
the root `Cargo.toml`, `scripts/check-vendored-parser.sh` and its CI step, and
the patch file. `scripts/check-vendored-parser.sh` keeps this copy from drifting
from the pristine crate plus the patch in the meantime.

Crates published to crates.io cannot carry a `[patch]`, so consumers of
`postrust-graphql` from crates.io get the upstream parser; the fix reaches them
only through this repository's builds (the Docker images).
