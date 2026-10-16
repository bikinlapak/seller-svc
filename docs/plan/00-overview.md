# 00 — Overview and key decisions

## Goal

One mobile-first app, **bikinlapak Seller**, where a merchant registers, sets up
the shop, manages products and stock, sees reports, chats with Asisten Stok
(PasStok), photographs waste and sends it to the right destination.
worksh.io stays the system of record; PasStok runs on Google Cloud.

This document covers the backend half. The frontend plan lives in
[`../../../seller-app/docs/plan`](../../../seller-app/docs/plan).

## Key decisions

| Decision | Choice | Why |
| -------- | ------ | --- |
| One app or several | One SvelteKit PWA (`seller-app`), installable, mobile-first; desktop gets the same app with a side-panel layout | No app-store wall; same code for phone and laptop; push via Web Push |
| System of record | worksh.io (works-server GraphQL + tenant Postgres) for accounts, tenants, products, stock, lots, POs, journal | Already built and tested; no second ledger |
| New backend code | **Rust** crates on Cloud Run: `seller-svc` (BFF), `passtok-executor`, `passtok-extractor`, `passtok-risk`, `passtok-agent`, `passtok-notifier`, sharing `passtok-core` | See *Go → Rust* below |
| Where chat + PasStok state live | A `passtok` schema in each tenant's Postgres (threads, messages, suggestions, decisions, waste records, recipients), owned by worksh.io, written only through the BFF/executor | Decision L28: one thread, rendered by any surface |
| Events | Pub/Sub topics + BigQuery subscriptions; `outbox` table in Postgres for exactly-once publish | Native on GCP; same mental model as Kafka (topic, subscription, DLQ) |
| AI | BigQuery `AI.FORECAST` (TimesFM) for numbers; Gemini on Vertex AI for text, Q&A, vision and daily specials | Numbers from SQL/Rust, words from Gemini (PRD §8) |
| Writes from AI | **None.** Every write is a merchant tap → signed single-use token → executor → worksh.io mutation | Safety rule FR-18/FR-27 |
| Realtime | SSE from the BFF; Redis (Memorystore) pub/sub to fan out across BFF instances | Simpler than WebSockets; works through proxies |

## Go → Rust: the one deliberate deviation from the Notion plan

The Notion build plan names Go services ("your strongest language") and the ADK
for Go. This repo implements the same topology in **Rust** instead, because
`seller-svc` already exists as an axum service with a works-server GraphQL
client, and `seller-app` already talks to it.

Service-for-service the design is unchanged — same boundaries, same events, same
"no AI writes" rule. Three things do change, and they are the ones to watch:

1. **No ADK.** There is no Agent Development Kit for Rust. `passtok-agent`
   implements its own small tool loop against the Vertex AI `generateContent`
   REST API: a system prompt, a fixed set of read-only tools, structured output
   via `responseSchema`, and a hard turn cap. This is less framework and more
   code we own — acceptable, because the agent's job is narrow (turn an event
   into a card, answer a grounded question, extract items from a photo) and the
   refusal boundary is easier to test when we wrote the loop.
2. **No official Rust SDKs for parts of GCP.** Pub/Sub, GCS and BigQuery have
   good community crates; Vertex AI does not. Those calls go over REST with ADC
   tokens. See [04-rust-stack.md](04-rust-stack.md).
3. **Repository layout.** The Notion plan assumes one monorepo
   (`apps/`, `services/`, `sql/`, …). Reality: `seller-app` and `seller-svc` are
   two git repos next to `comingsoon`. `seller-svc` becomes a **Cargo
   workspace** holding every backend crate plus `sql/`, `schemas/`, `seed/` and
   `infra/`; `seller-app` stays a standalone SvelteKit repo. Record this as
   ADR-0001 so nobody "fixes" it later.

What Rust buys in exchange: one language from HTTP handler to SQL with
compile-time-checked queries (`sqlx`), exhaustive matching on event and
suggestion state, and no GC pauses on the SSE fan-out path.

## Scope boundaries

`seller-svc` **does**: merchant session, screen-shaped endpoints, SSE, upload
URLs, the `passtok` schema, forecast/risk rules, agent prompting, token
signing and verification, event publishing, push and WhatsApp dispatch.

`seller-svc` **does not**: duplicate worksh.io authorization (works-server
enforces permissions and the tenant from the seller's access token); own
products, pages, stock or the journal; let any AI code path issue a mutation.

## Hackathon cut line

Phases 0–8 are the demo build and deliver every P0 in the PRD. Phase 9 is demo
hardening. Phases 10–12 are after the hackathon. The deadline is not recorded
yet, so phases are ordered by dependency, each sized S (≤1 day), M (2–3 days)
or L (4–5 days) for one engineer. See [05-phases.md](05-phases.md).

## Current state of this repo (2026-10-15)

`seller-svc` today is the Phase 0 skeleton for the *seller dashboard* scope only:

- `src/lib.rs` — `AppState`, `build_router`, CORS; tests build the router with
  `tower::ServiceExt::oneshot`, no port opened.
- `src/config.rs` — env config, prefix `SELLER_SVC_`; `from_lookup` is testable.
- `src/error.rs` — `AppError` → `{ "error": { code, message } }`; internal and
  upstream detail stays in the log.
- `src/routes/` — `health`, plus `products` and `builder` under `/v1`, both 501.
- `src/works/` — `WorksClient::execute`, an untyped GraphQL client.

Everything in this plan is additive to that. Code comments stay in Indonesian,
the same as `../comingsoon`; these plan documents are in English, matching the
existing `README.md` and `CLAUDE.md`.
