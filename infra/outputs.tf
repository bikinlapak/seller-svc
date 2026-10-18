output "seller_app_url" {
  description = "Public URL of the seller application."
  value       = google_cloud_run_v2_service.seller_app.uri
}

output "seller_svc_url" {
  description = "seller-svc URL. Callable only with IAM."
  value       = google_cloud_run_v2_service.seller_svc.uri
}

output "artifact_registry" {
  description = "Image prefix for docker push."
  value       = "${var.region}-docker.pkg.dev/${var.project_id}/${google_artifact_registry_repository.bikinlapak.repository_id}"
}

# The three values below go into GitHub Actions as variables (not secrets —
# nothing here is secret; the security comes from attribute_condition, not
# from hiding the provider name).
output "workload_identity_provider" {
  description = "Set as the WIF_PROVIDER variable in both repos."
  value       = module.gh_oidc.provider_name
}

output "deployer_service_account" {
  description = "Set as the WIF_SERVICE_ACCOUNT variable in both repos."
  value       = google_service_account.deployer.email
}

output "action_key_secret" {
  description = "Secret that must be filled manually before seller-svc can sign tokens."
  value       = google_secret_manager_secret.action_key.secret_id
}

output "next_steps" {
  description = "What a human still has to do after apply."
  value       = <<-EOT
    1. Fill the signing key (once per environment):
         openssl genpkey -algorithm ed25519 -outform DER \
           | base64 \
           | gcloud secrets versions add ${google_secret_manager_secret.action_key.secret_id} --data-file=-

    2. Set variables in both GitHub repos (Settings > Secrets and variables > Actions > Variables):
         WIF_PROVIDER        = ${module.gh_oidc.provider_name}
         WIF_SERVICE_ACCOUNT = ${google_service_account.deployer.email}
         GCP_PROJECT_ID      = ${var.project_id}
         GCP_REGION          = ${var.region}
         AR_REPO             = ${google_artifact_registry_repository.bikinlapak.repository_id}
         ENV_NAME            = ${var.env}

    3. Push to main in either repo; the deploy workflow will replace the
       placeholder image with the build result.
  EOT
}
