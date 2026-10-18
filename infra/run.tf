resource "google_cloud_run_v2_service" "seller_svc" {
  name     = "seller-svc-${local.suffix}"
  location = var.region

  # No allUsers in iam.tf, so only IAM-authorized callers get in.
  # INGRESS_ALL is still used because seller-app calls it over the internet
  # with an OIDC token; if a VPC connector arrives later, lower to INTERNAL.
  ingress             = "INGRESS_TRAFFIC_ALL"
  deletion_protection = false

  template {
    service_account = google_service_account.seller_svc.email

    scaling {
      min_instance_count = var.min_instances_svc
      max_instance_count = var.max_instances
    }

    containers {
      image = local.seller_svc_image

      # seller-svc reads SELLER_SVC_BIND_ADDR, not the PORT that Cloud Run
      # injects, so the port is pinned here to keep the two in agreement.
      ports {
        container_port = 8080
      }

      env {
        name  = "SELLER_SVC_BIND_ADDR"
        value = "0.0.0.0:8080"
      }

      env {
        name  = "SELLER_SVC_LOG_JSON"
        value = "true"
      }

      env {
        name  = "SELLER_SVC_WORKS_GRAPHQL_URL"
        value = var.works_graphql_url
      }

      # A browser never calls seller-svc directly — seller-app calls it
      # server-side — so there is no origin to allow. Left empty on purpose.
      env {
        name  = "SELLER_SVC_CORS_ORIGINS"
        value = ""
      }

      env {
        name = "SELLER_SVC_ACTION_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.action_key.secret_id
            version = "latest"
          }
        }
      }

      resources {
        limits = {
          cpu    = "1"
          memory = "512Mi"
        }
      }

      startup_probe {
        http_get {
          path = "/healthz"
        }
        initial_delay_seconds = 2
        period_seconds        = 3
        failure_threshold     = 10
      }

      # /readyz would touch works-server; using it for liveness would make
      # Cloud Run restart seller-svc just because an upstream is down.
      liveness_probe {
        http_get {
          path = "/healthz"
        }
        period_seconds = 30
      }
    }
  }

  # The image is replaced by GitHub Actions, not by Terraform. Without this,
  # every `terraform apply` would roll the service back to the old tfvars
  # image.
  lifecycle {
    ignore_changes = [
      template[0].containers[0].image,
      client,
      client_version,
    ]
  }

  depends_on = [
    google_project_service.ini,
    google_secret_manager_secret_iam_member.svc_read_action_key,
  ]
}

resource "google_cloud_run_v2_service" "seller_app" {
  name     = "seller-app-${local.suffix}"
  location = var.region

  ingress             = "INGRESS_TRAFFIC_ALL"
  deletion_protection = false

  template {
    service_account = google_service_account.seller_app.email

    scaling {
      min_instance_count = 0
      max_instance_count = var.max_instances
    }

    containers {
      image = local.seller_app_image

      # Cloud Run injects PORT itself (a reserved env name); SvelteKit's
      # adapter-node reads it, so listening on that port is enough.
      ports {
        container_port = 8080
      }

      env {
        name  = "SELLER_SVC_URL"
        value = google_cloud_run_v2_service.seller_svc.uri
      }

      env {
        name  = "SELLER_SVC_MOCK"
        value = "false"
      }

      # seller-svc refuses callers without IAM, so seller-app fetches an ID
      # token from the metadata server with this audience. See
      # src/lib/server/api/http.ts in seller-app.
      env {
        name  = "SELLER_SVC_AUDIENCE"
        value = google_cloud_run_v2_service.seller_svc.uri
      }

      # REQUIRED. SvelteKit 3's adapter-node no longer reads ORIGIN and
      # assumes https when not told otherwise; behind the Cloud Run proxy,
      # without these two headers every form POST is rejected with
      # "Cross-site POST form submissions are forbidden" — meaning every
      # Approve button dies, and it is completely invisible in dev mode.
      env {
        name  = "PROTOCOL_HEADER"
        value = "x-forwarded-proto"
      }

      env {
        name  = "HOST_HEADER"
        value = "x-forwarded-host"
      }

      resources {
        limits = {
          cpu    = "1"
          memory = "512Mi"
        }
      }

      startup_probe {
        http_get {
          # The app's assistant page (English route; /asisten was stale).
          path = "/assistant"
        }
        initial_delay_seconds = 3
        period_seconds        = 3
        failure_threshold     = 10
      }
    }
  }

  lifecycle {
    ignore_changes = [
      template[0].containers[0].image,
      client,
      client_version,
    ]
  }

  depends_on = [google_project_service.ini]
}
