# syntax=docker/dockerfile:1
# Build on the target Linux platform; Node and Rust therefore use the same ABI.
ARG NODE_IMAGE=node:22-alpine
ARG RUST_IMAGE=rust:1.98.1-alpine
ARG RUNTIME_IMAGE=alpine:3.23

FROM ${NODE_IMAGE} AS node

FROM ${RUST_IMAGE} AS source
RUN apk add --no-cache build-base cmake pkgconf binutils file git
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s /usr/local/lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx
ENV CARGO_HOME=/root/.cargo
WORKDIR /src/everything-manual
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY config.example.toml price-catalog.example.toml ./
COPY .cargo/ .cargo/
COPY crates/ crates/
COPY xtask/ xtask/
COPY migrations/ migrations/
COPY contracts/ contracts/
COPY tests/fixtures/ tests/fixtures/
COPY apps/web/ apps/web/
ARG TARGETARCH
ARG CARGO_BUILD_JOBS=4
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS} \
    CARGO_PROFILE_DEV_DEBUG=0 \
    CARGO_PROFILE_TEST_DEBUG=0 \
    CARGO_INCREMENTAL=0

FROM source AS build
ARG TARGETARCH
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    --mount=type=cache,target=/root/.npm \
    --mount=type=cache,id=manual-target-${TARGETARCH},target=/src/everything-manual/target,sharing=locked \
    case "$TARGETARCH" in \
      amd64) target=x86_64-unknown-linux-musl ;; \
      arm64) target=aarch64-unknown-linux-musl ;; \
      *) echo 'Only linux/amd64 and linux/arm64 are supported.' >&2; exit 2 ;; \
    esac \
    && cargo xtask dist --target "$target" \
    && mkdir /out \
    && cp "dist/$target/"* /out/ \
    && cp target/debug/xtask /usr/local/bin/release-checker \
    && ! readelf -d /out/everything-manual | grep -q '(NEEDED)'

FROM source AS validation
ARG TARGETARCH
# Keep async UI query deadlines reliable on constrained/emulated CI runners.
# All tests and their assertion deadlines remain unchanged.
ENV VITEST_MAX_WORKERS=1
RUN apk add --no-cache su-exec \
    && addgroup -g 10001 qa \
    && adduser -D -H -u 10001 -G qa qa \
    && rustup component add --toolchain 1.98.1 rustfmt clippy \
    && mkdir -p /tmp/manual-validation \
    && chown 10001:10001 /tmp/manual-validation
# Permission-denial tests must run without root's ability to bypass Unix modes.
# Cache ownership changes only in this QA stage; later root builds can reuse it.
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    --mount=type=cache,target=/root/.npm \
    --mount=type=cache,id=manual-target-${TARGETARCH},target=/src/everything-manual/target,sharing=locked \
    chmod o+x /root \
    && mkdir -p /root/.cargo /root/.npm \
    && chown -R 10001:10001 /src/everything-manual /root/.cargo /root/.npm \
    && su-exec 10001:10001 env HOME=/tmp/manual-validation npm_config_cache=/root/.npm \
        sh -ec 'test "$(id -u)" = 10001; npm --prefix apps/web ci; cargo xtask check' \
    && cp target/debug/xtask /usr/local/bin/release-checker

FROM ${RUNTIME_IMAGE} AS runtime
RUN apk add --no-cache ca-certificates su-exec \
    && addgroup -g 10001 manual \
    && adduser -D -H -u 10001 -G manual manual \
    && mkdir -p /data /runtime-config /run/private /opt/everything-manual \
    && chown 10001:10001 /data /runtime-config /run/private \
    && chmod 700 /data /runtime-config /run/private
COPY --from=build /out/everything-manual /usr/local/bin/everything-manual
COPY --from=build /out/SHA256SUMS /out/licenses.json /out/build-info.json /out/dynamic-dependencies.txt /opt/everything-manual/
COPY --chmod=755 docker/entrypoint.sh /usr/local/bin/container-entrypoint
COPY docker/config.toml /etc/everything-manual/config.toml
ENV EM_DATA_DIR=/data \
    EM_LISTEN=127.0.0.1:8081 \
    EM_CONFIG=/etc/everything-manual/config.toml \
    EM_PROVIDER_OVERRIDES_DIR=/runtime-config \
    EM_SECRETS_BACKEND=file \
    EM_SECRETS_MASTER_KEY_FILE=/run/private/master.key
USER 10001:10001
WORKDIR /data
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=6 \
    CMD wget -q -T 2 -O /dev/null http://127.0.0.1:8081/api/v1/health/ready || exit 1
STOPSIGNAL SIGTERM
ENTRYPOINT ["container-entrypoint"]
CMD ["serve"]

# Copy a complete release directory without extracting it from the runtime image.
FROM scratch AS release
COPY --from=build /out/ /
