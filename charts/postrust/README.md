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

### AI node (optional)

`ai.enabled=true` adds a second Deployment, a Service and a ConfigMap running
`postrust-ai` ([docs/ai-workers.md](../../docs/ai-workers.md)) against the same
database. It is off by default.

| Key | Default | Notes |
|---|---|---|
| `ai.enabled` | `false` | |
| `ai.replicas` | `1` | |
| `ai.image.repository` | `postrust/postrust-ai` | No model servers in it; see `values.yaml` |
| `ai.image.tag` | `0.1.0` | Its own version line, not `appVersion` |
| `ai.config` | no models | The node's TOML, mounted as a file |
| `ai.auth.jwtSecretRef` | `{}` | `{name, key}` of a Secret holding the HS256 secret; unset means an open API |
| `ai.cache.persistence.enabled` | `false` | `emptyDir` when off |
| `ai.service.port` | `7700` | |

The chart sets the listen address, the node id (the pod name), the advertise URL
(the pod IP) and `DATABASE_URL` (the same Secret as the API server). GPU
scheduling (`ai.nodeSelector`, `ai.tolerations`, `ai.resources.limits`) is
sketched in `values.yaml`.

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
