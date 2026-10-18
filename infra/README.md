# infra — Terraform for PasStok

One Terraform root, one GCP project per environment. It manages **both**
services: `seller-svc` (this repo) and `seller-app` (the sibling repo),
because they are one system and one's URL goes into the other's env.

## What it creates

| File | Contents |
| ---- | -------- |
| `main.tf` | Enabled APIs, derived values |
| `registry.tf` | Artifact Registry Docker repo + cleanup policies |
| `iam.tf` | Runtime service account per service, deployer, bindings |
| `secrets.tf` | Container for the action-token signing key |
| `run.tf` | The two Cloud Run services |
| `github.tf` | Workload Identity Federation (`gh-oidc` module) |

## The security shape

Three deliberate choices that will feel wrong if changed without cause:

**`seller-svc` is not public.** There is no `allUsers` for it anywhere; the
only holder of `roles/run.invoker` is the `seller-app` runtime service
account. `POST /v1/actions/{token}` is a write path to worksh.io — it must
not be reachable from the internet even with a verified token. The
consequence lives in the code: `seller-app` fetches an ID token from the
metadata server (`SELLER_SVC_AUDIENCE`) and sends it as a bearer. Locally
that variable is empty and the whole path is skipped.

**No service-account keys.** GitHub Actions exchanges its OIDC token
through WIF. No JSON key is created, stored in a repo secret, or leakable
through logs.

**`attribute_condition` is the fence, not `sa_mapping`.** GitHub's issuer
serves the entire world. Without

```hcl
attribute_condition = "assertion.repository_owner == 'bikinlapak'"
```

a workflow in **any repo on GitHub** could exchange its token for our
deployer credentials. `sa_mapping` already narrows per repo, but the
condition above is enforced first, at the provider level, so a typo in
`sa_mapping` does not immediately open the door. Do not remove that line.

## Runner

The referenced module,
[`terraform-google-modules/github-actions-runners`](https://github.com/terraform-google-modules/terraform-google-github-actions-runners),
contains four submodules: three for **self-hosted runners** (GKE, MIG VM,
MIG container VM) and one for **WIF** (`gh-oidc`). Only `gh-oidc` is used
here.

The runner is self-hosted, but **not via the submodule**: the
`github-runner` VM (e2-medium, Debian) was created and registered manually
to the repo on GitHub. Docker on that VM keeps its data-root and swap on a
second disk (`/mnt/storage/workspace`), so toolchain images and BuildKit
cache never fill the system disk. The deploy workflow (`deploy.yml`) runs
there; light CI (`ci.yml`) stays on GitHub-hosted runners.

Why still no runner submodule: a single VM needs no auto-scaling/MIG, and
nothing about it needs Terraform management yet. Once runners multiply or
must be hardened automatically, move to `gh-runner-mig-container-vm` from
the same module.

## First run

Terraform cannot create its own state storage, so the first two steps are
manual.

```sh
# 1. Project + state bucket (once per environment)
gcloud projects create passtok-staging
gcloud billing projects link passtok-staging --billing-account=<ID>
gcloud storage buckets create gs://passtok-staging-tfstate \
  --project=passtok-staging --location=asia-southeast2 \
  --uniform-bucket-level-access
gcloud storage buckets update gs://passtok-staging-tfstate --versioning

# 2. Fill in the environment values
cp envs/staging.tfvars.example envs/staging.tfvars
cp envs/staging.gcs.tfbackend.example envs/staging.gcs.tfbackend
$EDITOR envs/staging.tfvars

# 3. Apply. Services come up with the Cloud Run placeholder image.
terraform init -backend-config=envs/staging.gcs.tfbackend
terraform plan  -var-file=envs/staging.tfvars
terraform apply -var-file=envs/staging.tfvars

# 4. Fill the signing key. IMPORTANT: base64 — Cloud Run rejects binary env
#    vars, and config.rs decodes base64 itself. Key material never enters
#    Terraform.
openssl genpkey -algorithm ed25519 -outform DER \
  | base64 \
  | gcloud secrets versions add seller-svc-action-key-staging --data-file=-

# 5. Set variables in both GitHub repos; `terraform output next_steps`
#    prints the values ready to paste.
```

After that, push to `main` in either repo: the `deploy.yml` workflow builds
the image and replaces the placeholder.

## Things that bite

**Images are not managed by Terraform.** `run.tf` sets `ignore_changes` on
`template[0].containers[0].image`. GitHub Actions owns the image; without
this, every `terraform apply` would roll the service back to the old
tfvars image.

**`PROTOCOL_HEADER` and `HOST_HEADER` are mandatory for `seller-app`.**
SvelteKit 3's `adapter-node` no longer reads `ORIGIN` and assumes https
when not told otherwise. Behind the Cloud Run proxy, without those two env
vars every form POST is rejected with `403 Cross-site POST form
submissions are forbidden` — meaning every *Approve* button dies,
invisible in `npm run dev` because CSRF checks are skipped in dev mode.

**`PORT` is reserved.** Cloud Run injects `PORT` itself; setting it in the
template is rejected with a 400. The container port must simply match what
the app listens on (8080).

**Replacing the action key invalidates live tokens.** A merchant staring
at a suggestion card must reload. That is correct, but do not do it in the
middle of a demo.

**Not here yet:** Pub/Sub, BigQuery, Memorystore, Vertex AI, Cloud
Scheduler, and the extractor job. They arrive together with the services
that use them (Phases 4–7 in `../docs/plan/05-phases.md`). Enabling APIs
and creating resources before anything uses them only adds cost and attack
surface.

## Daily commands

```sh
terraform fmt -recursive
terraform validate
terraform plan -var-file=envs/staging.tfvars
```

CI runs `fmt -check`, `init -backend=false` and `validate` on every PR to
this repo.
