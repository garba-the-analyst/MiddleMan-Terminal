# VOLUME 0 — Executive Overview, DevOps Strategy & Architecture Specs

**Version:** 2.4.0 · **Owner:** Platform/DevOps · **Scope:** Everything that runs on the VPS.

---

## 1. Architectural Overview & Technical Scope

MiddleMan ships as five containers on a single 1 vCPU / 1 GB RAM VPS (Hetzner CX22 or
DigitalOcean basic droplet). There is no Kubernetes, no service mesh; the only external managed
dependencies are Neon PostgreSQL and Cloudinary (gift-card image CDN). Caddy terminates TLS and
routes; Redis buffers all WhatsApp traffic; the Rust core does all business logic.

```
Internet --> :80/:443 Caddy --+--> admin-dashboard:80   (Vue SPA)
                              +--> mm-api:3000           (/api/v1/*)
                              +--> wa-bridge:3001        (/bridge/* — QR pairing, health)

wa-bridge --XADD--> redis <--XREADGROUP-- mm-api workers
wa-bridge <--HTTP POST /bridge/send-message-- mm-api (outbound, typing-simulated)
mm-api ----------------------------------------> Neon PostgreSQL (TLS)
mm-api / wa-bridge ----------------------------> Gemini, Flutterwave, Yellow Card, Jupiter, GoPlus
```

### Container Resource Allocation Matrix (hard law — do not exceed)

| Container         | CPU Limit | RAM Soft Limit | RAM Hard Limit | Base Image         |
|-------------------|-----------|----------------|----------------|--------------------|
| `wa-bridge`       | 0.25      | 150 MB         | 220 MB         | `node:20-alpine`   |
| `mm-api`          | 0.40      | 120 MB         | 180 MB         | `scratch` (static) |
| `admin-dashboard` | 0.05      | 20 MB          | 40 MB          | `nginx:alpine`     |
| `redis`           | 0.10      | 40 MB          | 80 MB          | `redis:7-alpine`   |
| `caddy`           | 0.10      | 30 MB          | 60 MB          | `caddy:alpine`     |
| Host overhead     | ~0.10     | —              | ~420 MB        | Linux kernel+sshd  |
| **TOTAL**         | **1.00**  | **360 MB**     | **<=1000 MB**  | —                  |

PostgreSQL is **not** self-hosted in production (Neon). A local `postgres` service exists only
for development via `docker-compose.override.yml`.

## 2. Mathematical Formulation — Memory Budget Guard

Per-container RSS must satisfy at all times `t`:

```
RSS_c(t) <= HardLimit_c            for every container c   (cgroup OOM boundary)
sum_c SoftLimit_c = 360 MB  <= 640 MB usable             (>=360 MB headroom for page cache/kernel)
```

Alert rule (wired in Vol Eta): if any container exceeds 90% of its soft limit for 3 consecutive
60 s samples, emit SMTP + WhatsApp alert to the ops number.

## 3. Complete Implementation

### 3.1 `docker-compose.yml` (production baseline)

```yaml
name: middleman

x-logging: &default-logging
  driver: json-file
  options:
    max-size: "10m"
    max-file: "3"

services:
  redis:
    image: redis:7-alpine
    command: >
      redis-server
      --save 60 1
      --appendonly yes
      --appendfsync everysec
      --loglevel warning
      --maxmemory 64mb
      --maxmemory-policy volatile-lru
    restart: always
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 10s
      timeout: 3s
      retries: 5
    volumes:
      - redis_data:/data
    logging: *default-logging
    deploy:
      resources:
        limits: { cpus: "0.10", memory: 80M }
        reservations: { memory: 40M }
    networks: [mm-internal]

  wa-bridge:
    build:
      context: ./apps/wa-bridge
      dockerfile: Dockerfile
    restart: always
    environment:
      - NODE_ENV=production
      - REDIS_URL=redis://redis:6379
      - PORT=3001
      - INTERNAL_API_SECRET=${INTERNAL_API_SECRET}
      - MM_API_URL=http://mm-api:3000
      - CLOUDINARY_CLOUD_NAME=${CLOUDINARY_CLOUD_NAME}
      - CLOUDINARY_API_KEY=${CLOUDINARY_API_KEY}
      - CLOUDINARY_API_SECRET=${CLOUDINARY_API_SECRET}
    volumes:
      - wa_sessions:/app/auth_info_baileys
    healthcheck:
      test: ["CMD", "wget", "-qO-", "http://127.0.0.1:3001/bridge/health"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 20s
    logging: *default-logging
    depends_on:
      redis: { condition: service_healthy }
    deploy:
      resources:
        limits: { cpus: "0.25", memory: 220M }
        reservations: { memory: 150M }
    networks: [mm-internal]

  mm-api:
    build:
      context: .
      dockerfile: crates/mm-api/Dockerfile
    restart: always
    environment:
      - DATABASE_URL=${DATABASE_URL}
      - REDIS_URL=redis://redis:6379
      - MM_MASTER_KEY=${MM_MASTER_KEY}
      - JWT_SECRET=${JWT_SECRET}
      - INTERNAL_API_SECRET=${INTERNAL_API_SECRET}
      - WA_BRIDGE_URL=http://wa-bridge:3001
      - GEMINI_API_KEY=${GEMINI_API_KEY}
      - FLUTTERWAVE_SECRET_KEY=${FLUTTERWAVE_SECRET_KEY}
      - FLW_WEBHOOK_HASH=${FLW_WEBHOOK_HASH}
      - RUST_LOG=info
    healthcheck:
      test: ["CMD", "/mm-api", "--health"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 10s
    logging: *default-logging
    depends_on:
      redis: { condition: service_healthy }
    deploy:
      resources:
        limits: { cpus: "0.40", memory: 180M }
        reservations: { memory: 120M }
    networks: [mm-internal]

  admin-dashboard:
    build:
      context: ./apps/admin-dashboard
      dockerfile: Dockerfile
    restart: always
    environment:
      - VITE_API_BASE=/api/v1
    logging: *default-logging
    deploy:
      resources:
        limits: { cpus: "0.05", memory: 40M }
        reservations: { memory: 20M }
    networks: [mm-internal]

  caddy:
    image: caddy:2-alpine
    restart: always
    ports:
      - "80:80"
      - "443:443"
    environment:
      - DOMAIN=${DOMAIN}
      - ACME_EMAIL=${ACME_EMAIL}
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy_data:/data
      - caddy_config:/config
    logging: *default-logging
    depends_on:
      - admin-dashboard
      - mm-api
      - wa-bridge
    deploy:
      resources:
        limits: { cpus: "0.10", memory: 60M }
        reservations: { memory: 30M }
    networks: [mm-internal]

volumes:
  redis_data:
  wa_sessions:
  caddy_data:
  caddy_config:

networks:
  mm-internal:
    driver: bridge
```

### 3.2 Development override — `docker-compose.override.yml`

```yaml
services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: mm_user
      POSTGRES_PASSWORD: mm_password
      POSTGRES_DB: middleman_db
    ports: ["5433:5432"]
    volumes:
      - pgdata:/var/lib/postgresql/data
    networks: [mm-internal]

volumes:
  pgdata:
```

### 3.3 Rust release image — `crates/mm-api/Dockerfile`

Static musl binary on `scratch` (~8 MB, zero shell attack surface). The binary implements a
`--health` mode (exit 0 once DB pool + Redis respond) because scratch has no shell/wget.

```dockerfile
FROM rust:1.82-alpine AS builder
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --release -p mm-api --target x86_64-unknown-linux-musl

FROM scratch
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/mm-api /mm-api
COPY --from=builder /build/migrations /migrations
EXPOSE 3000
ENTRYPOINT ["/mm-api"]
```

For arm64 VPS swap target to `aarch64-unknown-linux-musl`.

### 3.4 Bridge image — `apps/wa-bridge/Dockerfile`

```dockerfile
FROM node:20-alpine AS deps
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev

FROM node:20-alpine AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY tsconfig.json ./
COPY src ./src
RUN npx tsc -p tsconfig.json

FROM node:20-alpine
WORKDIR /app
ENV NODE_ENV=production
COPY --from=deps /app/node_modules ./node_modules
COPY --from=build /app/dist ./dist
EXPOSE 3001
CMD ["node", "dist/index.js"]
```

### 3.5 Dashboard image — `apps/admin-dashboard/Dockerfile`

```dockerfile
FROM node:20-alpine AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
RUN npm run build

FROM nginx:1.27-alpine
COPY deploy/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html
EXPOSE 80
```

`apps/admin-dashboard/deploy/nginx.conf`:

```nginx
server {
    listen 80;
    root /usr/share/nginx/html;
    index index.html;
    gzip on;
    gzip_types text/css application/javascript application/json image/svg+xml;

    location /assets/ {
        expires 30d;
        add_header Cache-Control "public, immutable";
    }

    location / {
        try_files $uri $uri/ /index.html;
    }
}
```

### 3.6 Edge routing — `Caddyfile`

```caddyfile
{
    email {$ACME_EMAIL}
}

{$DOMAIN} {
    encode zstd gzip

    handle_path /api/v1/* {
        reverse_proxy mm-api:3000
    }

    handle /api/v1/ws* {
        reverse_proxy mm-api:3000
    }

    handle_path /bridge/* {
        reverse_proxy wa-bridge:3001
    }

    handle {
        reverse_proxy admin-dashboard:80
    }

    header {
        Strict-Transport-Security "max-age=31536000; includeSubDomains; preload"
        X-Content-Type-Options nosniff
        X-Frame-Options DENY
        Referrer-Policy strict-origin-when-cross-origin
        -Server
    }
}
```

Note: `handle_path` strips the prefix, so `mm-api` routes stay unprefixed internally
(`/wa-webhook`, `/admin/...`). The bridge control plane is additionally guarded by
`X-Internal-Secret` at the application layer (Vol 3).

### 3.7 VPS hardening runbook — `deploy/provision.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail

apt-get update && apt-get upgrade -y
apt-get install -y ufw fail2ban unattended-upgrades htop jq ca-certificates curl gnupg

sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
sed -i 's/^#\?PermitRootLogin.*/PermitRootLogin prohibit-password/' /etc/ssh/sshd_config
systemctl restart ssh

ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp
ufw allow 80/tcp
ufw allow 443/tcp
ufw --force enable

cat > /etc/fail2ban/jail.local <<'CFG'
[sshd]
enabled = true
maxretry = 5
bantime = 1h
findtime = 10m
CFG
systemctl enable --now fail2ban

install -d -m 700 -o root -g root /opt/middleman/secrets
curl -fsSL https://get.docker.com | sh
echo "Provisioning complete. Deploy with: cd /opt/middleman && docker compose up -d --build"
```

### 3.8 Secrets discipline

- Secrets live in `/opt/middleman/.env` (chmod 600, root-owned), never in git.
- `MM_MASTER_KEY`: 64 hex chars (32 bytes). Generate: `openssl rand -hex 32`.
- Rotation: master key rotation is a Versioned re-encrypt job (see Vol Alpha §6); JWT and
  internal secrets rotate by redeploy.
- The Baileys session volume (`wa_sessions`) is a live credential store: back it up encrypted,
  never expose it via any HTTP route.

## 4. Data Schemas & Structural Interfaces (infra-level)

| Interface | Contract |
|-----------|----------|
| Caddy -> mm-api | HTTP/1.1 + WebSocket upgrade on `/api/v1/ws`, no request-body buffering for WS |
| Caddy -> wa-bridge | `/bridge/*` prefix stripped; JSON; requires `X-Internal-Secret` header |
| mm-api -> wa-bridge | `POST /bridge/send-message` `{recipient_jid, text}` |
| wa-bridge -> Redis | `XADD inbound:wa:events * payload <json>` |
| Compose health model | `restart: always` + healthcheck-gated dependencies; unhealthy containers are restarted by `autoheal`-equivalent cron (`docker restart $(docker ps -q --filter health=unhealthy)`) |

## 5. Error Handling, Retry & Failure Policies

| Failure | Detection | Response |
|---------|-----------|----------|
| Redis down | `redis-cli ping` fails / stream XADD errors | Bridge buffers last N=500 events in RAM ring, replays on reconnect; mm-api consumer exits, Docker restarts it |
| Neon unreachable | Pool acquire timeout 5 s | mm-api returns 503 on admin routes; WhatsApp consumers pause (backoff loop), never drop messages (Redis retains them) |
| Baileys logged out | `DisconnectReason.loggedOut` | Bridge emits QR pairing state; ops scans within 24 h; inbound messages queue in phone's offline queue meanwhile |
| Container OOM | cgroup kill | Docker restart policy revives with capped memory; if repeated >5/hour, Vol Eta alert fires |
| Disk pressure | `< 20%` free | Log rotation caps at ~180 MB total; Cloudinary keeps card images off-disk |

## 6. Verification Test Cases & Command Sequences

```bash
# V0-T1: full stack boots healthy
docker compose up -d --build
docker compose ps                       # all services Up (healthy)

# V0-T2: memory envelope respected under load
docker stats --no-stream                # sum of MEM USAGE must be < 900 MB
hey -z 30s -c 20 https://$DOMAIN/api/v1/admin/health
docker stats --no-stream                # re-check; mm-api < 180 MB

# V0-T3: TLS + routing
curl -sI https://$DOMAIN | head -1      # HTTP/2 200
curl -s https://$DOMAIN/api/v1/admin/health
curl -s -o /dev/null -w '%{http_code}' https://$DOMAIN/api/v1/admin/dashboard   # 401 without JWT

# V0-T4: persistence across restarts
docker compose restart redis
redis-cli -h 127.0.0.1 XLEN inbound:wa:events   # count preserved (AOF)

# V0-T5: edge lockdown from outside
nmap -p 3000,3001,5432 $DOMAIN          # all filtered/closed
```
