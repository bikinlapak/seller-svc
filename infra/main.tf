# APIs in active use. Pub/Sub, BigQuery, Memorystore and Vertex AI arrive
# together with the services that need them (Phases 4-7) — enabling them
# earlier only adds surface area without benefit.
resource "google_project_service" "ini" {
  for_each = toset([
    "aiplatform.googleapis.com",
    "artifactregistry.googleapis.com",
    "cloudresourcemanager.googleapis.com",
    "iam.googleapis.com",
    "iamcredentials.googleapis.com",
    "run.googleapis.com",
    "secretmanager.googleapis.com",
    "sts.googleapis.com",
  ])

  service = each.value

  # Do not disable APIs on `terraform destroy`: APIs are shared and
  # disabling them can take down other things in the same project.
  disable_on_destroy = false
}

locals {
  suffix = var.env

  # Placeholder so the first `apply` succeeds before any built image exists.
  placeholder = "us-docker.pkg.dev/cloudrun/container/hello"

  seller_svc_image = coalesce(var.seller_svc_image, local.placeholder)
  seller_app_image = coalesce(var.seller_app_image, local.placeholder)
}
