# seller-svc

Backend for the bikinlapak seller dashboard ([seller-app](../seller-app)). Products and the store's web builder are managed in worksh.io. This service is a thin layer that calls the works-server GraphQL API on behalf of the logged-in seller.

## Build plan

`seller-svc` is the backend half of **bikinlapak Seller** (PasStok). The full
plan — architecture, `passtok` data model, API surface, Rust stack, phases 0–12,
quality bar — is in [`docs/plan/`](docs/plan/README.md). The frontend plan is in
[`../seller-app/docs/plan`](../seller-app/docs/plan/README.md).

## Running

```sh
cp .env.example .env
cargo run                 # http://127.0.0.1:8080/healthz
cargo test
cargo clippy --all-targets
```

## Endpoints

Everything `seller-app` calls is implemented and under test. The data behind it
still comes from `src/seed.rs` — worksh.io and the `passtok` schema are not
wired yet (plan Phases 2 and 6), so these serve the real contract with
stand-in content.

| Method | Path                     | Serves                                |
| ------ | ------------------------ | ------------------------------------- |
| GET    | `/healthz`               | liveness; does not touch works-server |
| GET    | `/v1/home`               | Beranda asisten aggregate             |
| GET    | `/v1/assistant/thread`   | `?topik=` one of nine threads         |
| GET    | `/v1/assistant/settings` | Pengaturan AI                         |
| POST   | `/v1/actions/{token}`    | run an action token → result card     |
| GET    | `/v1/products`           | product list                          |
| GET    | `/v1/stock`              | stock by location + summary           |
| GET    | `/v1/builder/pages`      | 501, not implemented yet              |

## Action tokens

`POST /v1/actions/{token}` is the only endpoint that changes state on the
assistant's suggestion, and it is the safety rule (FR-18/FR-27) in code: no AI
path writes. The agent may only _sign_ a proposal; the write happens after the
merchant taps, and only if the token survives, in order:

1. Ed25519 signature, `aud`, `iss`
2. `exp` — expired tokens are rejected (410)
3. `sub` — the token's tenant must match the caller (401)
4. `jti` — single use; a second tap gets 409
5. `max_qty` — the final quantity may go **down**, never up (422)

`jti` is claimed only after every other check passes, so a rejected call does
not burn a valid token. The store is in memory today; it becomes
`passtok.action_tokens` with `used_at` written in the same transaction as the
worksh.io mutation. `src/token.rs` holds the tests, including a concurrency one
asserting that eight simultaneous taps produce exactly one write.

Error codes a client can branch on: `token_invalid`, `token_expired`,
`token_used`, `qty_above_cap`, `bad_request`, `not_found`, `upstream_error`.
