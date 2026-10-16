terraform {
  required_version = ">= 1.5"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 6.0"
    }
  }

  # State hidup di GCS supaya tidak ada yang memegang state di laptopnya
  # sendiri. Bucket-nya tidak dibuat di sini — ayam-telur: Terraform butuh
  # state sebelum bisa membuat tempat state. Bikin sekali, manual:
  #
  #   gcloud storage buckets create gs://<project>-tfstate \
  #     --location=asia-southeast2 --uniform-bucket-level-access
  #   gcloud storage buckets update gs://<project>-tfstate --versioning
  #
  # lalu `terraform init -backend-config=envs/staging.gcs.tfbackend`.
  backend "gcs" {}
}

provider "google" {
  project = var.project_id
  region  = var.region
}
