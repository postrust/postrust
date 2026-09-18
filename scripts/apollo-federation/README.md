# Apollo Federation compatibility fixture

This directory adapts Postrust to Apollo's official Federation subgraph
compatibility suite. The target schema and data are pinned to Apollo commit
`346d4882d4e72a9e505369557c44a349cbca71bc`.

- Upstream repository: <https://github.com/apollographql/apollo-federation-subgraph-compatibility>
- Vendored schema: `implementations/_template_library_/products.graphql`
- Test contract: `COMPATIBILITY.md`

`products.graphql` is the target contract. Do not change it merely to match
Postrust's current output; mismatches are compatibility findings.

## Start the fixture

From the repository root:

```bash
docker compose -f scripts/apollo-federation/compose.yaml up --build --wait
```

The Products subgraph is available at
<http://localhost:4001/v1/graphql>.

## Smoke tests

Inspect the subgraph SDL:

```bash
curl --fail --show-error http://localhost:4001/v1/graphql \
  -H 'content-type: application/json' \
  --data '{"query":"query { _service { sdl } }"}'
```

Query the canonical product row:

```bash
curl --fail --show-error http://localhost:4001/v1/graphql \
  -H 'content-type: application/json' \
  --data '{"query":"query { product(id: \"apollo-federation\") { id sku package } }"}'
```

Resolve that row as a Federation entity:

```bash
curl --fail --show-error http://localhost:4001/v1/graphql \
  -H 'content-type: application/json' \
  --data '{"query":"query { _entities(representations: [{__typename: \"Product\", id: \"apollo-federation\"}]) { ... on Product { id sku } } }"}'
```

Stop the containers and remove the disposable database volume:

```bash
docker compose -f scripts/apollo-federation/compose.yaml down --volumes
```

## Run Apollo's suite locally

Apollo prepends a generated Compose file from the repository root. Set the
build-context override so Docker resolves the Products image from that same
directory:

```bash
POSTRUST_BUILD_CONTEXT=. \
POSTRUST_FIXTURE_DIR=scripts/apollo-federation \
npx --yes @apollo/federation-subgraph-compatibility@2.2.2 docker \
  --compose scripts/apollo-federation/compose.yaml \
  --schema scripts/apollo-federation/products.graphql \
  --path /v1/graphql \
  --port 4001 \
  --format json \
  --debug
```

The report is written to `results.json` in the current directory. Omit
`--format json` to write `results.md` instead.

## Optional compatibility gaps

The database and metadata reproduce as much of the canonical model as Postrust
can currently express. The target schema intentionally retains features that
are not yet supported, including repeatable and nested keys, extended/external
fields, `@requires`, `@provides`, `@override`, `@tag`, `@inaccessible`,
`@composeDirective`, and `@interfaceObject`.

Apollo's required compatibility checks are enforced in CI. Optional capability
failures remain visible in the report without failing the workflow.

## GitHub Actions

`.github/workflows/apollo-federation.yml` runs Apollo's official compatibility
action on pull requests that change the GraphQL/Federation implementation or
this fixture. It can also be started manually with `workflow_dispatch`.

The action is pinned to the commit behind Apollo action release `v2.1.1`.
Required failures block the workflow; optional failures remain non-blocking.
The workflow uploads the suite results and fixture Docker logs as an artifact.
