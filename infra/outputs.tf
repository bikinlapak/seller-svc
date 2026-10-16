output "seller_app_url" {
  description = "URL publik aplikasi penjual."
  value       = google_cloud_run_v2_service.seller_app.uri
}

output "seller_svc_url" {
  description = "URL seller-svc. Hanya bisa dipanggil dengan IAM."
  value       = google_cloud_run_v2_service.seller_svc.uri
}

output "artifact_registry" {
  description = "Awalan image untuk docker push."
  value       = "${var.region}-docker.pkg.dev/${var.project_id}/${google_artifact_registry_repository.bikinlapak.repository_id}"
}

# Tiga nilai di bawah masuk ke GitHub Actions sebagai variables (bukan
# secrets — tidak ada yang rahasia di sini; keamanannya dari
# attribute_condition, bukan dari menyembunyikan nama provider).
output "workload_identity_provider" {
  description = "Isi ke variable WIF_PROVIDER di kedua repo."
  value       = module.gh_oidc.provider_name
}

output "deployer_service_account" {
  description = "Isi ke variable WIF_SERVICE_ACCOUNT di kedua repo."
  value       = google_service_account.deployer.email
}

output "action_key_secret" {
  description = "Secret yang harus diisi manual sebelum seller-svc bisa menandatangani token."
  value       = google_secret_manager_secret.action_key.secret_id
}

output "langkah_berikutnya" {
  description = "Yang masih harus dikerjakan manusia setelah apply."
  value       = <<-EOT
    1. Isi kunci penandatangan (sekali per lingkungan):
         openssl genpkey -algorithm ed25519 -outform DER \
           | gcloud secrets versions add ${google_secret_manager_secret.action_key.secret_id} --data-file=-

    2. Set variables di kedua repo GitHub (Settings > Secrets and variables > Actions > Variables):
         WIF_PROVIDER        = ${module.gh_oidc.provider_name}
         WIF_SERVICE_ACCOUNT = ${google_service_account.deployer.email}
         GCP_PROJECT_ID      = ${var.project_id}
         GCP_REGION          = ${var.region}
         AR_REPO             = ${google_artifact_registry_repository.bikinlapak.repository_id}
         ENV_NAME            = ${var.env}

    3. Push ke main di salah satu repo; workflow deploy akan mengganti image
       placeholder dengan hasil build.
  EOT
}
