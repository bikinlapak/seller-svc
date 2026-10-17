# 04 — Rust stack

Versions are pinned in the workspace `Cargo.toml` under
`[workspace.dependencies]`; member crates inherit with `dep.workspace = true`,
so there is one place to bump a version.

## Already in the tree — keep

| Need        | Crate                                                    | Note                                                      |
| ----------- | -------------------------------------------------------- | --------------------------------------------------------- |
| HTTP server | `axum` 0.8                                               | Router per module, `Router<AppState>`, nested under `/v1` |
| Runtime     | `tokio` 1 (`full`)                                       |                                                           |
| Middleware  | `tower-http` 0.6 (`cors`, `trace`)                       | Add `compression`, `timeout`, `limit`                     |
| Serde       | `serde` 1, `serde_json` 1                                |                                                           |
| Errors      | `thiserror` 2                                            | `AppError` → stable JSON `code`                           |
| Logging     | `tracing`, `tracing-subscriber` (`env-filter`, `json`)   | JSON on, Cloud Logging parses it                          |
| HTTP client | `reqwest` 0.12, `default-features = false`, `rustls-tls` |                                                           |
| Env         | `dotenvy` 0.15                                           |                                                           |
| Test        | `tower` 0.5 (`util`), `http-body-util` 0.1               | Router tests via `oneshot`, no port                       |

**The TLS constraint is load-bearing.** `reqwest`'s `rustls-tls` feature uses
_ring_, which avoids dragging in `aws-lc-sys` and therefore cmake — the same
reason works-core does it. Every crate added below must keep
`cargo tree -i aws-lc-sys` empty. Check it in CI, not by memory: the usual way
this breaks is a transitive `rustls` default feature flipping to `aws-lc-rs`.

## To add

### Database

| Need     | Crate                                                                                          | Why this one                                                                                                     |
| -------- | ---------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Postgres | `sqlx` 0.8 — `postgres`, `runtime-tokio-rustls`, `macros`, `chrono`, `uuid`, `json`, `migrate` | Compile-time-checked SQL against a real schema, async, no ORM to fight. `sqlx::test` gives per-test transactions |
| Pooling  | `sqlx::PgPool`                                                                                 | Per-tenant pool, LRU-bounded                                                                                     |

Set `SQLX_OFFLINE=true` with a checked-in `.sqlx/` so CI builds without a
database.

### worksh.io GraphQL

| Need         | Crate                               | Why                                                                                                                                                 |
| ------------ | ----------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Typed client | `cynic` 3 + `cynic-codegen`         | Generates Rust structs from the works-server SDL and fails the build when a query stops matching the schema — the thing an untyped client cannot do |
| Fallback     | the existing `WorksClient::execute` | Keep for one-off and dynamic queries; the typed layer wraps it                                                                                      |

Vendor the SDL into `passtok-worksh/schema/works.graphql` and refresh it with a
`just schema` task, so a works-core change shows up as a compile error.

### Tokens and crypto

| Need          | Crate                                     | Why                                                                                                                                                                                                                    |
| ------------- | ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Action tokens | `jsonwebtoken` 9 with **EdDSA** (Ed25519) | A standard JWT with `jti`, `exp`, `aud` and custom claims (`tenant`, `suggestion`, `action`, `max_qty`); `jsonwebtoken` uses ring, so the aws-lc rule holds. Keys in Secret Manager, public key cached in the executor |
| Session ids   | `rand` 0.9 + `base64`                     | 256-bit opaque ids                                                                                                                                                                                                     |
| Web Push      | `web-push` 0.10                           | VAPID signing and payload encryption                                                                                                                                                                                   |
| Hashing       | `sha2`                                    | Idempotency keys, photo digests                                                                                                                                                                                        |

Prefer a JWT over a bespoke envelope purely for debuggability: a signed token
you can paste into a decoder is one you can reason about during a demo.

### Google Cloud

| Need             | Crate / approach                                                     | Why                                                                                                                                                                                  |
| ---------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| ADC tokens       | `gcp_auth` 0.12                                                      | Metadata server on Cloud Run, service-account key locally                                                                                                                            |
| Pub/Sub publish  | `google-cloud-pubsub` 0.3x                                           | Mature community crate; publish from the outbox publisher                                                                                                                            |
| Pub/Sub consume  | **Push subscriptions**, verified with `jsonwebtoken` + Google's JWKS | No subscriber crate needed; Cloud Run scales to zero                                                                                                                                 |
| Cloud Storage    | `google-cloud-storage` 0.2x                                          | V4 signed URLs for upload and receipt download                                                                                                                                       |
| BigQuery         | `gcp-bigquery-client` 0.2x                                           | Query + load jobs from the extractor and risk engine                                                                                                                                 |
| Vertex AI Gemini | **REST via `reqwest`** + `gcp_auth`                                  | No official or trustworthy Rust SDK. A thin `passtok_core::gemini` module: `generate_content`, `responseSchema` for structured output, inline image parts, token and cost accounting |
| Secret Manager   | `google-cloud-secretmanager` or REST                                 | Read at boot, cache in memory, no hot reload for v1                                                                                                                                  |

### Everything else

| Need                         | Crate                                                            | Why                                                                                                                                              |
| ---------------------------- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| Redis                        | `redis` 0.27 (`tokio-comp`, `connection-manager`)                | SSE fan-out pub/sub, rate limits, session store, idempotency cache                                                                               |
| SSE streams                  | `tokio-stream` (`BroadcastStream`) + `axum::response::Sse`       |                                                                                                                                                  |
| IDs                          | `ulid` 1                                                         | Sortable, time-prefixed; matches the plan's choice                                                                                               |
| Money                        | newtype `Idr(i64)` in `passtok-core`                             | IDR has no minor unit in practice; integer rupiah, `Display` with thousands separators. No float anywhere near money                             |
| Dates                        | `chrono` 0.4 + `chrono-tz`                                       | `Asia/Jakarta` for every merchant-facing boundary (a "day" in a report is a WIB day)                                                             |
| Event types from JSON Schema | `typify` (build script)                                          | The Rust equivalent of the plan's "generated Go types"                                                                                           |
| Schema validation            | `jsonschema` 0.26                                                | Validate outgoing events in debug and in tests                                                                                                   |
| Validation                   | `garde` 0.20                                                     | Derive request validation                                                                                                                        |
| Templating                   | `minijinja` 2                                                    | WhatsApp templates, push copy, receipt HTML                                                                                                      |
| PDF receipts                 | `printpdf` 0.7 for v1                                            | Pure Rust, enough for a donation receipt. If the layout outgrows it, switch to headless Chromium on Cloud Run — decide when it hurts, not before |
| CSV import                   | `csv` 1                                                          | Streaming, with a row-level error report                                                                                                         |
| Images                       | `image` 0.25, `kamadak-exif` 0.6                                 | Server-side EXIF strip as a second line of defence, thumbnailing                                                                                 |
| Rate limiting                | `tower_governor` 0.5 for per-IP, Redis bucket for per-session    |                                                                                                                                                  |
| Tracing export               | `opentelemetry` + `opentelemetry-otlp` + `tracing-opentelemetry` | Traces to Cloud Trace via OTLP; `trace_id` propagated into the event envelope                                                                    |
| Mock HTTP                    | `wiremock` 0.6 (dev)                                             | Fake works-server and Vertex in integration tests                                                                                                |
| Snapshot tests               | `insta` 1 (dev)                                                  | The plan's golden-file tests for reorder, expiry, sell-through, production, diversion                                                            |
| Property tests               | `proptest` 1 (dev)                                               | Pack rounding and expiry-cap arithmetic — the rules where an off-by-one costs real money                                                         |
| Fixtures                     | `fake` 2 (dev) + the `seed/` generator                           |                                                                                                                                                  |

## Container build

Multi-stage Dockerfile per crate, `cargo-chef` for dependency-layer caching:

```dockerfile
FROM rust:1.97-slim AS chef
# ... cargo chef prepare / cook, then
RUN cargo build --release -p ${CRATE}
FROM gcr.io/distroless/cc-debian12
COPY --from=builder /app/target/release/${CRATE} /app/${CRATE}
```

Distroless `cc` rather than static musl: `ring` and `sqlx` build cleanly against
glibc, and musl's allocator costs noticeably more on a connection-heavy service.
One `Dockerfile` with a `CRATE` build arg keeps six images from drifting.

## Toolchain and CI gates

- `rust-toolchain.toml` pinning the stable version and `clippy`, `rustfmt`.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test --workspace`, `cargo deny check` (licences + advisories).
- `cargo tree -i aws-lc-sys` must be empty — fail the build if it is not.
- `SQLX_OFFLINE=true` so CI needs no Postgres for the type-check.
- `just` recipes for the common loops (`just dev`, `just test`, `just schema`,
  `just seed`, `just migrate`).
