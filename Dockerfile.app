# Runtime base; the workflow/Makefile always passes the real value via
# --build-arg BASE_IMAGE. Keep the default in sync with deploy.yml.
# bikinlapak-base is shared across services: need a new runtime dependency?
# Add it to Dockerfile.base, bump the tag (v1.1, v2.0, ...), done.
ARG BASE_IMAGE=asia-southeast2-docker.pkg.dev/project-9567c06f-f9aa-4397-a2c/bikinlapak-staging/bikinlapak-base:v1.0

# ---- Stage 1: Rust build ----
FROM rust:1.97-bookworm AS builder
WORKDIR /app
COPY . .
RUN --mount=type=cache,id=seller-svc-target,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release --locked && cp target/release/seller-svc /seller-svc

# ---- Stage 2: thin runtime — no apt; the deps live in the base image ----
FROM ${BASE_IMAGE}
COPY --from=builder /seller-svc /usr/local/bin/seller-svc
ENV SELLER_SVC_BIND_ADDR=0.0.0.0:8080 SELLER_SVC_LOG_JSON=true
EXPOSE 8080
USER nobody
CMD ["seller-svc"]
