# infra — Terraform untuk PasStok

Satu root Terraform, satu project GCP per lingkungan. Mengurus **kedua**
layanan: `seller-svc` (repo ini) dan `seller-app` (repo sebelah), karena
keduanya satu sistem dan URL yang satu masuk ke env yang lain.

## Yang dibuat

| Berkas | Isi |
| ------ | --- |
| `main.tf` | API yang dinyalakan, nilai turunan |
| `registry.tf` | Artifact Registry Docker + aturan pembersihan |
| `iam.tf` | Service account runtime per layanan, deployer, binding |
| `secrets.tf` | Wadah kunci penandatangan token aksi |
| `run.tf` | Dua layanan Cloud Run |
| `github.tf` | Workload Identity Federation (modul `gh-oidc`) |

## Bentuk keamanannya

Tiga hal yang sengaja dipilih, dan akan terasa salah kalau diubah tanpa sebab:

**`seller-svc` tidak publik.** Tidak ada `allUsers` di mana pun untuknya; satu
-satunya yang memegang `roles/run.invoker` adalah service account runtime
`seller-app`. `POST /v1/actions/{token}` adalah jalur tulis ke worksh.io —
tidak boleh bisa dijangkau dari internet meski tokennya sudah diverifikasi.
Konsekuensinya ada di kode: `seller-app` mengambil ID token dari metadata
server (`SELLER_SVC_AUDIENCE`) dan mengirimnya sebagai bearer. Di lokal
variabel itu kosong dan seluruh jalur dilewati.

**Tidak ada kunci service account.** GitHub Actions menukar OIDC token-nya
lewat WIF. Tidak ada JSON key yang dibuat, disimpan di secret repo, atau bisa
bocor lewat log.

**`attribute_condition` adalah pagarnya, bukan `sa_mapping`.** Issuer GitHub
melayani seluruh dunia. Tanpa

```hcl
attribute_condition = "assertion.repository_owner == 'bikinlapak'"
```

workflow di repo **mana pun di GitHub** bisa menukar tokennya dengan
kredensial deployer kita. `sa_mapping` sudah mempersempit per repo, tapi
syarat di atas ditegakkan lebih dulu di tingkat provider, jadi salah ketik di
`sa_mapping` tidak langsung membuka pintu. Jangan hapus baris itu.

## Runner: kenapa submodulnya tidak dipakai

Modul yang dirujuk,
[`terraform-google-modules/github-actions-runners`](https://github.com/terraform-google-modules/terraform-google-github-actions-runners),
berisi empat submodul: tiga untuk **self-hosted runner** (GKE, MIG VM, MIG
container VM) dan satu untuk **WIF** (`gh-oidc`).

Yang dipakai di sini hanya `gh-oidc`. Runner-nya tidak, karena:

- runner bawaan GitHub gratis untuk repo publik dan murah untuk privat, pada
  dua repo sekecil ini;
- self-hosted runner adalah mesin yang harus ditambal, dipantau dan dibayar
  saat menganggur — infrastruktur baru untuk masalah yang belum ada;
- alasan kuat memakainya biasanya akses ke jaringan privat, dan CI di sini
  tidak membutuhkannya: tes unit tidak keluar jaringan, dan tes integrasi
  menuju worksh.io staging lewat internet.

**Kapan ini berubah:** begitu CI perlu menyentuh Memorystore, Cloud SQL
privat, atau works-server yang hanya ada di dalam VPC (Fase 6 ke atas).
Saat itu tambahkan `gh-runner-mig-container-vm` dari modul yang sama —
submodulnya sudah dipilih, tinggal dipasang.

## Jalan pertama kali

Terraform tidak bisa membuat tempat penyimpanan state-nya sendiri, jadi dua
langkah pertama manual.

```sh
# 1. Project + bucket state (sekali per lingkungan)
gcloud projects create passtok-staging
gcloud billing projects link passtok-staging --billing-account=<ID>
gcloud storage buckets create gs://passtok-staging-tfstate \
  --project=passtok-staging --location=asia-southeast2 \
  --uniform-bucket-level-access
gcloud storage buckets update gs://passtok-staging-tfstate --versioning

# 2. Isi nilai lingkungan
cp envs/staging.tfvars.example envs/staging.tfvars
cp envs/staging.gcs.tfbackend.example envs/staging.gcs.tfbackend
$EDITOR envs/staging.tfvars

# 3. Apply. Layanan berdiri dengan image placeholder Cloud Run.
terraform init -backend-config=envs/staging.gcs.tfbackend
terraform plan  -var-file=envs/staging.tfvars
terraform apply -var-file=envs/staging.tfvars

# 4. Isi kunci penandatangan. Materi kunci tidak pernah masuk Terraform.
openssl genpkey -algorithm ed25519 -outform DER \
  | gcloud secrets versions add seller-svc-action-key-staging --data-file=-

# 5. Set variables di kedua repo GitHub; `terraform output langkah_berikutnya`
#    mencetak nilainya siap salin.
```

Setelah itu push ke `main` di salah satu repo: workflow `deploy.yml` membangun
image dan mengganti placeholder.

## Hal yang menggigit

**Image tidak dikelola Terraform.** `run.tf` memasang `ignore_changes` pada
`template[0].containers[0].image`. GitHub Actions yang memegang image;
tanpa itu, tiap `terraform apply` akan mengembalikan layanan ke image lama.

**`PROTOCOL_HEADER` dan `HOST_HEADER` wajib di `seller-app`.** `adapter-node`
di SvelteKit 3 tidak lagi membaca `ORIGIN` dan menganggap protokolnya `https`
kalau tidak diberi tahu. Di belakang proxy Cloud Run, tanpa dua env itu setiap
POST form ditolak `403 Cross-site POST form submissions are forbidden` —
artinya semua tombol *Setujui* mati. Tidak terlihat sama sekali di
`npm run dev`, karena pemeriksaan CSRF dilewati di mode dev.

**Kunci aksi diganti = token beredar hangus.** Penjual yang sedang menatap
kartu saran harus memuat ulang. Itu memang benar, tapi jangan lakukan di
tengah demo.

**Belum ada di sini:** Pub/Sub, BigQuery, Memorystore, Vertex AI, Cloud
Scheduler, dan job extractor. Semuanya menyusul bersama layanan yang
memakainya (Fase 4–7 di `../docs/plan/05-phases.md`). Menyalakan API dan
membuat resource sebelum ada yang memakainya hanya menambah biaya dan
permukaan serang.

## Perintah harian

```sh
terraform fmt -recursive
terraform validate
terraform plan -var-file=envs/staging.tfvars
```

CI menjalankan `fmt -check`, `init -backend=false` dan `validate` pada setiap
PR ke repo ini.
