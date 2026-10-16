resource "google_artifact_registry_repository" "bikinlapak" {
  location      = var.region
  repository_id = "bikinlapak-${local.suffix}"
  description   = "Image seller-svc dan seller-app"
  format        = "DOCKER"

  # Simpan 10 tag terakhir; sisanya dibuang. Image lama menumpuk cepat kalau
  # tiap commit ke main men-deploy.
  cleanup_policies {
    id     = "simpan-10-terbaru"
    action = "KEEP"
    most_recent_versions {
      keep_count = 10
    }
  }

  cleanup_policies {
    id     = "buang-yang-lebih-tua"
    action = "DELETE"
    condition {
      older_than = "2592000s" # 30 hari
    }
  }

  depends_on = [google_project_service.ini]
}
