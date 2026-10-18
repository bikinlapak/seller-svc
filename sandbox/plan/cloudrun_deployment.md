That is a very standard and robust architecture. By hosting the GitHub runner on a Compute Engine VM, the runner has the necessary compute power and privileged Docker access to build your images, and it uses Terraform to handle the actual deployment to Cloud Run.

Here is exactly how to configure this setup, focusing on the Terraform code your runner will execute to deploy the Docker container to Cloud Run.

### 1. Permissions Required for the Runner VM

For your self-hosted runner to execute Terraform successfully without hardcoding credentials, you must attach a Google Cloud Service Account to the Compute Engine VM.

This VM Service Account needs the following IAM roles:

* **Cloud Run Admin** (`roles/run.admin`): To create and update Cloud Run services.
* **Service Account User** (`roles/iam.serviceAccountUser`): To attach a runtime identity to the Cloud Run service.
* **Artifact Registry Writer** (`roles/artifactregistry.writer`): To push the Docker image before deploying (if your CI/CD pipeline builds the image).
* **Storage Object Admin** (`roles/storage.objectAdmin`): To manage the Terraform state file in a GCS bucket.

### 2. Terraform Code to Deploy Cloud Run

This is the Terraform configuration (`main.tf`) that your GitHub Runner will apply. It provisions a runtime service account for your application and deploys the Docker container to Cloud Run.

```hcl
terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 5.0"
    }
  }

  # Best practice: Store state in a GCS bucket so the runner doesn't lose it between jobs
  backend "gcs" {
    bucket = "your-terraform-state-bucket"
    prefix = "terraform/cloudrun/state"
  }
}

provider "google" {
  project = "your-gcp-project-id"
  region  = "us-central1"
}

# 1. Create a dedicated Service Account for the app AT RUNTIME
# (This follows the principle of least privilege for the container itself)
resource "google_service_account" "app_sa" {
  account_id   = "my-cloudrun-app-sa"
  display_name = "Runtime Service Account for Cloud Run App"
}

# 2. Deploy the Docker container to Cloud Run
resource "google_cloud_run_v2_service" "app_service" {
  name     = "my-docker-app"
  location = "us-central1"
  ingress  = "INGRESS_TRAFFIC_ALL"

  template {
    # Attach the runtime service account created above
    service_account = google_service_account.app_sa.email

    containers {
      # The URL of the Docker image in Artifact Registry or GCR
      image = "us-docker.pkg.dev/your-gcp-project-id/your-repo/your-image:latest"

      # Match this to the EXPOSE port in your Dockerfile
      ports {
        container_port = 8080
      }

      # Inject environment variables your container needs
      env {
        name  = "ENVIRONMENT"
        value = "production"
      }

      # Optional: Resource limits
      resources {
        limits = {
          cpu    = "1"
          memory = "512Mi"
        }
      }
    }
  }
}

# 3. Make the Cloud Run service publicly accessible (Remove if this is an internal API)
resource "google_cloud_run_v2_service_iam_member" "public_access" {
  project  = google_cloud_run_v2_service.app_service.project
  location = google_cloud_run_v2_service.app_service.location
  name     = google_cloud_run_v2_service.app_service.name
  role     = "roles/run.invoker"
  member   = "allUsers"
}

# Output the URL so the GitHub Action logs show exactly where the app is live
output "cloud_run_url" {
  value       = google_cloud_run_v2_service.app_service.uri
  description = "The live URL of the deployed Cloud Run service"
}

```

### 3. The GitHub Actions Workflow

In your repository, your workflow will run on the VM, build the Docker container (if needed), and execute the Terraform code. Because the VM has a Service Account attached, Terraform automatically picks up the GCP credentials via Application Default Credentials (ADC).

```yaml
name: Deploy to Cloud Run via Terraform

on:
  push:
    branches:
      - main

jobs:
  deploy:
    runs-on: self-hosted # This targets your Compute Engine VM

    env:
      IMAGE_URL: us-docker.pkg.dev/your-gcp-project-id/your-repo/your-image:${{ github.sha }}

    steps:
      - name: Checkout Code
        uses: actions/checkout@v4

      # 1. Build and Push the Docker Container (Assuming VM has Docker installed)
      - name: Build and Push Docker Image
        run: |
          docker build -t $IMAGE_URL .
          # The VM's service account handles authentication to Artifact Registry automatically
          gcloud auth configure-docker us-docker.pkg.dev
          docker push $IMAGE_URL

      # 2. Update Terraform variables with the newly built image tag
      - name: Setup Terraform
        uses: hashicorp/setup-terraform@v3

      - name: Terraform Init
        run: terraform init
        working-directory: ./terraform-folder

      # 3. Apply the Terraform to deploy the container
      - name: Terraform Apply
        run: |
          terraform apply -auto-approve \
            -var="image=$IMAGE_URL"
        working-directory: ./terraform-folder

```
