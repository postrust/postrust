# Deployment

Deploy Postrust to various environments. This guide covers deploying both the standard Postrust server and customized versions with custom routes.

## Building with Custom Routes

If you've added [custom routes](./custom-routes.md), build your customized version:

### Standard Build

```bash
# Build optimized binary with custom routes
cargo build --release -p postrust-server

# Binary location
./target/release/postrust
```

### Build with Admin UI

```bash
# Include admin dashboard, Swagger UI, and GraphQL
cargo build --release -p postrust-server --features admin-ui
```

### Build Features

| Feature | Description |
|---------|-------------|
| `default` | Core REST API with custom routes |
| `admin-ui` | Admin dashboard + Swagger + GraphQL playground |

## Standalone Server

### Building for Production

```bash
# Build optimized binary
cargo build --release -p postrust-server

# Binary location
./target/release/postrust
```

### Running with systemd

Create `/etc/systemd/system/postrust.service`:

```ini
[Unit]
Description=Postrust REST API Server
After=network.target postgresql.service

[Service]
Type=simple
User=postrust
Group=postrust
WorkingDirectory=/opt/postrust
ExecStart=/opt/postrust/postrust
Restart=always
RestartSec=5

# Core Environment
Environment=DATABASE_URL=postgres://user:pass@localhost:5432/mydb
Environment=PGRST_DB_ANON_ROLE=web_anon
Environment=PGRST_JWT_SECRET=your-secret-key
Environment=PGRST_LOG_LEVEL=info

# Custom Routes Environment (for webhooks, etc.)
Environment=STRIPE_WEBHOOK_SECRET=whsec_...
Environment=GITHUB_WEBHOOK_SECRET=...

# Security
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl enable postrust
sudo systemctl start postrust
sudo systemctl status postrust
```

## Docker

### Using the Official Image

```bash
docker run -d \
  --name postrust \
  -p 3000:3000 \
  -e DATABASE_URL="postgres://user:pass@host:5432/db" \
  -e PGRST_DB_ANON_ROLE="web_anon" \
  -e PGRST_JWT_SECRET="your-secret-key" \
  postrust/postrust:latest
```

### Building Your Own Image (with Custom Routes)

If you've added custom routes in `crates/postrust-server/src/custom.rs`, build your own image:

```dockerfile
# Dockerfile
FROM rust:1.83-bookworm as builder

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build with admin-ui feature (optional)
RUN cargo build --release -p postrust-server --features admin-ui

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y libssl3 ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/postrust /usr/local/bin/

# Default environment
ENV PGRST_SERVER_HOST=0.0.0.0
ENV PGRST_SERVER_PORT=3000
ENV PGRST_DB_POOL_SIZE=10

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3000/_/health || exit 1

CMD ["postrust"]
```

Build and run:

```bash
# Build the image
docker build -t my-postrust:latest .

# Run with environment file
docker run -d \
  --name postrust \
  -p 3000:3000 \
  --env-file .env \
  my-postrust:latest

# Or with inline environment variables
docker run -d \
  --name postrust \
  -p 3000:3000 \
  -e DATABASE_URL="postgres://user:pass@host:5432/db" \
  -e PGRST_DB_ANON_ROLE="web_anon" \
  -e PGRST_JWT_SECRET="your-secret-key" \
  -e STRIPE_WEBHOOK_SECRET="whsec_..." \
  my-postrust:latest
```

### Multi-Architecture Build

Build for multiple platforms (useful for deploying to ARM64 servers like AWS Graviton):

```bash
# Create builder for multi-arch
docker buildx create --name mybuilder --use

# Build and push for amd64 and arm64
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -t your-registry/postrust:latest \
  --push .
```

### Docker Compose

```yaml
version: '3.8'

services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_PASSWORD: postgres
      POSTGRES_DB: app
    volumes:
      - postgres_data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 5s
      timeout: 5s
      retries: 5

  postrust:
    image: postrust/postrust:latest
    depends_on:
      postgres:
        condition: service_healthy
    environment:
      DATABASE_URL: postgres://postgres:postgres@postgres:5432/app
      PGRST_DB_ANON_ROLE: web_anon
      PGRST_JWT_SECRET: ${JWT_SECRET}
    ports:
      - "3000:3000"

volumes:
  postgres_data:
```

## AWS Lambda

### Prerequisites

```bash
# Install cargo-lambda
cargo install cargo-lambda
```

### Building

```bash
# Build for Lambda
cargo lambda build --release -p postrust-lambda

# Output: target/lambda/postrust-lambda/bootstrap
```

### AWS SAM Deployment

`template.yaml`:

```yaml
AWSTemplateFormatVersion: '2010-09-09'
Transform: AWS::Serverless-2016-10-31

Parameters:
  DatabaseUrl:
    Type: String
    NoEcho: true
  JwtSecret:
    Type: String
    NoEcho: true

Resources:
  PostrustFunction:
    Type: AWS::Serverless::Function
    Properties:
      Handler: bootstrap
      Runtime: provided.al2023
      CodeUri: target/lambda/postrust-lambda/
      MemorySize: 256
      Timeout: 30
      Environment:
        Variables:
          DATABASE_URL: !Ref DatabaseUrl
          PGRST_DB_ANON_ROLE: web_anon
          PGRST_JWT_SECRET: !Ref JwtSecret
      VpcConfig:
        SecurityGroupIds:
          - !Ref LambdaSecurityGroup
        SubnetIds:
          - !Ref PrivateSubnet1
          - !Ref PrivateSubnet2
      Events:
        Api:
          Type: HttpApi
          Properties:
            Path: /{proxy+}
            Method: ANY

Outputs:
  ApiUrl:
    Value: !Sub "https://${ServerlessHttpApi}.execute-api.${AWS::Region}.amazonaws.com"
```

Deploy:

```bash
sam build
sam deploy --guided
```

### Serverless Framework

`serverless.yml`:

```yaml
service: postrust-api

provider:
  name: aws
  runtime: provided.al2023
  region: us-east-1
  memorySize: 256
  timeout: 30
  environment:
    DATABASE_URL: ${ssm:/postrust/database-url}
    PGRST_DB_ANON_ROLE: web_anon
    PGRST_JWT_SECRET: ${ssm:/postrust/jwt-secret}
  vpc:
    securityGroupIds:
      - sg-xxxxxxxxx
    subnetIds:
      - subnet-xxxxxxxx
      - subnet-yyyyyyyy

package:
  artifact: target/lambda/postrust-lambda/bootstrap.zip

functions:
  api:
    handler: bootstrap
    events:
      - httpApi:
          path: /{proxy+}
          method: ANY
```

Deploy:

```bash
serverless deploy
```

### Lambda Best Practices

1. **Connection Pooling**: Use a small pool size (1-2 connections)
   ```bash
   PGRST_DB_POOL_SIZE=1
   ```

2. **VPC Configuration**: Place Lambda in the same VPC as RDS

3. **RDS Proxy**: Use RDS Proxy for connection pooling at scale

4. **Provisioned Concurrency**: Reduce cold starts for critical APIs

5. **Memory**: 256-512 MB is usually sufficient

## Kubernetes

### Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: postrust
spec:
  replicas: 3
  selector:
    matchLabels:
      app: postrust
  template:
    metadata:
      labels:
        app: postrust
    spec:
      containers:
      - name: postrust
        image: postrust/postrust:latest
        ports:
        - containerPort: 3000
        env:
        - name: DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: postrust-secrets
              key: database-url
        - name: PGRST_JWT_SECRET
          valueFrom:
            secretKeyRef:
              name: postrust-secrets
              key: jwt-secret
        - name: PGRST_DB_ANON_ROLE
          value: "web_anon"
        resources:
          requests:
            memory: "64Mi"
            cpu: "100m"
          limits:
            memory: "256Mi"
            cpu: "500m"
        livenessProbe:
          httpGet:
            path: /
            port: 3000
          initialDelaySeconds: 5
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /
            port: 3000
          initialDelaySeconds: 5
          periodSeconds: 5
```

### Service

```yaml
apiVersion: v1
kind: Service
metadata:
  name: postrust
spec:
  selector:
    app: postrust
  ports:
  - port: 80
    targetPort: 3000
  type: ClusterIP
```

### Ingress

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: postrust
  annotations:
    kubernetes.io/ingress.class: nginx
    cert-manager.io/cluster-issuer: letsencrypt-prod
spec:
  tls:
  - hosts:
    - api.example.com
    secretName: postrust-tls
  rules:
  - host: api.example.com
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: postrust
            port:
              number: 80
```

### Helm Chart

```bash
# Install
helm install postrust ./charts/postrust \
  --set database.url="postgres://..." \
  --set auth.jwtSecret="..." \
  --set replicas=3

# Upgrade
helm upgrade postrust ./charts/postrust --reuse-values
```

## Cloudflare Workers

> Note: Full Workers support requires Cloudflare Hyperdrive for database connections.

### Building

```bash
# Install wrangler
npm install -g wrangler

# Build
cargo build --release --target wasm32-unknown-unknown -p postrust-worker
```

### wrangler.toml

```toml
name = "postrust-api"
main = "build/worker.js"
compatibility_date = "2024-01-01"

[vars]
PGRST_DB_SCHEMAS = "public"
PGRST_DB_ANON_ROLE = "web_anon"

[[hyperdrive]]
binding = "DB"
id = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
```

### Deploy

```bash
wrangler deploy
```

## Reverse Proxy

### Nginx

```nginx
upstream postrust {
    server 127.0.0.1:3000;
    keepalive 32;
}

server {
    listen 443 ssl http2;
    server_name api.example.com;

    ssl_certificate /etc/ssl/certs/api.example.com.crt;
    ssl_certificate_key /etc/ssl/private/api.example.com.key;

    location / {
        proxy_pass http://postrust;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Connection "";

        # Timeouts
        proxy_connect_timeout 60s;
        proxy_send_timeout 60s;
        proxy_read_timeout 60s;
    }
}
```

### Caddy

```
api.example.com {
    reverse_proxy localhost:3000
}
```

## Monitoring

### Health Check Endpoint

```bash
curl http://localhost:3000/
# Returns OpenAPI spec if healthy
```

### Prometheus Metrics

Coming soon: `/metrics` endpoint for Prometheus scraping.

### Logging

Configure structured JSON logging:

```bash
PGRST_LOG_LEVEL=info
RUST_LOG=postrust=info
```

Output:
```json
{"timestamp":"2024-01-15T10:30:00Z","level":"INFO","message":"Request completed","method":"GET","path":"/users","status":200,"duration_ms":15}
```

## Fly.io

Fly.io provides easy deployment for Rust applications with global distribution.

### fly.toml

```toml
app = "my-postrust-api"
primary_region = "iad"

[build]
  dockerfile = "Dockerfile"

[env]
  PGRST_SERVER_HOST = "0.0.0.0"
  PGRST_SERVER_PORT = "8080"
  PGRST_DB_ANON_ROLE = "web_anon"
  PGRST_DB_SCHEMAS = "public"

[http_service]
  internal_port = 8080
  force_https = true
  auto_stop_machines = true
  auto_start_machines = true
  min_machines_running = 1

  [http_service.concurrency]
    type = "requests"
    hard_limit = 250
    soft_limit = 200

[[services]]
  protocol = "tcp"
  internal_port = 8080

  [[services.ports]]
    port = 80
    handlers = ["http"]

  [[services.ports]]
    port = 443
    handlers = ["tls", "http"]

  [[services.http_checks]]
    interval = "10s"
    timeout = "2s"
    path = "/_/health"
```

### Deploy

```bash
# Install Fly CLI
curl -L https://fly.io/install.sh | sh

# Login
fly auth login

# Launch (first time)
fly launch

# Set secrets
fly secrets set DATABASE_URL="postgres://..."
fly secrets set PGRST_JWT_SECRET="..."
fly secrets set STRIPE_WEBHOOK_SECRET="whsec_..."

# Deploy
fly deploy

# Check logs
fly logs

# Scale
fly scale count 3
```

### Connect to Fly Postgres

```bash
# Create Fly Postgres cluster
fly postgres create --name my-postrust-db

# Attach to app
fly postgres attach my-postrust-db

# DATABASE_URL is automatically set
```

## Railway

Railway provides simple Git-based deployments.

### railway.json

```json
{
  "$schema": "https://railway.app/railway.schema.json",
  "build": {
    "builder": "DOCKERFILE",
    "dockerfilePath": "Dockerfile"
  },
  "deploy": {
    "startCommand": "postrust",
    "healthcheckPath": "/_/health",
    "healthcheckTimeout": 30,
    "restartPolicyType": "ON_FAILURE",
    "restartPolicyMaxRetries": 3
  }
}
```

### Deploy

```bash
# Install Railway CLI
npm install -g @railway/cli

# Login
railway login

# Initialize project
railway init

# Link to existing project (if any)
railway link

# Add environment variables
railway variables set DATABASE_URL="postgres://..."
railway variables set PGRST_JWT_SECRET="..."

# Deploy
railway up

# View logs
railway logs
```

### Railway with Postgres

```bash
# Add Postgres plugin
railway add -p postgresql

# Environment variable is automatically set
# DATABASE_URL=postgres://...
```

## Render

### render.yaml (Blueprint)

```yaml
services:
  - type: web
    name: postrust-api
    env: docker
    dockerfilePath: ./Dockerfile
    dockerContext: .
    healthCheckPath: /_/health
    envVars:
      - key: DATABASE_URL
        fromDatabase:
          name: postrust-db
          property: connectionString
      - key: PGRST_JWT_SECRET
        sync: false
      - key: PGRST_DB_ANON_ROLE
        value: web_anon
      - key: PGRST_SERVER_PORT
        value: 10000
      - key: STRIPE_WEBHOOK_SECRET
        sync: false
    autoDeploy: true

databases:
  - name: postrust-db
    databaseName: postrust
    user: postrust
    plan: starter
```

### Deploy

1. Push `render.yaml` to your repository
2. Go to Render Dashboard → Blueprints
3. Connect your repository
4. Deploy

## DigitalOcean App Platform

### app.yaml

```yaml
name: postrust-api
region: nyc
services:
  - name: api
    dockerfile_path: Dockerfile
    source_dir: /
    http_port: 3000
    instance_count: 1
    instance_size_slug: basic-xxs
    health_check:
      http_path: /_/health
    envs:
      - key: DATABASE_URL
        scope: RUN_TIME
        value: ${db.DATABASE_URL}
      - key: PGRST_JWT_SECRET
        scope: RUN_TIME
        type: SECRET
      - key: PGRST_DB_ANON_ROLE
        value: web_anon

databases:
  - name: db
    engine: PG
    version: "16"
    size: db-s-dev-database
```

### Deploy

```bash
# Install doctl
brew install doctl

# Authenticate
doctl auth init

# Create app from spec
doctl apps create --spec app.yaml

# Update app
doctl apps update <app-id> --spec app.yaml
```

## CI/CD Pipelines

### GitHub Actions

```yaml
# .github/workflows/deploy.yml
name: Deploy

on:
  push:
    branches: [main]

env:
  REGISTRY: ghcr.io
  IMAGE_NAME: ${{ github.repository }}

jobs:
  build-and-push:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write

    steps:
      - uses: actions/checkout@v4

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3

      - name: Login to Container Registry
        uses: docker/login-action@v3
        with:
          registry: ${{ env.REGISTRY }}
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Build and push
        uses: docker/build-push-action@v5
        with:
          context: .
          push: true
          tags: ${{ env.REGISTRY }}/${{ env.IMAGE_NAME }}:latest
          cache-from: type=gha
          cache-to: type=gha,mode=max

  deploy:
    needs: build-and-push
    runs-on: ubuntu-latest
    steps:
      - name: Deploy to Fly.io
        uses: superfly/flyctl-actions/setup-flyctl@master
      - run: flyctl deploy --remote-only
        env:
          FLY_API_TOKEN: ${{ secrets.FLY_API_TOKEN }}
```

### GitLab CI

```yaml
# .gitlab-ci.yml
stages:
  - build
  - deploy

variables:
  DOCKER_IMAGE: $CI_REGISTRY_IMAGE:$CI_COMMIT_SHA

build:
  stage: build
  image: docker:24
  services:
    - docker:24-dind
  script:
    - docker login -u $CI_REGISTRY_USER -p $CI_REGISTRY_PASSWORD $CI_REGISTRY
    - docker build -t $DOCKER_IMAGE .
    - docker push $DOCKER_IMAGE
  only:
    - main

deploy:
  stage: deploy
  image: alpine:latest
  before_script:
    - apk add --no-cache curl
    - curl -L https://fly.io/install.sh | sh
  script:
    - flyctl deploy --image $DOCKER_IMAGE
  environment:
    name: production
  only:
    - main
```

## Production Checklist

### Security

- [ ] Use HTTPS/TLS in production
- [ ] Set strong `PGRST_JWT_SECRET` (min 32 characters)
- [ ] Configure proper CORS settings
- [ ] Enable Row Level Security (RLS) on all tables
- [ ] Use connection pooling (PgBouncer or built-in)
- [ ] Set appropriate `PGRST_DB_MAX_ROWS` limit
- [ ] Validate webhook signatures

### Performance

- [ ] Set appropriate `PGRST_DB_POOL_SIZE` (10-50 typically)
- [ ] Enable response compression
- [ ] Configure CDN for static responses
- [ ] Set up database indexes
- [ ] Monitor slow queries

### Reliability

- [ ] Configure health checks (`/_/health`, `/_/ready`)
- [ ] Set up automated restarts
- [ ] Configure log aggregation
- [ ] Set up alerts for errors/downtime
- [ ] Enable database backups
- [ ] Test disaster recovery

### Monitoring

- [ ] Application logs → CloudWatch/Datadog/Loki
- [ ] Database metrics → RDS Insights/pganalyze
- [ ] HTTP metrics → Prometheus/Grafana
- [ ] Error tracking → Sentry
- [ ] Uptime monitoring → UptimeRobot/Pingdom
