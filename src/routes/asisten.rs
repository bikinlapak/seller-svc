//! Asisten Lapak: beranda, thread, pengaturan, dan jalur aksi.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::contract::{AsistenHome, KartuHasil, PengaturanAi, Thread};
use crate::{AppError, AppState, seed};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/home", get(home))
        .route("/assistant/thread", get(thread))
        .route("/assistant/settings", get(settings))
        .route("/actions/{token}", post(aksi))
}

/// Agregat beranda: satu permintaan untuk satu layar.
async fn home() -> Json<AsistenHome> {
    Json(seed::home())
}

#[derive(Deserialize)]
struct ThreadQuery {
    topik: String,
}

async fn thread(
    State(state): State<AppState>,
    Query(q): Query<ThreadQuery>,
) -> Result<Json<Thread>, AppError> {
    seed::thread(&q.topik, &state.action_keys)?
        .map(Json)
        .ok_or(AppError::NotFound)
}

async fn settings() -> Json<PengaturanAi> {
    Json(seed::pengaturan_ai())
}

#[derive(Deserialize)]
struct AksiBody {
    jumlah: i64,
}

/// Jalankan aksi dari kartu saran.
///
/// Ini satu-satunya endpoint yang mengubah keadaan atas usul asisten, dan
/// urutannya penting: verifikasi tanda tangan dulu, lalu klaim sekali pakai
/// beserta batas jumlah, baru mutasi. Tidak ada jalur lain — agent tidak
/// memegang kredensial tulis sama sekali.
async fn aksi(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(body): Json<AksiBody>,
) -> Result<Json<KartuHasil>, AppError> {
    let claims = state.action_keys.verify(&token)?;

    // TODO(Fase 6): tenant datang dari sesi penjual, bukan konstanta.
    let tenant = seed::TENANT;
    state.tokens.klaim(&claims, tenant, body.jumlah)?;

    tracing::info!(
        tenant = %tenant,
        jti = %claims.jti,
        sug = %claims.sug,
        act = %claims.act,
        jumlah = body.jumlah,
        batas = claims.max_qty,
        "aksi disetujui"
    );

    // TODO(Fase 6): createPurchaseOrder + submitPurchaseOrder lewat
    // state.works, lalu tulis result card + decision.recorded ke outbox.
    let jumlah = body.jumlah;
    let satuan = &claims.satuan;
    let pesan =
        format!("Halo CV Sumber Kopi, kami pesan Kopi Arabika {jumlah} {satuan} (PO-0771).");

    Ok(Json(KartuHasil {
        judul: "PO-0771 dibuat".into(),
        detail: format!("{jumlah} {satuan} · CV Sumber Kopi · tiba Rabu"),
        wa_url: format!("https://wa.me/?text={}", urlencode(&pesan)),
        po_href: "/stok/st1".into(),
    }))
}

/// Percent-encoding untuk query string. Cukup untuk teks WhatsApp; kalau nanti
/// perlu lebih, pakai crate `form_urlencoded`.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::urlencode;

    #[test]
    fn urlencode_menyandi_spasi_dan_non_ascii() {
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("PO-0771."), "PO-0771.");
        // Bahasa Indonesia aman, tapi pastikan multibyte tidak rusak.
        assert_eq!(urlencode("±"), "%C2%B1");
    }
}
