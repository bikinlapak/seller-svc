# Build Plan v1 — seller-svc (backend)

Rust backend for **bikinlapak Seller**: one mobile-first app where a merchant
registers, sets up the shop, manages products and stock, sees reports, chats
with **Asisten Stok** (PasStok), photographs waste and sends it to the right
destination.

Source of scope: Notion — [Build Plan v1 — bikinlapak Seller app](https://app.notion.com/p/3fa1608449d581d88c2dff0c6d0f0120)
(parent: *PasStok — Forecast, Auto-PO & Waste Reduction (Hackathon)*).
Frontend counterpart: [`../../../seller-app/docs/plan`](../../../seller-app/docs/plan).

Status: plan, 2026-10-15.

## The one-line architecture

worksh.io stays the system of record (accounts, tenants, products, stock, lots,
POs, journal). `seller-svc` is the Rust layer in front of it: it holds the
merchant session, shapes screen-sized responses, owns the `passtok` schema for
chat and PasStok state, runs the forecast/risk pipeline, and is the **only**
path by which an AI suggestion can become a write — and only after a merchant
tap.

## Documents

| # | Document | What it settles |
| - | -------- | --------------- |
| 00 | [overview.md](00-overview.md) | Goal, scope, key decisions, the Go→Rust substitution |
| 01 | [architecture.md](01-architecture.md) | Cargo workspace, crates, topology, deploy units |
| 02 | [data-model.md](02-data-model.md) | `passtok` Postgres schema, BigQuery datasets, works-core asks |
| 03 | [api-surface.md](03-api-surface.md) | Every HTTP endpoint mapped to an axum route module |
| 04 | [rust-stack.md](04-rust-stack.md) | Crate choices, with the reason for each |
| 05 | [phases.md](05-phases.md) | Phases 0–12 as backend tasks, with exit criteria |
| 06 | [quality.md](06-quality.md) | Definition of done, testing strategy, security |
| 07 | [risks-and-questions.md](07-risks-and-questions.md) | Risks, mitigations, open questions |

## Reading order

New to the project: 00 → 01 → 05. Picking up a task: 05 for the checklist, then
02/03 for the shape of the thing you are building, 04 for which crate to reach
for.
