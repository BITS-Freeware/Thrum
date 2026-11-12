# Build the web client, then the server, then ship one small image that
# serves both. The server's API lives under /v1 and /ws; the web client is
# static files for everything else (GEIST3_WEB_DIR).

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
RUN cargo build --release -p geist3-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rust /src/target/release/geist3-server /usr/local/bin/geist3-server
COPY --from=web /src/web/dist /usr/share/geist3/web
ENV GEIST3_DB=/data/geist3.sqlite \
    GEIST3_PORT=8080 \
    GEIST3_WEB_DIR=/usr/share/geist3/web \
    GEIST3_BLOB_DIR=/data/blobs
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["geist3-server"]
