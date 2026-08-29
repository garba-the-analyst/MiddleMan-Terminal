# MiddleMan — Fly.io single-image (mm-api + wa-bridge)
FROM rust:bookworm AS rust-builder
WORKDIR /app
ENV SQLX_OFFLINE=true
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY .sqlx ./.sqlx
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --release -p mm-api --features tls

FROM node:20-bookworm AS node-builder
WORKDIR /app/apps/wa-bridge
COPY apps/wa-bridge/package*.json ./
RUN npm ci
COPY apps/wa-bridge ./
RUN npm run build

FROM node:20-bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=rust-builder /app/target/release/mm-api ./mm-api
COPY --from=node-builder /app/apps/wa-bridge/dist ./wa-bridge/dist
COPY --from=node-builder /app/apps/wa-bridge/node_modules ./wa-bridge/node_modules
COPY --from=node-builder /app/apps/wa-bridge/package.json ./wa-bridge/package.json
COPY migrations ./migrations
ENV NODE_ENV=production
EXPOSE 8080 3001
CMD sh -c "./mm-api & node wa-bridge/dist/index.js & wait"
