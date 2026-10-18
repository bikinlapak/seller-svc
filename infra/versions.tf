terraform {
  required_version = ">= 1.5"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 6.0"
    }
  }

  # State lives in GCS so nobody holds state on their own laptop. The bucket
  # is not created here — chicken-and-egg: Terraform needs state before it
  # can create a place for state. Create it once, manually:
  #
  #   gcloud storage buckets create gs://<project>-tfstate \
  #     --location=asia-southeast2 --uniform-bucket-level-access
  #   gcloud storage buckets update gs://<project>-tfstate --versioning
  #
  # then `terraform init -backend-config=envs/staging.gcs.tfbackend`.
  backend "gcs" {}
}

provider "google" {
  project = var.project_id
  region  = var.region
}
