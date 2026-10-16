# API yang dipakai sekarang. Pub/Sub, BigQuery, Memorystore dan Vertex AI
# menyusul bersama layanan yang memakainya (Fase 4-7) — menyalakannya lebih
# awal hanya menambah permukaan tanpa manfaat.
resource "google_project_service" "ini" {
  for_each = toset([
    "artifactregistry.googleapis.com",
    "cloudresourcemanager.googleapis.com",
    "iam.googleapis.com",
    "iamcredentials.googleapis.com",
    "run.googleapis.com",
    "secretmanager.googleapis.com",
    "sts.googleapis.com",
  ])

  service = each.value

  # Jangan matikan API saat `terraform destroy`: API dipakai bersama dan
  # mematikannya bisa menjatuhkan hal lain di project yang sama.
  disable_on_destroy = false
}

locals {
  suffix = var.env

  # Placeholder supaya `apply` pertama berhasil sebelum ada image hasil build.
  placeholder = "us-docker.pkg.dev/cloudrun/container/hello"

  seller_svc_image = coalesce(var.seller_svc_image, local.placeholder)
  seller_app_image = coalesce(var.seller_app_image, local.placeholder)
}
