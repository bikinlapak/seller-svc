# Kunci privat Ed25519 penandatangan token aksi.
#
# Terraform membuat wadahnya, bukan isinya: materi kunci tidak boleh masuk
# state atau tfvars. Isi sekali per lingkungan, di luar Terraform:
#
#   openssl genpkey -algorithm ed25519 -outform DER \
#     | gcloud secrets versions add seller-svc-action-key-<env> --data-file=-
#
# Kalau kunci ini berganti, semua token saran yang beredar jadi tidak sah —
# penjual yang sedang melihat kartu harus memuat ulang. Itu benar: token lama
# memang tidak boleh berlaku lagi.
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
