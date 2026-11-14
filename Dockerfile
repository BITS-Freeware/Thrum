# Build the web client, then the server, then ship one small image that
# serves both. The server's API lives under /v1 and /ws; the web client is
# static files for everything else (THRUM_WEB_DIR).

FROM node:22-alpine AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN npm run build

FROM rust:1-slim AS rust
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY .cargo .cargo
RUN cargo build --release -p thrum-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rust /src/target/release/thrum-server /usr/local/bin/thrum-server
COPY --from=web /src/web/dist /usr/share/thrum/web
ENV THRUM_DB=/data/thrum.sqlite \
    THRUM_PORT=8080 \
    THRUM_WEB_DIR=/usr/share/thrum/web \
    THRUM_BLOB_DIR=/data/blobs
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["thrum-server"]
