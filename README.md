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

## Infrastructure (Google Cloud)

Managed by Terraform in [`infra/`](infra/README.md) — one root module for both
seller services, applied per environment.

| Concern | Choice |
| ------- | ------ |
| Secret management | Secret Manager; only the Ed25519 action-key secret. Filled manually (base64 of PKCS#8 DER), never in Terraform state |
| Container registry | Artifact Registry (`bikinlapak-<env>` repo, Docker format) with cleanup policies: keep 10 most recent tags, delete older than 30 days |
| Compute | Cloud Run (min 0 / max 4 instances); startup + liveness probes on `/healthz`, which deliberately does not touch works-server |
| CI/CD identity | Workload Identity Federation — GitHub Actions exchanges an OIDC token for short-lived GCP credentials; no service-account keys exist anywhere |
| Runner | Self-hosted GitHub Actions runner on a GCP VM (`github-runner`, Debian, docker data-root and swap on a second disk). CI (fmt/clippy/test) stays on GitHub-hosted runners |
| Terraform scope | Terraform owns services, IAM, WIF and secrets *containers*; the deployed image is owned by the workflow (`lifecycle.ignore_changes` on image) |

### Image strategy: fat base + thin app

Two Dockerfiles, mirroring the 2fa-gateway pattern:

- `Dockerfile.base` — the fat image, built once per OS/library change, tagged
  `v2`. It is the *runtime* base (`debian:bookworm-slim` + ca-certificates),
  so a deploy never runs apt — the 2fa-gateway pattern.
- `Dockerfile.app` — the thin image: Rust build in a pinned
  `rust:1.97-bookworm` stage, then the binary copied onto `BASE_IMAGE`.
  Tagged with the commit SHA.

### Workflow strategy

On merge to `main`/`master`, `deploy.yml` runs on the self-hosted runner:

1. Authenticate to GCP via WIF (OIDC, no stored keys)
2. **Verify or build the base image** — `docker pull` it; only if missing,
   build `Dockerfile.base` and push (~once per toolchain bump)
3. Build the thin app image from the current commit, push `:sha` to Artifact
   Registry
4. `gcloud run deploy` the new revision; internal startup/liveness probes
   validate `/healthz` before traffic shifts

Everything heavier (services, IAM, secrets, WIF) is Terraform's job; the
workflow stays thin on purpose.

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
