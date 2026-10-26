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

FROM oven/bun:1.4.4-slim@sha256:1846adf22496bd711036aae8be147e57bdebbaea2e5264e3e0b5af1db187d9a4 AS frontend
WORKDIR /build
COPY package.json bun.lock ./
COPY frontend/package.json frontend/
RUN bun install --frozen-lockfile
COPY frontend/ frontend/
RUN bun run build

FROM rust:1.99-slim-trixie@sha256:1a939748a7cbab172de91a014e3804a97af1394a8c844faa67fce9401c4e40fb AS backend
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY migrations/ migrations/
COPY src/ src/
# The caches survive between builds; the binary is copied out of them.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked --bin portal \
    && cp target/release/portal /portal

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:3ee00291cf386ec52a552e6a858d555debd74776eb4bfa53fa3df2cd6677515e
LABEL org.opencontainers.image.title="Status List Portal" \
      org.opencontainers.image.description="Developer portal for the Status List Server" \
      org.opencontainers.image.source="https://github.com/ADORSYS-GIS/status-list-portal" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"
COPY --from=backend /portal /usr/local/bin/portal
COPY --from=frontend /build/frontend/build /usr/share/status-list-portal/frontend
ENV APP_ENV=production \
    APP_LOG_FORMAT=json \
    APP_SERVER__BIND_ADDRESS=0.0.0.0:8080 \
    APP_SERVER__FRONTEND_DIR=/usr/share/status-list-portal/frontend
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/portal"]
CMD ["server"]
