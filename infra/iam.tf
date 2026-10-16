# Satu service account runtime per layanan. Pemisahannya yang membuat
# pertanyaan "kode ini bisa menulis apa?" bisa dijawab dengan membaca IAM,
# bukan membaca seluruh kode.

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

# seller-svc membaca kunci penandatangan token aksi. Itu saja — tidak ada
# akses ke secret lain, dan tidak ada peran setingkat project.
resource "google_secret_manager_secret_iam_member" "svc_baca_kunci_aksi" {
  secret_id = google_secret_manager_secret.action_key.id
  role      = "roles/secretmanager.secretAccessor"
  member    = google_service_account.seller_svc.member
}

# seller-app tidak memegang secret apa pun. Satu-satunya izinnya adalah
# memanggil seller-svc.
resource "google_cloud_run_v2_service_iam_member" "app_panggil_svc" {
  project  = var.project_id
  location = google_cloud_run_v2_service.seller_svc.location
  name     = google_cloud_run_v2_service.seller_svc.name
  role     = "roles/run.invoker"
  member   = google_service_account.seller_app.member
}

# seller-app terbuka untuk umum — ini aplikasinya. seller-svc tidak:
# tidak ada allUsers di mana pun, jadi /v1/actions tidak bisa dijangkau
# langsung dari internet.
resource "google_cloud_run_v2_service_iam_member" "app_publik" {
  project  = var.project_id
  location = google_cloud_run_v2_service.seller_app.location
  name     = google_cloud_run_v2_service.seller_app.name
  role     = "roles/run.invoker"
  member   = "allUsers"
}

/* ---------- Deployer: identitas yang dipakai GitHub Actions ---------- */

resource "google_service_account" "deployer" {
  account_id   = "deployer-${local.suffix}"
  display_name = "GitHub Actions deployer (${var.env})"
  depends_on   = [google_project_service.ini]
}

resource "google_project_iam_member" "deployer" {
  for_each = toset([
    "roles/run.admin",               # deploy revisi baru
    "roles/artifactregistry.writer", # dorong image
  ])

  project = var.project_id
  role    = each.value
  member  = google_service_account.deployer.member
}

# Deployer boleh men-deploy *sebagai* SA runtime, tapi tidak mewarisi izin
# keduanya. Diberikan per service account, bukan setingkat project — tanpa ini
# `roles/iam.serviceAccountUser` di level project akan membuat deployer bisa
# meniru service account apa pun yang ada.
resource "google_service_account_iam_member" "deployer_pakai_runtime" {
  for_each = {
    svc = google_service_account.seller_svc.name
    app = google_service_account.seller_app.name
  }

  service_account_id = each.value
  role               = "roles/iam.serviceAccountUser"
  member             = google_service_account.deployer.member
}
