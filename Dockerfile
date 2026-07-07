# syntax=docker/dockerfile:1.19.0
# Dockerfile for the Rust Axum application.
#
# Intended deployment path:
#   Cloud Build -> Artifact Registry -> Cloud Run
#
# This intentionally uses a normal multi-stage container build instead of Cloud
# Run source buildpacks. Current Cloud Run automatic base image update flows are
# centered around Google buildpack base images, which is not the default path for
# this Rust service.

ARG RUST_IMAGE=rust:1.96.1-bookworm
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian13:nonroot
ARG VERSION=dev

FROM ${RUST_IMAGE} AS builder

WORKDIR /app

# Keep the builder compatible with crates that still link against OpenSSL while
# also ensuring CA roots are available for dependency downloads.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY . .

# The crate scaffold will provide Cargo.toml, Cargo.lock, and src/.
RUN cargo build --release --locked --bin axum-playground

FROM ${RUNTIME_IMAGE} AS runtime

ARG RUNTIME_IMAGE
ARG VERSION
LABEL org.opencontainers.image.base.name="${RUNTIME_IMAGE}" \
      org.opencontainers.image.version="${VERSION}"

COPY --from=builder --chmod=0555 /app/target/release/axum-playground /server

USER 65532:65532

# Cloud Run injects PORT and expects the ingress container to listen on 0.0.0.0.
ENV PORT=8080
EXPOSE 8080

ENTRYPOINT ["/server"]
