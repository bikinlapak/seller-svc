# CLAUDE.md

## What this is

`seller-svc` is the Rust backend (axum 0.8, tokio) for the bikinlapak seller dashboard. Its frontend is `../seller-app`. Features: **product management**, the **web builder** (online-store pages), and — for the hackathon build — the **PasStok engine** behind the Shop Assistant (PRD v1.3: forecast, auto-PO, expiry risk, waste by destination).

Products, stock and pages belong to worksh.io, so this service **does not own a database for them**. It calls the works-server GraphQL API (`src/works/`) with the seller's access token, and works-server enforces permissions and the tenant. Don't duplicate authorization rules here. Until that is wired, `src/shop/demo.rs` supplies one tenant in the same shapes.

PasStok would live in worksh.io per the PRD (decision L28); it lives here for the hackathon by decision, in memory. Design rule from the PRD: **numbers come from the engine, words from the agent** — no quantity is ever produced by a language model, and every write goes through a signed, single-use action token.

**Everything is in English**: code comments, identifiers, and the API
contract shared with `../seller-app`. This diverges deliberately from
`../comingsoon`, which is Indonesian — do not "fix" it back.

## Commands

```sh
cargo run                       # reads .env (optional), see .env.example
cargo test                      # unit tests (config) + tests/http.rs (router via tower oneshot)
cargo clippy --all-targets
cargo fmt
cd deployment && docker compose up --build -d   # full local stack on :3000, see deployment/
```

## Layout

- `src/lib.rs`: `AppState`, `build_router`, the CORS layer. Integration tests build the router from here without opening a port.
- `src/config.rs`: config comes from env vars prefixed `SELLER_SVC_`. `from_lookup` is kept separate so it can be tested.
- `src/error.rs`: `AppError` → JSON `{ "error": { code, message } }`. Internal and upstream errors only reach the log, never the client.
- `src/routes/`: `health` (liveness, doesn't touch works-server), plus under
  `/v1`: `assistant` (home, threads, messages, SSE stream, settings, report,
  actions), `products`, `stock`, `waste` (photo upload, photos, donation
  receipts), `reports` (waste, dashboard, activity) and `admin` (demo reset /
  re-run). `builder` is still a 501 stub.
- `src/contract.rs`: the response shapes seller-app consumes, camelCase.
- `src/engine/`: the deterministic core — `forecast` (TimesFM stand-in),
  `reorder` (expiry cap, pack/MOQ), `expiry` (FEFO risk, cover), `waste`
  (destinations, legal filter, diversion), `special`. Pure and unit-tested.
- `src/agent/`: turns engine output into cards (`morning_run`), grounded Q&A
  (`answer`), wording (`explain`, Gemini stand-in), and `vision` (stand-in, or
  Gemini with `SELLER_SVC_GEMINI_API_KEY`).
- `src/executor.rs`: the only write path — validate, claim the token, apply,
  audit, emit the event.
- `src/store.rs`: PasStok state (threads, suggestions, POs, waste ledger,
  donations, audit, outbox) behind one mutex; a restart resets the demo.
- `src/views.rs`: screen-shaped views; `src/shop/`: catalog types + demo tenant.
- `src/token.rs`: action tokens — Ed25519 JWT, single use, tenant and quantity
  cap. The only path by which a suggestion becomes a write.
- `infra/`: Terraform for both services (see `infra/README.md`).
- `deployment/`: the local stack — `docker-compose.yml` (seller-svc, `../../seller-app`, Caddy edge on :3000), `Caddyfile`, and `.env.example` (copy to `.env`; every value has a default).
- `src/works/`: `WorksClient::execute`, a generic GraphQL client. Add typed queries here, not in handlers.

## Notes

- reqwest uses `rustls-tls` (ring). Don't enable features that pull in `aws-lc-sys`; `cargo tree -i aws-lc-sys` should stay empty.
