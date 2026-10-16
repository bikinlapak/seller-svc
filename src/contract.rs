//! Bentuk respons yang dikonsumsi seller-app.
//!
//! Berkas ini adalah pasangan Rust dari `seller-app/src/lib/types/api.ts`.
//! Keduanya ditulis tangan karena dua repo tidak bisa berbagi skema saat
//! compile; `tests/contract.rs` menjaga keduanya tetap cocok.
//!
//! Semua field diserialisasi camelCase supaya tidak perlu pemetaan di sisi
//! SvelteKit.

use serde::{Deserialize, Serialize};

/// Rupiah utuh, tanpa sen. Tidak pernah float.
pub type Idr = i64;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kondisi {
    Aman,
    Hampir,
    Lewat,
}

#[derive(Debug, Clone, Serialize)]
pub struct Toko {
    pub nama: String,
    pub pemilik: String,
    /// Sapaan ikut waktu setempat; backend yang memutuskan, bukan browser.
    pub sapaan: String,
}

/* ---------- Beranda asisten ---------- */

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Ikon {
    Stok,
    Kedaluwarsa,
    Donasi,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerluPerhatian {
    pub id: String,
    pub judul: String,
    pub detail: String,
    pub ikon: Ikon,
    pub href: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerintahCepat {
    pub label: String,
    pub href: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AsistenHome {
    pub toko: Toko,
    pub perhatian: Vec<PerluPerhatian>,
    pub perintah: Vec<PerintahCepat>,
}

/* ---------- Thread ---------- */

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Penulis {
    Agent,
    Staff,
    Sistem,
}

#[derive(Debug, Clone, Serialize)]
pub struct KotakDeteksi {
    /// Persen terhadap bidang foto, bukan piksel: tidak tergantung lebar layar.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub yakin: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AksiInline {
    pub label: String,
    /// `None` = tombol sekunder tanpa efek tulis.
    pub token: Option<String>,
    pub utama: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KartuSaran {
    pub label: String,
    pub keyakinan: String,
    pub judul: String,
    pub alasan: String,
    /// Jumlah yang diusulkan. Token aksi membawa batas yang sama; penjual boleh
    /// menurunkan, tidak boleh menaikkan.
    pub jumlah: i64,
    pub satuan: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KartuHasil {
    pub judul: String,
    pub detail: String,
    pub wa_url: String,
    pub po_href: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tujuan {
    pub id: String,
    pub label: String,
    pub dipilih: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BarangSisa {
    pub id: String,
    pub nama: String,
    pub jumlah: i64,
    pub satuan: String,
    pub kondisi: Kondisi,
    pub kondisi_label: String,
    pub tujuan: Vec<Tujuan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peringatan: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Kandidat {
    pub id: String,
    pub nama: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BarangRagu {
    pub keyakinan: i64,
    pub kandidat: Vec<Kandidat>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KartuSisa {
    pub label: String,
    pub nilai: String,
    pub barang: Vec<BarangSisa>,
    pub ragu: Option<BarangRagu>,
    pub catatan: String,
}

/// Satu baris thread. `tipe` memilih komponen yang merender di seller-app.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "tipe", rename_all = "snake_case")]
pub enum Pesan {
    Teks {
        id: String,
        penulis: Penulis,
        waktu: String,
        teks: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        aksi: Option<Vec<AksiInline>>,
    },
    #[serde(rename_all = "camelCase")]
    Foto {
        id: String,
        penulis: Penulis,
        waktu: String,
        foto_label: String,
        kotak: Vec<KotakDeteksi>,
    },
    Tanda {
        id: String,
        penulis: Penulis,
        waktu: String,
        teks: String,
    },
    KartuSaran {
        id: String,
        penulis: Penulis,
        waktu: String,
        kartu: KartuSaran,
    },
    KartuHasil {
        id: String,
        penulis: Penulis,
        waktu: String,
        kartu: KartuHasil,
    },
    KartuSisa {
        id: String,
        penulis: Penulis,
        waktu: String,
        kartu: KartuSisa,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Thread {
    pub topik: String,
    pub toko: Toko,
    /// Ditampilkan di baris meta agen, mis. "Gemini vision".
    pub sumber: Option<String>,
    pub pesan: Vec<Pesan>,
}

/* ---------- Produk ---------- */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Produk {
    pub id: String,
    pub nama: String,
    pub harga: Option<Idr>,
    pub harga_label: Option<String>,
    pub subjudul: Option<String>,
    pub kategori: String,
    pub stok_label: String,
    pub stok_kondisi: Kondisi,
    pub status: String,
    pub status_penting: bool,
    pub foto_warna: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaftarProduk {
    pub total: usize,
    pub kategori: Vec<String>,
    pub produk: Vec<Produk>,
}

/* ---------- Stok ---------- */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RingkasanStok {
    pub nilai: String,
    pub hampir_habis: usize,
    pub hampir_kedaluwarsa: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BarisStok {
    pub id: String,
    pub nama: String,
    pub jumlah_label: String,
    /// 0-100. Dihitung backend dari on-hand terhadap titik pesan ulang.
    pub persen: i64,
    pub kondisi: Kondisi,
    pub kiri: String,
    pub kanan: String,
    pub kanan_penting: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaftarStok {
    pub lokasi: String,
    pub ringkasan: RingkasanStok,
    pub baris: Vec<BarisStok>,
}

/* ---------- Pengaturan AI ---------- */

#[derive(Debug, Clone, Serialize)]
pub struct ModeAsisten {
    pub id: String,
    pub label: String,
    pub catatan: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelTugas {
    pub tugas: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catatan: Option<String>,
    pub nama: String,
    /// `true` = angkanya dihitung sistem, bukan model bahasa.
    pub sistem: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Otonomi {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catatan: Option<String>,
    pub aktif: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bahasa {
    pub pilihan: Vec<String>,
    pub dipilih: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Biaya {
    pub batas_label: String,
    pub terpakai_persen: i64,
    pub catatan: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PengaturanAi {
    pub mode: Vec<ModeAsisten>,
    pub mode_dipilih: String,
    pub model: Vec<ModelTugas>,
    pub otonomi: Vec<Otonomi>,
    pub bahasa: Bahasa,
    pub biaya: Biaya,
    pub privasi: String,
}
