# Fat toolchain base; the workflow always passes the real value via
# --build-arg BASE_IMAGE. Keep the default in sync with deploy.yml.
ARG BASE_IMAGE=rust:1.97-bookworm

FROM ${BASE_IMAGE} AS builder
WORKDIR /app
COPY . .
RUN --mount=type=cache,id=seller-svc-target,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release --locked && cp target/release/seller-svc /seller-svc

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /seller-svc /usr/local/bin/seller-svc
ENV SELLER_SVC_BIND_ADDR=0.0.0.0:8080 SELLER_SVC_LOG_JSON=true
EXPOSE 8080
USER nobody
CMD ["seller-svc"]
