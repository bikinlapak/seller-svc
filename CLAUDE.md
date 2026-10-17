# CLAUDE.md

## What this is

`seller-svc` is the Rust backend (axum 0.8, tokio) for the bikinlapak seller dashboard. Its frontend is `../seller-app`. Two features: **product management** and the **web builder** (online-store pages). Both are sourced from worksh.io, so this service **does not own a database for products or pages**. It calls the works-server GraphQL API (`src/works/`) with the seller's access token, and works-server enforces permissions and the tenant. Don't duplicate authorization rules here.

**Everything is in English**: code comments, identifiers, and the API
contract shared with `../seller-app`. This diverges deliberately from
`../comingsoon`, which is Indonesian — do not "fix" it back.

## Commands

```sh
cargo run                       # reads .env (optional), see .env.example
cargo test                      # unit tests (config) + tests/http.rs (router via tower oneshot)
cargo clippy --all-targets
cargo fmt
```

## Layout

- `src/lib.rs`: `AppState`, `build_router`, the CORS layer. Integration tests build the router from here without opening a port.
- `src/config.rs`: config comes from env vars prefixed `SELLER_SVC_`. `from_lookup` is kept separate so it can be tested.
- `src/error.rs`: `AppError` → JSON `{ "error": { code, message } }`. Internal and upstream errors only reach the log, never the client.
- `src/routes/`: `health` (liveness, doesn't touch works-server), plus
  `assistant`, `products` and `stock` under `/v1`. `builder` is still a 501 stub.
- `src/contract.rs`: the response shapes seller-app consumes, camelCase.
- `src/token.rs`: action tokens — Ed25519 JWT, single use, tenant and quantity
  cap. The only path by which a suggestion becomes a write.
- `src/seed.rs`: stand-in data until worksh.io and the `passtok` schema are wired.
- `infra/`: Terraform for both services (see `infra/README.md`).
- `src/works/`: `WorksClient::execute`, a generic GraphQL client. Add typed queries here, not in handlers.

## Notes

- reqwest uses `rustls-tls` (ring). Don't enable features that pull in `aws-lc-sys`; `cargo tree -i aws-lc-sys` should stay empty.
