# Build toolchain for Quire: Rust + wasm32 target + trunk + wasm-bindgen, clang/lld/llvm (ring compiles C for
# wasm32 inside Inkboard's network crates), Node + pnpm for the web frontend.
# Nothing is compiled on the host: every build runs in this image with CPU/memory caps (see scripts/dev.sh).
# Build it with scripts/toolchain.sh, which uses the capped `quire-limited` buildx builder.
FROM node:24-trixie-slim AS node

FROM rust:1.99-trixie

RUN apt-get update \
 && apt-get install -y --no-install-recommends clang lld llvm \
 && rm -rf /var/lib/apt/lists/*

COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
 && npm install -g pnpm@11.24.0 \
 && npm cache clean --force

# Keep in step with the wasm-bindgen version in vendor/inkboard/Cargo.lock.
ARG TRUNK_VERSION=0.22.1
ARG WASM_BINDGEN_VERSION=0.2.129
ENV CARGO_BUILD_JOBS=6
RUN rustup target add wasm32-unknown-unknown \
 && cargo install --locked "trunk@${TRUNK_VERSION}" "wasm-bindgen-cli@${WASM_BINDGEN_VERSION}" \
 && rm -rf /usr/local/cargo/registry /usr/local/cargo/git \
 && chmod -R a+rwX /usr/local/cargo /usr/local/rustup

# clang is the C compiler for the wasm32 target (gcc has no WebAssembly backend).
ENV CC_wasm32_unknown_unknown=clang \
    AR_wasm32_unknown_unknown=llvm-ar
