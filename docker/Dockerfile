# syntax=docker/dockerfile:1.12
# Built only through scripts/image.sh (target runtime) and scripts/toolchain.sh (target toolchain), which use the capped
# quire-limited builder. The three base images are pinned by index digest and binaryen by its release checksum (W0b,
# 2026-10-28; docs/ARCHITECTURE.md §13.2).
ARG RUST_IMAGE=rust:1.99.1-trixie@sha256:53766005847cbe02c3f59bfa49c1c4bb16695f2c9e5829c3d576a15406f5e733
ARG NODE_IMAGE=node:24-trixie-slim@sha256:89c5244bc22a9bc3287d89ebc7e49b9cb5bb84cd40e6175ac3421d5a9f68b95a
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian13:nonroot@sha256:3ee00291cf386ec52a552e6a858d555debd74776eb4bfa53fa3df2cd6677515e

FROM ${NODE_IMAGE} AS node

# Everything the image build needs. The official rust image installs the minimal profile: add clippy and rustfmt.
FROM ${RUST_IMAGE} AS toolchain-base
ARG BINARYEN_VERSION=131
ARG BINARYEN_SHA256=b5bf1f0eaf17c63ee588ff7a5954dc8f6ce2c26989051c66f24dfe9ece3e46db
# Keep in step with wasm-bindgen in vendor/inkboard/Cargo.lock.
ARG WASM_BINDGEN_VERSION=0.2.129
ENV CARGO_BUILD_JOBS=6 BINARYEN_CORES=6 RUSTUP_AUTO_INSTALL=0 \
    CC_wasm32_unknown_unknown=clang AR_wasm32_unknown_unknown=llvm-ar \
    CC_x86_64_unknown_linux_musl=musl-gcc \
    PATH=/opt/binaryen/bin:$PATH
# musl-tools: ring (under rustls) compiles C for the static CLI (D54).
RUN apt-get update && apt-get install -y --no-install-recommends clang lld llvm brotli musl-tools \
 && rm -rf /var/lib/apt/lists/*
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
 && npm install -g pnpm@11.24.0 && npm cache clean --force
RUN curl -fsSLo /tmp/binaryen.tgz "https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-x86_64-linux.tar.gz" \
 && echo "${BINARYEN_SHA256}  /tmp/binaryen.tgz" | sha256sum -c - \
 && mkdir -p /opt/binaryen && tar -xzf /tmp/binaryen.tgz -C /opt/binaryen --strip-components=1 && rm /tmp/binaryen.tgz
RUN rustup component add clippy rustfmt && rustup target add wasm32-unknown-unknown x86_64-unknown-linux-musl \
 && cargo install --locked "wasm-bindgen-cli@${WASM_BINDGEN_VERSION}" \
 && rm -rf /usr/local/cargo/registry /usr/local/cargo/git \
 && chmod -R a+rwX /usr/local/cargo /usr/local/rustup

# Dev image quire-toolchain:latest (scripts/toolchain.sh). trunk is only for Inkboard's own public page.
FROM toolchain-base AS toolchain
RUN cargo install --locked trunk@0.22.1 \
 && rm -rf /usr/local/cargo/registry /usr/local/cargo/git && chmod -R a+rwX /usr/local/cargo

FROM toolchain-base AS inkboard-web
ARG INKBOARD_BUILD=full
WORKDIR /src
COPY vendor/inkboard vendor/inkboard
COPY patches/inkboard patches/inkboard
COPY web/inkboard-frame web/inkboard-frame
COPY scripts/inkboard.sh scripts/inkboard.sh
RUN --mount=type=cache,id=quire-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=quire-inkboard-target,target=/cache/inkboard-target \
    if [ "$INKBOARD_BUILD" = stub ]; then scripts/inkboard.sh stub --out /out/inkboard; \
    else INKBOARD_TARGET_DIR=/cache/inkboard-target scripts/inkboard.sh build --out /out/inkboard; fi

FROM toolchain-base AS web
WORKDIR /src/web
COPY web/package.json web/pnpm-lock.yaml ./
RUN --mount=type=cache,id=quire-pnpm,target=/pnpm-store pnpm fetch --frozen-lockfile --store-dir /pnpm-store
COPY web/ ./
RUN --mount=type=cache,id=quire-pnpm,target=/pnpm-store \
    pnpm install --offline --frozen-lockfile --store-dir /pnpm-store && pnpm build \
 && find dist -type f \( -name '*.js' -o -name '*.css' -o -name '*.html' -o -name '*.svg' -o -name '*.json' \) \
      -exec brotli -q 11 -k {} \; -exec gzip -9 -k -n {} \;

FROM toolchain-base AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo .cargo
# inkboard-core inherits version, edition, license, rust-version and [lints] from Inkboard's workspace root, so that
# manifest is needed too. Cargo reads only its [workspace] tables; the other members are never loaded.
COPY vendor/inkboard/Cargo.toml vendor/inkboard/Cargo.toml
COPY vendor/inkboard/crates/inkboard-core vendor/inkboard/crates/inkboard-core
COPY crates crates
COPY fixtures fixtures
# These two COPYs order the server's rustc build after the wasm build: two rustc builds never overlap.
COPY --from=web /src/web/dist web/dist
COPY --from=inkboard-web /out/inkboard web/dist-inkboard
ARG QUIRE_COMMIT=unknown
RUN --mount=type=cache,id=quire-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=quire-server-target,target=/cache/target \
    QUIRE_COMMIT="$QUIRE_COMMIT" CARGO_TARGET_DIR=/cache/target cargo build --release --locked -p quire-server \
 && install -D /cache/target/release/quire-server /out/quire-server && mkdir -p /out/data

# The CLI (D54): a static musl binary, in the image and as a standalone download (scripts/cli-dist.sh). Cargo needs
# every workspace manifest, including inkboard-core's, to resolve the lockfile, but compiles only quire-cli's graph.
FROM toolchain-base AS cli-src
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo .cargo
COPY vendor/inkboard/Cargo.toml vendor/inkboard/Cargo.toml
COPY vendor/inkboard/crates/inkboard-core vendor/inkboard/crates/inkboard-core
COPY crates crates

# For the runtime image: this COPY orders the CLI's rustc build after the server's, so two rustc builds never overlap.
FROM cli-src AS cli
COPY --from=server /out/quire-server /tmp/order-after-server
RUN --mount=type=cache,id=quire-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=quire-cli-target,target=/cache/target \
    CARGO_TARGET_DIR=/cache/target cargo build --release --locked -p quire-cli --target x86_64-unknown-linux-musl \
 && install -D /cache/target/x86_64-unknown-linux-musl/release/quire /out/quire

# For scripts/cli-dist.sh (`--target cli-dist --output type=local`): the same build without the server.
FROM cli-src AS cli-alone
RUN --mount=type=cache,id=quire-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=quire-cli-target,target=/cache/target \
    CARGO_TARGET_DIR=/cache/target cargo build --release --locked -p quire-cli --target x86_64-unknown-linux-musl \
 && install -D /cache/target/x86_64-unknown-linux-musl/release/quire /out/quire
FROM scratch AS cli-dist
COPY --from=cli-alone /out/quire /quire-x86_64-unknown-linux-musl
# THIRD_PARTY.md covers every crate in the workspace lock (W7), so also the CLI's.
COPY LICENSE THIRD_PARTY.md /

FROM ${RUNTIME_IMAGE} AS runtime
COPY --from=server /out/quire-server /usr/local/bin/quire-server
COPY --from=cli /out/quire /usr/local/bin/quire
COPY --from=server --chown=65532:65532 /out/data /data
COPY LICENSE THIRD_PARTY.md /usr/share/doc/quire/
COPY vendor/inkboard/LICENSE-MIT vendor/inkboard/LICENSE-APACHE vendor/inkboard/THIRD_PARTY_LICENSES.md /usr/share/doc/quire/inkboard/
ENV QUIRE_DATA_DIR=/data QUIRE_LISTEN=0.0.0.0:8080 RUST_LOG=info MALLOC_ARENA_MAX=2
USER 65532:65532
VOLUME /data
EXPOSE 8080
# quire-server healthcheck derives its URL from QUIRE_LISTEN (port; 0.0.0.0 → 127.0.0.1). --start-interval lets
# `compose up --wait` see "healthy" within about a second. The CLI derives its default URL the same way (§7.6).
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --start-interval=1s --retries=3 \
  CMD ["/usr/local/bin/quire-server", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/quire-server"]
CMD ["serve"]
