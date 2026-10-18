variable "project_id" {
  description = "Target GCP project. One project per environment."
  type        = string
}

variable "region" {
  description = <<-EOT
    Cloud Run region. Jakarta where possible — the sellers are there and
    latency is felt during demos. Vertex AI is not complete in
    asia-southeast2 yet, so model calls may fall through to
    asia-southeast1 (see ADR-0006).
  EOT
  type        = string
  default     = "asia-southeast2"
}

variable "env" {
  description = "Environment name, used as the suffix of resource names."
  type        = string

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{1,14}$", var.env))
    error_message = "env must be lowercase letters, digits and dashes, 2-15 characters."
  }
}

variable "github_owner" {
  description = "Owner of the GitHub repos. Used as the WIF fence."
  type        = string
  default     = "bikinlapak"
}

variable "github_repos" {
  description = <<-EOT
    Repos allowed to deploy. Each repo gets its own WIF binding, so another
    repo of the same org still cannot use this deployer.
  EOT
  type        = list(string)
  default     = ["seller-svc", "seller-app"]
}

variable "works_graphql_url" {
  description = "works-server GraphQL endpoint (worksh.io) for this environment."
  type        = string
}

variable "seller_svc_image" {
  description = <<-EOT
    Full seller-svc image with tag. Left empty on first apply: Terraform
    installs a placeholder image, then GitHub Actions replaces it with the
    build result. See lifecycle ignore_changes in run.tf.
  EOT
  type        = string
  default     = ""
}

variable "seller_app_image" {
  description = "Full seller-app image with tag. Treated the same way."
  type        = string
  default     = ""
}

variable "min_instances_svc" {
  description = <<-EOT
    seller-svc holds SSE connections once Phase 6 lands, so min 1 in the
    demo environment. Otherwise 0 so there is no idle cost.
  EOT
  type        = number
  default     = 0
}

variable "max_instances" {
  description = "Cost fence: upper bound of instances per service."
  type        = number
  default     = 4
}
