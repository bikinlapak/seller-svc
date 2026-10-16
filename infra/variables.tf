variable "project_id" {
  description = "Project GCP tujuan. Satu project per lingkungan."
  type        = string
}

variable "region" {
  description = <<-EOT
    Region Cloud Run. Jakarta kalau bisa — penjualnya di sana dan latensi
    terasa saat demo. Vertex AI belum lengkap di asia-southeast2, jadi
    pemanggilan model boleh jatuh ke asia-southeast1 (lihat ADR-0006).
  EOT
  type        = string
  default     = "asia-southeast2"
}

variable "env" {
  description = "Nama lingkungan, dipakai sebagai akhiran nama resource."
  type        = string

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{1,14}$", var.env))
    error_message = "env harus huruf kecil, angka dan tanda hubung, 2-15 karakter."
  }
}

variable "github_owner" {
  description = "Pemilik repo di GitHub. Dipakai sebagai pagar WIF."
  type        = string
  default     = "bikinlapak"
}

variable "github_repos" {
  description = <<-EOT
    Repo yang boleh men-deploy. Tiap repo dapat binding WIF sendiri, jadi
    repo lain milik org yang sama tetap tidak bisa memakai deployer ini.
  EOT
  type        = list(string)
  default     = ["seller-svc", "seller-app"]
}

variable "works_graphql_url" {
  description = "Endpoint GraphQL works-server (worksh.io) untuk lingkungan ini."
  type        = string
}

variable "seller_svc_image" {
  description = <<-EOT
    Image seller-svc lengkap dengan tag. Dibiarkan kosong saat apply pertama:
    Terraform memasang image placeholder, lalu GitHub Actions yang mengganti
    dengan hasil build. Lihat lifecycle ignore_changes di run.tf.
  EOT
  type        = string
  default     = ""
}

variable "seller_app_image" {
  description = "Image seller-app lengkap dengan tag. Sama perlakuannya."
  type        = string
  default     = ""
}

variable "min_instances_svc" {
  description = <<-EOT
    seller-svc menahan koneksi SSE begitu Fase 6 jalan, jadi min 1 di
    lingkungan demo. Di luar itu 0 supaya tidak ada biaya saat menganggur.
  EOT
  type        = number
  default     = 0
}

variable "max_instances" {
  description = "Pagar biaya: batas atas instance per layanan."
  type        = number
  default     = 4
}
