# Workload Identity Federation: GitHub Actions exchanges its OIDC token for
# short-lived GCP credentials. No service-account key is ever created,
# stored in a repo secret, or able to leak.
#
# The gh-oidc submodule of terraform-google-modules/github-actions-runners.
# The runner submodules (gke / mig-vm / mig-container-vm) of the same module
# are deliberately NOT used — see the "Runner" section in infra/README.md.
module "gh_oidc" {
  source  = "terraform-google-modules/github-actions-runners/google//modules/gh-oidc"
  version = "~> 5.0"

  project_id  = var.project_id
  pool_id     = "github-${local.suffix}"
  provider_id = "github-${local.suffix}"

  # The default mapping does not carry repository_owner, which the
  # attribute_condition below relies on.
  attribute_mapping = {
    "google.subject"             = "assertion.sub"
    "attribute.actor"            = "assertion.actor"
    "attribute.aud"              = "assertion.aud"
    "attribute.repository"       = "assertion.repository"
    "attribute.repository_owner" = "assertion.repository_owner"
  }

  # The most important fence in this file. GitHub's issuer serves the whole
  # world: without this condition, a workflow in ANY repo on GitHub could
  # exchange its token for our deployer credentials. sa_mapping below
  # narrows per repo, but this condition is enforced first, at the provider
  # level, so a typo in sa_mapping does not immediately open the door.
  attribute_condition = "assertion.repository_owner == '${var.github_owner}'"

  # Each repo may only use the deployer through its own binding.
  sa_mapping = {
    for repo in var.github_repos : "deployer-${repo}" => {
      sa_name   = google_service_account.deployer.name
      attribute = "attribute.repository/${var.github_owner}/${repo}"
    }
  }

  depends_on = [google_project_service.ini]
}
