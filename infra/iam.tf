# One runtime service account per service. This separation is what lets the
# question "what can this code write?" be answered by reading IAM instead of
# reading all the code.

resource "google_service_account" "seller_svc" {
  account_id   = "seller-svc-${local.suffix}"
  display_name = "Runtime seller-svc (${var.env})"
  depends_on   = [google_project_service.ini]
}

resource "google_service_account" "seller_app" {
  account_id   = "seller-app-${local.suffix}"
  display_name = "Runtime seller-app (${var.env})"
  depends_on   = [google_project_service.ini]
}

# seller-svc reads the action-token signing key. That is all — no access to
# any other secret, and no project-level roles.
resource "google_secret_manager_secret_iam_member" "svc_read_action_key" {
  secret_id = google_secret_manager_secret.action_key.id
  role      = "roles/secretmanager.secretAccessor"
  member    = google_service_account.seller_svc.member
}

# seller-app holds no secrets. Its only permission is calling seller-svc.
resource "google_cloud_run_v2_service_iam_member" "app_calls_svc" {
  project  = var.project_id
  location = google_cloud_run_v2_service.seller_svc.location
  name     = google_cloud_run_v2_service.seller_svc.name
  role     = "roles/run.invoker"
  member   = google_service_account.seller_app.member
}

# seller-app is public — it is the application. seller-svc is not:
# there is no allUsers anywhere, so /v1/actions cannot be reached directly
# from the internet.
resource "google_cloud_run_v2_service_iam_member" "app_public" {
  project  = var.project_id
  location = google_cloud_run_v2_service.seller_app.location
  name     = google_cloud_run_v2_service.seller_app.name
  role     = "roles/run.invoker"
  member   = "allUsers"
}

/* ---------- Deployer: the identity GitHub Actions uses ---------- */

resource "google_service_account" "deployer" {
  account_id   = "deployer-${local.suffix}"
  display_name = "GitHub Actions deployer (${var.env})"
  depends_on   = [google_project_service.ini]
}

resource "google_project_iam_member" "deployer" {
  for_each = toset([
    "roles/run.admin",               # deploy new revisions
    "roles/artifactregistry.writer", # push images
  ])

  project = var.project_id
  role    = each.value
  member  = google_service_account.deployer.member
}

# The deployer may deploy *as* a runtime SA, but inherits neither's
# permissions. Granted per service account, not project-wide — without this,
# a project-level `roles/iam.serviceAccountUser` would let the deployer
# impersonate any service account in the project.
resource "google_service_account_iam_member" "deployer_uses_runtime" {
  for_each = {
    svc = google_service_account.seller_svc.name
    app = google_service_account.seller_app.name
  }

  service_account_id = each.value
  role               = "roles/iam.serviceAccountUser"
  member             = google_service_account.deployer.member
}
