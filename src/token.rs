//! Token aksi: satu-satunya cara saran asisten berubah menjadi tulisan.
//!
//! Aturan FR-18/FR-27: tidak ada jalur AI yang boleh menulis. Agent hanya
//! boleh *menandatangani* usulan; tulisan sesungguhnya baru terjadi setelah
//! penjual mengetuk, dan executor memverifikasi token itu lebih dulu.
//!
//! Token berbentuk JWT EdDSA (Ed25519) supaya bisa dibaca saat demo dan saat
//! menelusuri masalah: tempel ke decoder mana pun dan klaimnya terlihat.
//! Yang diverifikasi, berurutan:
//!
//! 1. tanda tangan dan `aud`/`iss`,
//! 2. `exp` — token kedaluwarsa ditolak,
//! 3. `sub` — tenant pada token harus sama dengan tenant pemanggil,
//! 4. `jti` — sekali pakai; klaim kedua ditolak,
//! 5. `max_qty` — jumlah final tidak boleh melebihi yang diusulkan asisten.
//!
//! Urutannya disengaja: yang murah dan menentukan lebih dulu, dan `jti` baru
//! ditandai terpakai setelah semua pemeriksaan lain lolos.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::AppError;

const ISS: &str = "seller-svc";
const AUD: &str = "passtok-executor";

/// Umur token saran. Cukup untuk dipikirkan sambil membuka toko, tidak cukup
/// untuk disetujui minggu depan dengan harga minggu lalu.
pub const UMUR_DETIK: i64 = 60 * 60 * 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionClaims {
    pub iss: String,
    pub aud: String,
    /// Tenant (toko) pemilik saran ini.
    pub sub: String,
    /// Id unik token; dipakai untuk sekali pakai.
    pub jti: String,
    pub iat: i64,
    pub exp: i64,
    /// Aksi yang diizinkan, mis. `buat_po`.
    pub act: String,
    /// Saran yang mendasari aksi ini.
    pub sug: String,
    /// Batas atas jumlah. Penjual boleh menurunkan, tidak menaikkan.
    pub max_qty: i64,
    pub satuan: String,
}

pub fn sekarang() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// Kunci penandatangan. Privat hanya ada di penanda tangan (agent); executor
/// cukup memegang kunci publik.
pub struct ActionKeys {
    enc: EncodingKey,
    dec: DecodingKey,
}

impl ActionKeys {
    /// `pkcs8` = kunci privat Ed25519 (DER), `public` = 32 byte kunci publik.
    pub fn new(pkcs8: &[u8], public: &[u8]) -> Self {
        Self {
            enc: EncodingKey::from_ed_der(pkcs8),
            dec: DecodingKey::from_ed_der(public),
        }
    }

    /// Bikin pasangan kunci sekali jalan. Hanya untuk dev dan tes: di staging
    /// dan produksi kuncinya dari Secret Manager, supaya token tidak hangus
    /// setiap kali layanan di-restart.
    pub fn ephemeral() -> Result<Self, AppError> {
        let rng = ring::rand::SystemRandom::new();
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng)
            .map_err(|_| AppError::Config("gagal membuat kunci Ed25519".into()))?;
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(doc.as_ref())
            .map_err(|_| AppError::Config("kunci Ed25519 tidak sah".into()))?;
        use ring::signature::KeyPair;
        Ok(Self::new(doc.as_ref(), pair.public_key().as_ref()))
    }

    pub fn sign(&self, claims: &ActionClaims) -> Result<String, AppError> {
        encode(&Header::new(Algorithm::EdDSA), claims, &self.enc)
            .map_err(|e| AppError::Config(format!("gagal menandatangani token: {e}")))
    }

    /// Verifikasi tanda tangan, `exp`, `aud` dan `iss`. Belum memeriksa tenant,
    /// sekali pakai, atau batas jumlah — itu tugas [`TokenGuard::klaim`].
    pub fn verify(&self, token: &str) -> Result<ActionClaims, AppError> {
        let mut v = Validation::new(Algorithm::EdDSA);
        v.set_audience(&[AUD]);
        v.set_issuer(&[ISS]);
        v.set_required_spec_claims(&["exp", "aud", "iss", "sub"]);

        decode::<ActionClaims>(token, &self.dec, &v)
            .map(|d| d.claims)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => AppError::TokenExpired,
                _ => AppError::TokenInvalid,
            })
    }
}

/// Pembuat klaim untuk satu saran.
pub fn klaim_saran(tenant: &str, suggestion: &str, max_qty: i64, satuan: &str) -> ActionClaims {
    let iat = sekarang();
    ActionClaims {
        iss: ISS.into(),
        aud: AUD.into(),
        sub: tenant.into(),
        jti: ulid::Ulid::new().to_string(),
        iat,
        exp: iat + UMUR_DETIK,
        act: "buat_po".into(),
        sug: suggestion.into(),
        max_qty,
        satuan: satuan.into(),
    }
}

/// Catatan `jti` yang sudah terpakai.
///
/// Sementara di memori. Begitu skema `passtok` ada, ini menjadi tabel
/// `action_tokens` dan `used_at` ditulis **dalam transaksi yang sama** dengan
/// mutasi ke worksh.io — supaya sekali pakai tetap berlaku kalau proses mati
/// di tengah jalan. Sifat yang harus dipertahankan ada di tes:
/// dua ketukan bersamaan hanya boleh menghasilkan satu tulisan.
#[derive(Default)]
pub struct TokenGuard {
    terpakai: Mutex<HashMap<String, i64>>,
}

impl TokenGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Klaim token untuk dipakai satu kali.
    ///
    /// Mengembalikan `Ok` hanya untuk pemanggil pertama; pemanggil berikutnya
    /// dengan `jti` yang sama mendapat [`AppError::TokenUsed`].
    pub fn klaim(&self, claims: &ActionClaims, tenant: &str, jumlah: i64) -> Result<(), AppError> {
        if claims.sub != tenant {
            // Token milik toko lain: perlakukan seperti token palsu.
            return Err(AppError::TokenInvalid);
        }
        if jumlah < 1 {
            return Err(AppError::BadRequest("jumlah harus minimal 1".into()));
        }
        if jumlah > claims.max_qty {
            return Err(AppError::QtyAboveCap {
                diminta: jumlah,
                batas: claims.max_qty,
            });
        }

        let mut terpakai = self
            .terpakai
            .lock()
            .map_err(|_| AppError::Config("kunci token rusak".into()))?;

        // Buang catatan yang tokennya sudah kedaluwarsa; tidak ada gunanya
        // menyimpan jti yang tidak bisa dipakai lagi.
        let now = sekarang();
        terpakai.retain(|_, exp| *exp > now);

        if terpakai.insert(claims.jti.clone(), claims.exp).is_some() {
            return Err(AppError::TokenUsed);
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn jumlah_terpakai(&self) -> usize {
        self.terpakai.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENANT: &str = "toko-kopi-jasa";

    fn keys() -> ActionKeys {
        ActionKeys::ephemeral().unwrap()
    }

    #[test]
    fn token_bolak_balik_utuh() {
        let k = keys();
        let asli = klaim_saran(TENANT, "sug-1", 12, "kg");
        let token = k.sign(&asli).unwrap();
        assert_eq!(k.verify(&token).unwrap(), asli);
    }

    #[test]
    fn tanda_tangan_kunci_lain_ditolak() {
        let token = keys()
            .sign(&klaim_saran(TENANT, "sug-1", 12, "kg"))
            .unwrap();
        assert!(matches!(keys().verify(&token), Err(AppError::TokenInvalid)));
    }

    #[test]
    fn token_diutak_atik_ditolak() {
        let k = keys();
        let token = k.sign(&klaim_saran(TENANT, "sug-1", 12, "kg")).unwrap();
        // Ubah satu karakter payload: tanda tangan tidak lagi cocok.
        let mut bagian: Vec<&str> = token.split('.').collect();
        let payload = bagian[1].to_string();
        let diubah = format!("{}A", &payload[..payload.len() - 1]);
        bagian[1] = &diubah;
        assert!(matches!(
            k.verify(&bagian.join(".")),
            Err(AppError::TokenInvalid)
        ));
    }

    #[test]
    fn token_kedaluwarsa_ditolak() {
        let k = keys();
        let mut c = klaim_saran(TENANT, "sug-1", 12, "kg");
        c.iat = sekarang() - 7200;
        c.exp = sekarang() - 3600;
        let token = k.sign(&c).unwrap();
        assert!(matches!(k.verify(&token), Err(AppError::TokenExpired)));
    }

    #[test]
    fn token_sekali_pakai() {
        let g = TokenGuard::new();
        let c = klaim_saran(TENANT, "sug-1", 12, "kg");
        assert!(g.klaim(&c, TENANT, 9).is_ok());
        assert!(matches!(g.klaim(&c, TENANT, 9), Err(AppError::TokenUsed)));
    }

    #[test]
    fn token_tenant_lain_ditolak() {
        let g = TokenGuard::new();
        let c = klaim_saran(TENANT, "sug-1", 12, "kg");
        assert!(matches!(
            g.klaim(&c, "toko-lain", 9),
            Err(AppError::TokenInvalid)
        ));
        // Ditolak sebelum jti tercatat, jadi token aslinya masih bisa dipakai
        // pemiliknya.
        assert_eq!(g.jumlah_terpakai(), 0);
        assert!(g.klaim(&c, TENANT, 9).is_ok());
    }

    #[test]
    fn jumlah_di_atas_batas_ditolak() {
        let g = TokenGuard::new();
        let c = klaim_saran(TENANT, "sug-1", 12, "kg");
        assert!(matches!(
            g.klaim(&c, TENANT, 13),
            Err(AppError::QtyAboveCap {
                diminta: 13,
                batas: 12
            })
        ));
        // Dan tokennya belum hangus.
        assert!(g.klaim(&c, TENANT, 12).is_ok());
    }

    #[test]
    fn jumlah_boleh_diturunkan() {
        for jumlah in [1, 5, 12] {
            let g = TokenGuard::new();
            let c = klaim_saran(TENANT, "sug-1", 12, "kg");
            assert!(g.klaim(&c, TENANT, jumlah).is_ok(), "jumlah={jumlah}");
        }
    }

    #[test]
    fn jumlah_nol_atau_negatif_ditolak() {
        let g = TokenGuard::new();
        let c = klaim_saran(TENANT, "sug-1", 12, "kg");
        for jumlah in [0, -1] {
            assert!(
                matches!(g.klaim(&c, TENANT, jumlah), Err(AppError::BadRequest(_))),
                "jumlah={jumlah}"
            );
        }
    }

    /// Dua ketukan bersamaan pada token yang sama: tepat satu boleh menang.
    #[test]
    fn dua_ketukan_bersamaan_hanya_satu_menang() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        for _ in 0..50 {
            let g = Arc::new(TokenGuard::new());
            let c = Arc::new(klaim_saran(TENANT, "sug-1", 12, "kg"));
            let menang = Arc::new(AtomicUsize::new(0));

            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let (g, c, menang) = (g.clone(), c.clone(), menang.clone());
                    std::thread::spawn(move || {
                        if g.klaim(&c, TENANT, 9).is_ok() {
                            menang.fetch_add(1, Ordering::SeqCst);
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
            assert_eq!(menang.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn jti_kedaluwarsa_dibersihkan() {
        let g = TokenGuard::new();
        let mut lama = klaim_saran(TENANT, "sug-lama", 12, "kg");
        lama.exp = sekarang() - 1;
        // Token lama tercatat...
        g.terpakai
            .lock()
            .unwrap()
            .insert(lama.jti.clone(), lama.exp);
        // ...lalu klaim berikutnya menyapunya.
        let baru = klaim_saran(TENANT, "sug-baru", 12, "kg");
        g.klaim(&baru, TENANT, 1).unwrap();
        assert_eq!(g.jumlah_terpakai(), 1);
    }
}
