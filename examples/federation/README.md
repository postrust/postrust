# Two databases behind Apollo Router

Two PostgreSQL databases, a Postrust subgraph over each, and Apollo Router in
front. It is the example from
[PostgreSQL as an Apollo Federation subgraph, without writing resolvers](https://postrust.org/blog/postgresql-apollo-federation-subgraph).

- **catalog** (`catalog.sql`) knows what products are: name and price.
- **reviews** (`reviews.sql`) knows what people said about them, and holds
  products only by key. A row-level security policy shows anonymous callers
  published reviews and an `editor` all of them.

`products` is the same entity in both. `catalog.json` and `reviews.json` mark
it shared, so both subgraphs serve it as `Product @key(fields: "id")`, each
with the fields its own database has. `PGRST_GRAPHQL_TYPE_PREFIX` keeps the
rest of the two generated schemas from colliding.

## Run it

```bash
docker compose up --wait catalog reviews
docker compose up -d router
```

The `compose` service runs `rover supergraph compose` against the two
subgraphs, and the router starts once it has. Queries go to
<http://localhost:4000>:

```bash
curl http://localhost:4000 -H 'content-type: application/json' \
  --data '{"query":"{ catalog_products { name reviews { rating body } } }"}'
```

Send a token whose `role` claim is `editor`, signed with the
`PGRST_JWT_SECRET` in `compose.yaml`, and the unpublished review appears too:
the router passes `Authorization` on to both subgraphs (`router.yaml`), and
each runs its part of the query as that role.

## Check it

```bash
./check.sh
```

`check.sh` brings everything up, sends the blog post's queries through the
router, compares the answers, and tears it down again (`KEEP=1` leaves it
running). `POSTRUST_IMAGE=...` runs it against another image, such as a local
build.

`docker compose down --volumes` stops it.
