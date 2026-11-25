# syntax=docker/dockerfile:1
#
# The portal's image: the server, with the frontend it serves.
#
#   docker build --tag status-list-portal .
#   docker run --env-file portal.env --publish 8080:8080 status-list-portal
#
# The image runs `portal server` as a user without privileges. Other commands, such as
# `portal approve-registration 42`, replace `server`. docs/deployment.md lists the settings.
# The base images are pinned by digest; Dependabot proposes new ones.

FROM --platform=$BUILDPLATFORM oven/bun:1.4.4-slim@sha256:1846adf22496bd711036aae8be147e57bdebbaea2e5264e3e0b5af1db187d9a4 AS frontend
WORKDIR /build
# The build writes into the mounted sources; those writes are dropped afterwards, so the
# output is copied out.
RUN --mount=type=bind,source=package.json,target=package.json \
    --mount=type=bind,source=bun.lock,target=bun.lock \
    --mount=type=bind,source=frontend,target=frontend,rw \
    --mount=type=cache,target=/root/.bun/install/cache \
    bun install --frozen-lockfile \
    && bun run build \
    && cp -R frontend/build /frontend

# Each image cross-compiles to its musl target, which it sets in CARGO_BUILD_TARGET.
FROM --platform=$BUILDPLATFORM blackdex/rust-musl:x86_64-musl@sha256:a0040208c797f80d7b3c0687eacac954cab160c536260bc70ded55844fb89ea1 AS backend-amd64
FROM --platform=$BUILDPLATFORM blackdex/rust-musl:aarch64-musl@sha256:adb8580729a9151ea39a9e4890ba05caed3e24514ab9da376d2b7c5fae37de54 AS backend-arm64
# jemalloc cannot probe the page size when cross-compiled. Built for 64 KiB pages, it runs on
# every arm64 kernel, whose pages may be 4, 16 or 64 KiB.
ENV JEMALLOC_SYS_WITH_LG_PAGE=16

FROM backend-${TARGETARCH} AS backend
ARG TARGETPLATFORM
WORKDIR /build
RUN --mount=type=bind,source=Cargo.toml,target=Cargo.toml \
    --mount=type=bind,source=Cargo.lock,target=Cargo.lock \
    --mount=type=bind,source=build.rs,target=build.rs \
    --mount=type=bind,source=migrations,target=migrations \
    --mount=type=bind,source=src,target=src \
    --mount=type=bind,source=templates,target=templates \
    --mount=type=cache,target=/root/.cargo/registry,id=cargo-registry-${TARGETPLATFORM} \
    --mount=type=cache,target=/build/target,id=portal-target-${TARGETPLATFORM} \
    cargo build --release --locked --bin portal \
    && cp "target/${CARGO_BUILD_TARGET}/release/portal" /portal

# Holds CA certificates and the nonroot user, and no libc: the binary needs none.
FROM gcr.io/distroless/static-debian13:nonroot@sha256:aafe993665bf6200db38a02c11a9351e782a84b6a45e7f33b6695638c88e3931
LABEL org.opencontainers.image.title="Status List Portal" \
      org.opencontainers.image.description="Developer portal for the Status List Server" \
      org.opencontainers.image.source="https://github.com/ADORSYS-GIS/status-list-portal" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"
COPY --from=backend /portal /usr/local/bin/portal
COPY --from=frontend /frontend /usr/share/status-list-portal/frontend
ENV APP_ENV=production \
    APP_LOG_FORMAT=json \
    APP_SERVER__BIND_ADDRESS=0.0.0.0:8080 \
    APP_SERVER__FRONTEND_DIR=/usr/share/status-list-portal/frontend
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/portal"]
CMD ["server"]
