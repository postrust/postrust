# postrust

Helm chart for the Postrust API server.

```bash
helm install postrust ./charts/postrust \
  --set database.url="postgres://user:password@host:5432/database" \
  --set auth.jwtSecret="..." \
  --set replicas=3
```

`database.url` is required; the chart refuses to render without it rather than
installing a deployment that cannot start. To keep the connection string out of
your values, create the Secret yourself and point the chart at it:

```bash
kubectl create secret generic postrust-db \
  --from-literal=DATABASE_URL="postgres://..." \
  --from-literal=PGRST_JWT_SECRET="..."

helm install postrust ./charts/postrust --set database.existingSecret=postrust-db
```

## Values

| Key | Default | Notes |
|---|---|---|
| `replicas` | `1` | |
| `image.repository` | `postrust/postrust` | |
| `image.tag` | chart `appVersion` | |
| `database.url` | — | Required unless `database.existingSecret` is set |
| `database.existingSecret` | `""` | Secret holding `DATABASE_URL`, optionally `PGRST_JWT_SECRET` |
| `auth.jwtSecret` | `""` | Enables JWT verification when set |
| `auth.anonRole` | `web_anon` | Role for requests without a token |
| `server.port` | `3000` | |
| `server.schemas` | `public` | |
| `server.compatMode` | `false` | Also serve the REST surface at the root |
| `server.maxRows` | `""` | Cap on rows per request; empty means none |
| `service.type` | `ClusterIP` | |
| `service.port` | `80` | |

## Notes

`PGRST_SERVER_HOST` is set to `0.0.0.0` by the chart. The server binds
`127.0.0.1` by default, which inside a container means the Service reaches
nothing.

The readiness probe uses `/_/ready`, which checks database connectivity, so a
pod that cannot reach PostgreSQL is kept out of the Service. Liveness uses
`/_/health`, which only reports that the process is up.

Verify changes without a cluster:

```bash
helm lint charts/postrust --set database.url=postgres://u:p@h:5432/d
helm template postrust charts/postrust --set database.url=postgres://u:p@h:5432/d
```
