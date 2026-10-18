# Manual deployment to Cloud Run — the same flow the GitHub workflow runs,
# for when the pipeline is unavailable (bootstrap, hotfix, broken runner).
#
# Requires: docker with BuildKit (Dockerfile.app uses cache mounts) and
# gcloud with application-default credentials that may push to Artifact
# Registry. On Apple Silicon, images are cross-built for linux/amd64.
#
#   make release          # full flow: base (if missing) -> app -> push -> deploy -> health
#   make health           # check the running service only

REGION  ?= asia-southeast2
PROJECT ?= project-9567c06f-f9aa-4397-a2c
REPO    ?= bikinlapak-staging
ENV     ?= staging

SERVICE := seller-svc-$(ENV)
SA      := $(SERVICE)@$(PROJECT).iam.gserviceaccount.com
REG     := $(REGION)-docker.pkg.dev/$(PROJECT)/$(REPO)
BASE    := $(REG)/seller-svc-base:v1
TAG     ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo manual)
IMAGE   := $(REG)/seller-svc:$(TAG)
PLATFORM := linux/amd64
export DOCKER_BUILDKIT := 1

URL := $(shell gcloud run services describe $(SERVICE) --region=$(REGION) --project=$(PROJECT) --format='value(status.url)' 2>/dev/null)

.PHONY: help auth base build push deploy health release

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

auth: ## Authorize docker to push to Artifact Registry
	gcloud auth configure-docker $(REGION)-docker.pkg.dev --quiet

base: ## Verify the base image exists in the registry, build+push if missing
	@if docker pull "$(BASE)" >/dev/null 2>&1; then \
		echo "==> base image present: $(BASE)"; \
	else \
		echo "==> base image missing, building (once per toolchain bump)"; \
		docker build --platform $(PLATFORM) -f Dockerfile.base -t "$(BASE)" . && \
		docker push "$(BASE)"; \
	fi

build: ## Build the thin app image from the current commit
	docker build --platform $(PLATFORM) -f Dockerfile.app --build-arg BASE_IMAGE="$(BASE)" -t "$(IMAGE)" .

push: ## Push the app image
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

release: auth base build push deploy health ## Full manual deploy
	@echo "==> deployed $(IMAGE)"
