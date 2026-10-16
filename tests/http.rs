//! Tes router lewat `tower::ServiceExt::oneshot` — tanpa membuka port.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use seller_svc::{AppState, Config, build_router};
use serde_json::Value;
use tower::ServiceExt;

fn state() -> AppState {
    AppState::new(Config::from_lookup(|_| None).unwrap()).unwrap()
}

/// Satu instansi layanan per tes.
///
/// Harus dibagi, bukan dibuat ulang per permintaan: kunci penandatangan dan
/// catatan token sekali pakai ada di `AppState`, jadi dua instansi berarti dua
/// pasang kunci dan token dari yang satu ditolak yang lain.
struct Svc {
    app: axum::Router,
}

impl Svc {
    fn baru() -> Self {
        Self {
            app: build_router(state()),
        }
    }

    async fn kirim(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.kirim(Request::get(path).body(Body::empty()).unwrap())
            .await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.kirim(
            Request::post(path)
                .header("content-type", "application/json")
                .header("idempotency-key", "tes")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    /// Ambil token saran dari thread, seperti yang dilakukan seller-app.
    async fn token_saran(&self) -> String {
        let (_, json) = self.get("/v1/assistant/thread?topik=pesan").await;
        json["pesan"][0]["kartu"]["token"]
            .as_str()
            .expect("kartu saran harus membawa token")
            .to_string()
    }
}

#[tokio::test]
async fn healthz_returns_ok() {
    let svc = Svc::baru();
    let (status, json) = svc.get("/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn home_punya_bentuk_yang_dipakai_seller_app() {
    let svc = Svc::baru();
    let (status, json) = svc.get("/v1/home").await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(json["toko"]["nama"], "Toko Kopi Jasa");
    assert!(json["toko"]["sapaan"].is_string());

    let perhatian = json["perhatian"].as_array().unwrap();
    assert_eq!(perhatian.len(), 3);
    for p in perhatian {
        // Setiap baris perhatian harus menunjuk thread yang benar-benar ada.
        let topik = p["href"].as_str().unwrap().trim_start_matches("/asisten/");
        let (s, _) = svc
            .get(&format!("/v1/assistant/thread?topik={topik}"))
            .await;
        assert_eq!(s, StatusCode::OK, "href {topik} tidak punya thread");
    }

    for p in json["perintah"].as_array().unwrap() {
        let topik = p["href"].as_str().unwrap().trim_start_matches("/asisten/");
        let (s, _) = svc
            .get(&format!("/v1/assistant/thread?topik={topik}"))
            .await;
        assert_eq!(s, StatusCode::OK, "perintah {topik} tidak punya thread");
    }
}

#[tokio::test]
async fn thread_tidak_dikenal_404() {
    let svc = Svc::baru();
    let (status, json) = svc.get("/v1/assistant/thread?topik=entah").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "not_found");
}

#[tokio::test]
async fn thread_tanpa_topik_400() {
    let svc = Svc::baru();
    let (status, _) = svc.get("/v1/assistant/thread").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn thread_pesan_membawa_kartu_saran_bertoken() {
    let svc = Svc::baru();
    let (status, json) = svc.get("/v1/assistant/thread?topik=pesan").await;
    assert_eq!(status, StatusCode::OK);

    let pesan = json["pesan"].as_array().unwrap();
    let kartu = &pesan[0];
    assert_eq!(kartu["tipe"], "kartu_saran");
    assert_eq!(kartu["penulis"], "agent");
    assert_eq!(kartu["kartu"]["jumlah"], 12);
    assert_eq!(kartu["kartu"]["satuan"], "kg");

    // Token adalah JWT sungguhan, bukan placeholder.
    let token = kartu["kartu"]["token"].as_str().unwrap();
    assert_eq!(token.split('.').count(), 3, "token bukan JWT: {token}");
}

#[tokio::test]
async fn thread_sisa_memakai_bentuk_camel_case() {
    let svc = Svc::baru();
    let (status, json) = svc.get("/v1/assistant/thread?topik=sisa").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["sumber"], "Gemini vision");

    let pesan = json["pesan"].as_array().unwrap();
    // seller-app membaca `fotoLabel`, bukan `foto_label`.
    assert!(pesan[0]["fotoLabel"].is_string());
    assert_eq!(pesan[0]["kotak"].as_array().unwrap().len(), 3);

    let kartu = &pesan[1]["kartu"];
    let barang = kartu["barang"].as_array().unwrap();
    assert!(barang[0]["kondisiLabel"].is_string());

    // Aturan tujuan: yang lewat tanggal tidak menawarkan donasi atau jual,
    // dan "Buang" tidak pernah terpilih otomatis.
    for b in barang {
        let tujuan = b["tujuan"].as_array().unwrap();
        assert_eq!(
            tujuan.iter().filter(|t| t["dipilih"] == true).count(),
            1,
            "harus tepat satu tujuan terpilih"
        );
        for t in tujuan {
            if t["id"] == "buang" {
                assert_eq!(t["dipilih"], false, "Buang tidak boleh terpilih otomatis");
            }
        }
        if b["kondisi"] == "lewat" {
            let ids: Vec<&str> = tujuan.iter().map(|t| t["id"].as_str().unwrap()).collect();
            assert!(!ids.contains(&"donasi"), "lewat tanggal tidak boleh donasi");
            assert!(
                !ids.contains(&"spesial"),
                "lewat tanggal tidak boleh dijual"
            );
            assert!(b["peringatan"].is_string(), "harus ada peringatan");
        }
    }
}

#[tokio::test]
async fn produk_dan_stok_punya_bentuk_yang_benar() {
    let svc = Svc::baru();
    let (status, json) = svc.get("/v1/products").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["total"].is_number());
    for p in json["produk"].as_array().unwrap() {
        assert!(p["stokLabel"].is_string());
        assert!(["aman", "hampir", "lewat"].contains(&p["stokKondisi"].as_str().unwrap()));
        // Produk tanpa harga wajib punya subjudul yang menjelaskan.
        if p["hargaLabel"].is_null() {
            assert!(p["subjudul"].is_string(), "{}", p["nama"]);
        }
    }

    let svc = Svc::baru();
    let (status, json) = svc.get("/v1/stock").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["ringkasan"]["hampirHabis"].is_number());
    for b in json["baris"].as_array().unwrap() {
        let persen = b["persen"].as_i64().unwrap();
        assert!(
            (0..=100).contains(&persen),
            "persen di luar rentang: {persen}"
        );
        assert!(b["jumlahLabel"].is_string());
    }
}

#[tokio::test]
async fn pengaturan_ai_menandai_prediksi_sebagai_hitungan_sistem() {
    let svc = Svc::baru();
    let (status, json) = svc.get("/v1/assistant/settings").await;
    assert_eq!(status, StatusCode::OK);

    let mode: Vec<&str> = json["mode"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert!(mode.contains(&json["modeDipilih"].as_str().unwrap()));

    let prediksi = json["model"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["tugas"] == "Prediksi stok")
        .expect("model prediksi stok harus ada");
    assert_eq!(prediksi["sistem"], true);
}

/* ---------- Jalur aksi: satu-satunya jalan menulis ---------- */

#[tokio::test]
async fn aksi_menyetujui_jumlah_yang_diturunkan() {
    let svc = Svc::baru();
    let token = svc.token_saran().await;
    let (status, json) = svc
        .post(
            &format!("/v1/actions/{token}"),
            serde_json::json!({ "jumlah": 9 }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["judul"], "PO-0771 dibuat");
    assert!(json["detail"].as_str().unwrap().contains("9 kg"));
    // Pesan ke supplier membawa jumlah final, bukan yang diusulkan.
    let wa = json["waUrl"].as_str().unwrap();
    assert!(
        wa.contains("9%20kg"),
        "waUrl tidak membawa jumlah final: {wa}"
    );
    assert!(json["poHref"].is_string());
}

#[tokio::test]
async fn aksi_menolak_jumlah_di_atas_batas() {
    let svc = Svc::baru();
    let token = svc.token_saran().await;
    let (status, json) = svc
        .post(
            &format!("/v1/actions/{token}"),
            serde_json::json!({ "jumlah": 13 }),
        )
        .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json["error"]["code"], "qty_above_cap");
}

#[tokio::test]
async fn aksi_menolak_token_palsu() {
    let svc = Svc::baru();
    let (status, json) = svc
        .post(
            "/v1/actions/bukan.sebuah.token",
            serde_json::json!({ "jumlah": 9 }),
        )
        .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["error"]["code"], "token_invalid");
}

#[tokio::test]
async fn aksi_menolak_jumlah_nol() {
    let svc = Svc::baru();
    let token = svc.token_saran().await;
    let (status, json) = svc
        .post(
            &format!("/v1/actions/{token}"),
            serde_json::json!({ "jumlah": 0 }),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"]["code"], "bad_request");
}

/// Token sekali pakai: ketukan kedua pada token yang sama harus ditolak.
#[tokio::test]
async fn aksi_token_hanya_sekali_pakai() {
    let svc = Svc::baru();
    let token = svc.token_saran().await;
    let path = format!("/v1/actions/{token}");
    let badan = serde_json::json!({ "jumlah": 9 });

    let (pertama, _) = svc.post(&path, badan.clone()).await;
    assert_eq!(pertama, StatusCode::OK);

    let (kedua, json) = svc.post(&path, badan).await;
    assert_eq!(kedua, StatusCode::CONFLICT);
    assert_eq!(json["error"]["code"], "token_used");
}

/// Token yang ditandatangani instansi lain ditolak — kunci per layanan benar
/// dipakai, bukan diabaikan.
#[tokio::test]
async fn aksi_menolak_token_dari_instansi_lain() {
    let a = Svc::baru();
    let b = Svc::baru();
    let token = a.token_saran().await;

    let (status, json) = b
        .post(
            &format!("/v1/actions/{token}"),
            serde_json::json!({ "jumlah": 9 }),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["error"]["code"], "token_invalid");
}

#[tokio::test]
async fn builder_masih_501() {
    let svc = Svc::baru();
    let (status, _) = svc.get("/v1/builder/pages").await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}
