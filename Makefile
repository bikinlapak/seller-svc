# Manual deployment to Cloud Run — the same flow the GitHub workflow runs,
# for when the pipeline is unavailable (bootstrap, hotfix, broken runner).
#
# Project is PINNED to the worksh playground:
#   project-9567c06f-f9aa-4397-a2c (region asia-southeast2, env staging)
# Every gcloud call passes --project explicitly, so it works no matter what
# gcloud config a developer has locally.
#
# Prerequisites (once per developer — an owner runs):
#   gcloud auth login && gcloud auth application-default login
#   make grants GRANT_USER=<developer-email>
#
# Two build paths:
#   make release      — build locally (needs docker with BuildKit)
#   make vm-release   — build on the runner VM (for laptops without BuildKit;
#                       VM: github-runner, asia-southeast1-c)

REGION  ?= asia-southeast2
PROJECT ?= project-9567c06f-f9aa-4397-a2c
REPO    ?= bikinlapak-staging
ENV     ?= staging

SERVICE := seller-svc-$(ENV)
SA      := $(SERVICE)@$(PROJECT).iam.gserviceaccount.com
REG     := $(REGION)-docker.pkg.dev/$(PROJECT)/$(REPO)
BASE    := $(REG)/seller-svc-base:v2
TAG     ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo manual)
IMAGE   := $(REG)/seller-svc:$(TAG)
PLATFORM := linux/amd64
export DOCKER_BUILDKIT := 1

RUNNER_VM     ?= github-runner
RUNNER_ZONE   ?= asia-southeast1-c
VM_DIR        := /mnt/storage/workspace/deployment/seller-svc-manual
SSH           := gcloud compute ssh $(RUNNER_VM) --zone=$(RUNNER_ZONE) --command
SCP           := gcloud compute scp --zone=$(RUNNER_ZONE)

URL := $(shell gcloud run services describe $(SERVICE) --region=$(REGION) --project=$(PROJECT) --format='value(status.url)' 2>/dev/null)

.PHONY: help grants auth base build push deploy health release vm-sync vm-base vm-build vm-push vm-release

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

grants: ## OWNER ONLY: grant a developer push+deploy rights. make grants GRANT_USER=email
	@test -n "$(GRANT_USER)" || (echo "usage: make grants GRANT_USER=name@example.com" && exit 1)
	gcloud artifacts repositories add-iam-policy-binding $(REPO) --location=$(REGION) --project=$(PROJECT) \
		--member=user:$(GRANT_USER) --role=roles/artifactregistry.writer
	gcloud projects add-iam-policy-binding $(PROJECT) \
		--member=user:$(GRANT_USER) --role=roles/run.developer
	gcloud iam service-accounts add-iam-policy-binding $(SA) --project=$(PROJECT) \
		--member=user:$(GRANT_USER) --role=roles/iam.serviceAccountUser
	gcloud iam service-accounts add-iam-policy-binding $(SA) --project=$(PROJECT) \
		--member=user:$(GRANT_USER) --role=roles/iam.serviceAccountTokenCreator

auth: ## Authorize local docker to push to Artifact Registry
	gcloud auth configure-docker $(REGION)-docker.pkg.dev --quiet

base: ## Verify base image in registry; build+push locally if missing
	@if docker pull "$(BASE)" >/dev/null 2>&1; then \
		echo "==> base image present: $(BASE)"; \
	else \
		echo "==> base image missing, building (once per OS/library bump)"; \
		docker build --platform $(PLATFORM) -f Dockerfile.base -t "$(BASE)" . && \
		docker push "$(BASE)"; \
	fi

build: ## Build the thin app image locally (needs docker + BuildKit)
	docker build --platform $(PLATFORM) -f Dockerfile.app --build-arg BASE_IMAGE="$(BASE)" -t "$(IMAGE)" .

push: ## Push the app image from the local daemon
	docker push "$(IMAGE)"

deploy: ## Deploy the image as a new Cloud Run revision
	gcloud run deploy $(SERVICE) \
		--image="$(IMAGE)" \
		--region=$(REGION) \
		--project=$(PROJECT) \
		--service-account=$(SA) \
		--quiet

health: ## Check /health on the deployed service (identity token via the runtime SA)
	@T=$$(gcloud auth print-identity-token --impersonate-service-account=$(SA) --audiences="$(URL)" 2>/dev/null); \
	if [ -n "$$T" ]; then \
		curl -sf -H "Authorization: Bearer $$T" "$(URL)/health" && echo; \
	else \
		echo "cannot mint identity token (needs roles/iam.serviceAccountTokenCreator on $(SA)); falling back to revision status"; \
		gcloud run services describe $(SERVICE) --region=$(REGION) --project=$(PROJECT) --format='table(status.latestReadyRevisionName)'; \
	fi

release: auth base build push deploy health ## Full manual deploy, local build

vm-sync: ## Copy the build context (incl. scripts/) to the runner VM
	tar czf - --exclude .git --exclude target --exclude .env --exclude '._*' . \
		| $(SSH) "mkdir -p $(VM_DIR) && tar xzf - -C $(VM_DIR)"

vm-base: ## Verify base image from the VM; build+push there if missing
	$(SSH) "cd $(VM_DIR) && bash scripts/vm-base.sh $(BASE)"

vm-build: ## Build the app image on the VM (native amd64, warm BuildKit cache)
	$(SSH) "cd $(VM_DIR) && bash scripts/vm-build.sh $(BASE) $(IMAGE)"

vm-push: ## Push the app image from the VM
	$(SSH) "cd $(VM_DIR) && bash scripts/vm-push.sh $(IMAGE)"

vm-release: vm-sync vm-base vm-build vm-push deploy health ## Full manual deploy, build on the runner VM
	@echo "==> deployed $(IMAGE) (built on $(RUNNER_VM))"
