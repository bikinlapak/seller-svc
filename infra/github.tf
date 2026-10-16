# Workload Identity Federation: GitHub Actions menukar token OIDC-nya dengan
# kredensial GCP berumur pendek. Tidak ada kunci service account yang dibuat,
# disimpan di secret repo, atau bisa bocor.
#
# Submodul gh-oidc dari terraform-google-modules/github-actions-runners.
# Submodul runner (gke / mig-vm / mig-container-vm) di modul yang sama
# sengaja TIDAK dipakai — lihat infra/README.md, bagian "Runner".
module "gh_oidc" {
  source  = "terraform-google-modules/github-actions-runners/google//modules/gh-oidc"
  version = "~> 5.0"

  project_id  = var.project_id
  pool_id     = "github-${local.suffix}"
  provider_id = "github-${local.suffix}"

  # Mapping bawaan tidak membawa repository_owner, padahal itu yang dipakai
  # attribute_condition di bawah.
  attribute_mapping = {
    "google.subject"             = "assertion.sub"
    "attribute.actor"            = "assertion.actor"
    "attribute.aud"              = "assertion.aud"
    "attribute.repository"       = "assertion.repository"
    "attribute.repository_owner" = "assertion.repository_owner"
  }

  # Pagar terpenting di berkas ini. Issuer-nya GitHub untuk *semua orang*:
  # tanpa syarat ini, workflow di repo mana pun di dunia bisa menukar
  # tokennya dengan kredensial deployer kita. sa_mapping di bawah sudah
  # mempersempit per repo, tapi syarat ini ditegakkan lebih dulu, di tingkat
  # provider, jadi salah ketik di sa_mapping tidak langsung membuka pintu.
  attribute_condition = "assertion.repository_owner == '${var.github_owner}'"

  # Tiap repo hanya boleh memakai deployer lewat binding-nya sendiri.
  sa_mapping = {
    for repo in var.github_repos : "deployer-${repo}" => {
      sa_name   = google_service_account.deployer.name
      attribute = "attribute.repository/${var.github_owner}/${repo}"
    }
  }

  depends_on = [google_project_service.ini]
}
