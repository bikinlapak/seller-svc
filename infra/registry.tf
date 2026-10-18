resource "google_artifact_registry_repository" "bikinlapak" {
  location      = var.region
  repository_id = "bikinlapak-${local.suffix}"
  description   = "Images for seller-svc and seller-app"
  format        = "DOCKER"

  # Keep the 10 most recent tags; discard the rest. Old images pile up fast
  # when every push to main deploys.
  cleanup_policies {
    id     = "keep-10-most-recent"
    action = "KEEP"
    most_recent_versions {
      keep_count = 10
    }
  }

  cleanup_policies {
    id     = "delete-older"
    action = "DELETE"
    condition {
      older_than = "2592000s" # 30 days
    }
  }

  depends_on = [google_project_service.ini]
}
