# Build the wasm package and the web client, then the server, then ship one
# small image that serves both. The server's API, WebSocket included, lives
# under /v1; the web client is static files for everything else
# (THRUM_WEB_DIR).

FROM rust:1.91.0-slim-bookworm AS wasm
WORKDIR /src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install wasm-bindgen-cli --version 0.2.130
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY .cargo .cargo
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p thrum-wasm --target wasm32-unknown-unknown \
    && wasm-bindgen --target web --out-dir /out \
         target/wasm32-unknown-unknown/release/thrum_wasm.wasm

FROM node:22-alpine AS web
WORKDIR /src/web
RUN --mount=type=bind,source=web/package.json,target=package.json \
    --mount=type=bind,source=web/package-lock.json,target=package-lock.json \
    --mount=type=cache,target=/root/.npm \
    npm ci
COPY web ./
COPY --link --from=wasm /out/thrum_wasm.js /out/thrum_wasm.d.ts /out/thrum_wasm_bg.wasm /out/thrum_wasm_bg.wasm.d.ts ./src/wasm/
RUN npm run build

FROM rust:1.91.0-slim-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY .cargo .cargo
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p thrum-server \
    && cp target/release/thrum-server /usr/local/bin/thrum-server

FROM debian:bookworm-slim AS runtime
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates passwd util-linux \
    && groupadd --system --gid 10001 thrum \
    && useradd --system --uid 10001 --gid thrum --home-dir /var/lib/thrum --create-home thrum \
    && mkdir -p /data \
    && chown thrum:thrum /data \
    && find / -xdev -type f -perm /6000 -exec chmod a-s {} +
COPY --link --from=server /usr/local/bin/thrum-server /usr/local/bin/thrum-server
COPY --link --from=web /src/web/dist /usr/share/thrum/web
COPY --link docker/entrypoint.sh /usr/local/bin/thrum-entrypoint
ENV THRUM_BIND=0.0.0.0 \
    THRUM_IMAGE_BIND=0.0.0.0 \
    THRUM_DB=/data/thrum.sqlite \
    THRUM_PORT=8080 \
    THRUM_WEB_DIR=/usr/share/thrum/web \
    THRUM_BLOB_DIR=/data/blobs
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
    CMD ["thrum-server", "healthcheck"]
ENTRYPOINT ["thrum-entrypoint"]
