# The Ed25519 private key that signs action tokens.
#
# Terraform creates the container, not the content: key material must never
# enter state or tfvars. Fill it once per environment, outside Terraform:
#
#   openssl genpkey -algorithm ed25519 -outform DER \
#     | base64 \
#     | gcloud secrets versions add seller-svc-action-key-<env> --data-file=-
#
# IMPORTANT: base64 — Cloud Run rejects binary env vars, and config.rs
# decodes the base64 itself.
#
# If this key is replaced, every suggestion token in circulation becomes
# invalid — a merchant looking at a card must reload. That is correct: old
# tokens must indeed stop being honored.
resource "google_secret_manager_secret" "action_key" {
  secret_id = "seller-svc-action-key-${local.suffix}"

  replication {
    user_managed {
      replicas {
        location = var.region
      }
    }
  }

  depends_on = [google_project_service.ini]
}
