resource "google_cloud_run_v2_service" "seller_svc" {
  name     = "seller-svc-${local.suffix}"
  location = var.region

  # Tidak ada allUsers di iam.tf, jadi hanya pemanggil ber-IAM yang masuk.
  # INGRESS_ALL tetap dipakai karena seller-app memanggil lewat internet
  # dengan token OIDC; kalau nanti ada VPC connector, turunkan ke INTERNAL.
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

      # seller-svc membaca SELLER_SVC_BIND_ADDR, bukan PORT yang disuntik
      # Cloud Run, jadi portnya dipatok di sini supaya keduanya sepakat.
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

      # Browser tidak pernah memanggil seller-svc langsung — seller-app yang
      # memanggil dari sisi server — jadi tidak ada origin yang perlu
      # diizinkan. Dikosongkan dengan sengaja.
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

      # /readyz menyentuh works-server; dipakai untuk liveness akan membuat
      # Cloud Run me-restart seller-svc hanya karena upstream sedang turun.
      liveness_probe {
        http_get {
          path = "/healthz"
        }
        period_seconds = 30
      }
    }
  }

  # Image diganti oleh GitHub Actions, bukan oleh Terraform. Tanpa ini tiap
  # `terraform apply` akan mengembalikan layanan ke image lama di tfvars.
  lifecycle {
    ignore_changes = [
      template[0].containers[0].image,
      client,
      client_version,
    ]
  }

  depends_on = [
    google_project_service.ini,
    google_secret_manager_secret_iam_member.svc_baca_kunci_aksi,
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

      ports {
        container_port = 3000
      }

      env {
        name  = "PORT"
        value = "3000"
      }

      env {
        name  = "SELLER_SVC_URL"
        value = google_cloud_run_v2_service.seller_svc.uri
      }

      env {
        name  = "SELLER_SVC_MOCK"
        value = "false"
      }

      # seller-svc menolak pemanggil tanpa IAM, jadi seller-app mengambil ID
      # token dari metadata server dengan audience ini. Lihat
      # src/lib/server/api/http.ts di seller-app.
      env {
        name  = "SELLER_SVC_AUDIENCE"
        value = google_cloud_run_v2_service.seller_svc.uri
      }

      # WAJIB. adapter-node di SvelteKit 3 tidak lagi membaca ORIGIN dan
      # menganggap protokolnya https kalau tidak diberi tahu; di belakang
      # proxy Cloud Run, tanpa dua header ini setiap POST form ditolak
      # "Cross-site POST form submissions are forbidden" — artinya semua
      # tombol Setujui mati, dan tidak terlihat sama sekali saat dev.
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
          path = "/asisten"
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
