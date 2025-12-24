# Deployment

Deploy Postrust to various environments.

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

# Environment
Environment=DATABASE_URL=postgres://user:pass@localhost:5432/mydb
Environment=PGRST_DB_ANON_ROLE=web_anon
Environment=PGRST_JWT_SECRET=your-secret-key
Environment=PGRST_LOG_LEVEL=info

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

### Building Your Own Image

```dockerfile
FROM rust:1.75-bookworm as builder
WORKDIR /app
COPY . .
RUN cargo build --release -p postrust-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/postrust /usr/local/bin/
EXPOSE 3000
CMD ["postrust"]
```

Build and run:

```bash
docker build -t my-postrust .
docker run -d -p 3000:3000 --env-file .env my-postrust
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
