//! Data contoh Toko Kopi Jasa.
//!
//! Ini **stand-in**, bukan sumber kebenaran. worksh.io yang memegang produk,
//! stok, lot dan PO, dan skema `passtok` yang memegang thread serta saran;
//! keduanya belum tersambung (lihat docs/plan/05-phases.md, Fase 2 dan 6).
//! Sampai saat itu modul ini mengisi bentuk respons yang sama persis, supaya
//! seller-app bisa dibangun dan diuji dengan kontrak yang benar.
//!
//! Isinya sengaja sama dengan kanvas desain dan dengan
//! `seller-app/src/lib/mock/fixtures.ts`.

use crate::AppError;
use crate::contract::*;
use crate::token::{ActionKeys, klaim_saran};

/// Tenant demo. Nanti datang dari sesi penjual yang login.
pub const TENANT: &str = "toko-kopi-jasa";

pub fn toko() -> Toko {
    Toko {
        nama: "Toko Kopi Jasa".into(),
        pemilik: "Rima".into(),
        sapaan: "Pagi, Rima.".into(),
    }
}

pub fn home() -> AsistenHome {
    AsistenHome {
        toko: toko(),
        perhatian: vec![
            PerluPerhatian {
                id: "p1".into(),
                judul: "Kopi Arabika habis Kamis".into(),
                detail: "Saran: pesan 12 kg".into(),
                ikon: Ikon::Stok,
                href: "/asisten/pesan".into(),
            },
            PerluPerhatian {
                id: "p2".into(),
                judul: "14 croissant kedaluwarsa besok".into(),
                detail: "8 diperkirakan tidak laku".into(),
                ikon: Ikon::Kedaluwarsa,
                href: "/asisten/kedaluwarsa".into(),
            },
            PerluPerhatian {
                id: "p3".into(),
                judul: "Jemput donasi 21.30".into(),
                detail: "[FOOD BANK MITRA]".into(),
                ikon: Ikon::Donasi,
                href: "/asisten/donasi".into(),
            },
        ],
        perintah: vec![
            perintah("Tambah produk dari foto", "produk"),
            perintah("Stok susu oat berapa?", "stok"),
            perintah("Foto sisa hari ini", "sisa"),
            perintah("Ide menu dari stok", "spesial"),
            perintah("Laporan minggu ini", "laporan"),
            perintah("Ubah jam buka", "toko"),
        ],
    }
}

fn perintah(label: &str, topik: &str) -> PerintahCepat {
    PerintahCepat {
        label: label.into(),
        href: format!("/asisten/{topik}"),
    }
}

/// Topik thread yang dikenal. Dipakai juga untuk memvalidasi path.
pub const TOPIK: [&str; 9] = [
    "pesan",
    "sisa",
    "produk",
    "stok",
    "kedaluwarsa",
    "donasi",
    "spesial",
    "laporan",
    "toko",
];

pub fn thread(topik: &str, keys: &ActionKeys) -> Result<Option<Thread>, AppError> {
    let t = match topik {
        "pesan" => thread_pesan(keys)?,
        "sisa" => thread_sisa(),
        "produk" => ringkas(
            "Tambah produk dari foto",
            "Kirim foto produknya, saya buatkan nama, kategori dan deskripsi. Kamu yang menyetujui sebelum tersimpan.",
            None,
        ),
        "stok" => ringkas(
            "Cek & ubah stok",
            "Susu Oat 1 L tinggal 5 L di gudang utama, 2 lot. Mau saya catat penyesuaian?",
            None,
        ),
        "kedaluwarsa" => ringkas(
            "Hampir kedaluwarsa",
            "14 croissant kedaluwarsa besok, 8 diperkirakan tidak laku. Mau pasang diskon bertahap dari 17.00?",
            None,
        ),
        "donasi" => ringkas(
            "Donasi",
            "[FOOD BANK MITRA] bisa jemput 21.30. Sebelum lanjut, ada daftar periksa keamanan pangan yang harus kamu centang.",
            None,
        ),
        "spesial" => ringkas(
            "Menu spesial dari stok",
            "Dari stok yang ada: croissant + susu oat jadi 6 porsi pudding. Harga Rp18.000, balik modal Rp108.000.",
            None,
        ),
        "laporan" => ringkas(
            "Laporan & dampak",
            "Minggu ini omzet Rp12,4 jt, untung Rp3,1 jt. Dari 9 foto sisa, 78% terselamatkan — naik dari 61% minggu lalu.",
            None,
        ),
        "toko" => ringkas(
            "Atur toko",
            "Jam buka sekarang 07.00–21.00. Mau diubah untuk hari apa?",
            None,
        ),
        _ => return Ok(None),
    };
    Ok(Some(t))
}

fn ringkas(topik: &str, sapaan: &str, sumber: Option<&str>) -> Thread {
    Thread {
        topik: topik.into(),
        toko: toko(),
        sumber: sumber.map(Into::into),
        pesan: vec![Pesan::Teks {
            id: "a1".into(),
            penulis: Penulis::Agent,
            waktu: "Sekarang".into(),
            teks: sapaan.into(),
            aksi: None,
        }],
    }
}

/// B3 · Saran pesanan → PO. Jalur demo.
///
/// Tokennya ditandatangani di sini karena agent belum ada. Begitu Fase 6
/// jalan, agent yang menandatangani sekali saat menulis saran, dan tokennya
/// tersimpan bersama pesan — bukan dibuat ulang setiap kali thread dibaca.
fn thread_pesan(keys: &ActionKeys) -> Result<Thread, AppError> {
    let penuh = keys.sign(&klaim_saran(TENANT, "sug-0771", 12, "kg"))?;
    let sebagian = keys.sign(&klaim_saran(TENANT, "sug-0771", 9, "kg"))?;

    Ok(Thread {
        topik: "Saran pesanan".into(),
        toko: toko(),
        sumber: None,
        pesan: vec![
            Pesan::KartuSaran {
                id: "m1".into(),
                penulis: Penulis::Agent,
                waktu: "Senin 07.00".into(),
                kartu: KartuSaran {
                    label: "Saran pesanan".into(),
                    keyakinan: "Keyakinan tinggi".into(),
                    judul: "Kopi Arabika 1 kg · pesan 12 kg".into(),
                    alasan: "Stok 4 kg, terjual 1,6 kg/hari, habis Kamis. CV Sumber Kopi kirim 2 hari, tiba Rabu."
                        .into(),
                    jumlah: 12,
                    satuan: "kg".into(),
                    token: penuh,
                },
            },
            Pesan::Teks {
                id: "m2".into(),
                penulis: Penulis::Staff,
                waktu: "07.03".into(),
                teks: "kenapa 12? minggu depan libur 2 hari".into(),
                aksi: None,
            },
            Pesan::Teks {
                id: "m3".into(),
                penulis: Penulis::Agent,
                waktu: "07.04".into(),
                teks: "12 kg untuk 7 hari + cadangan. Kalau tutup 2 hari, 9 kg cukup. Libur saya catat untuk prediksi berikutnya."
                    .into(),
                aksi: Some(vec![
                    AksiInline {
                        label: "Pesan 9 kg".into(),
                        token: Some(sebagian),
                        utama: true,
                    },
                    AksiInline {
                        label: "Tetap 12 kg".into(),
                        token: None,
                        utama: false,
                    },
                ]),
            },
        ],
    })
}

/// B5 · Foto sisa → AI vision → tujuan.
fn thread_sisa() -> Thread {
    Thread {
        topik: "Foto sisa".into(),
        toko: toko(),
        sumber: Some("Gemini vision".into()),
        pesan: vec![
            Pesan::Foto {
                id: "s1".into(),
                penulis: Penulis::Staff,
                waktu: "21.10".into(),
                foto_label: "[FOTO NAMPAN SISA]".into(),
                kotak: vec![
                    KotakDeteksi {
                        x: 5.0,
                        y: 15.0,
                        w: 41.0,
                        h: 47.0,
                        yakin: true,
                    },
                    KotakDeteksi {
                        x: 53.0,
                        y: 20.0,
                        w: 39.0,
                        h: 57.0,
                        yakin: true,
                    },
                    KotakDeteksi {
                        x: 12.0,
                        y: 67.0,
                        w: 32.0,
                        h: 23.0,
                        yakin: false,
                    },
                ],
            },
            Pesan::KartuSisa {
                id: "s2".into(),
                penulis: Penulis::Agent,
                waktu: "21.11".into(),
                kartu: KartuSisa {
                    label: "Sisa hari ini · 3 barang".into(),
                    nilai: "±Rp167 rb".into(),
                    barang: vec![
                        BarangSisa {
                            id: "b1".into(),
                            nama: "Croissant Butter".into(),
                            jumlah: 3,
                            satuan: "pcs".into(),
                            kondisi: Kondisi::Aman,
                            kondisi_label: "masih aman · 96%".into(),
                            tujuan: vec![
                                tujuan("donasi", "Donasi", true),
                                tujuan("spesial", "Menu spesial", false),
                                tujuan("staf", "Makan staf", false),
                            ],
                            peringatan: None,
                        },
                        BarangSisa {
                            id: "b2".into(),
                            nama: "Susu Oat 1 L".into(),
                            jumlah: 2,
                            satuan: "L".into(),
                            kondisi: Kondisi::Lewat,
                            kondisi_label: "lewat tanggal · 92%".into(),
                            // Lewat tanggal: donasi dan jual tidak ditawarkan
                            // sama sekali, bukan sekadar dinonaktifkan.
                            tujuan: vec![
                                tujuan("kompos", "Kompos / maggot", true),
                                tujuan("buang", "Buang", false),
                            ],
                            peringatan: Some(
                                "Lewat tanggal: tidak boleh dijual atau didonasikan.".into(),
                            ),
                        },
                    ],
                    ragu: Some(BarangRagu {
                        keyakinan: 58,
                        kandidat: vec![
                            Kandidat {
                                id: "k1".into(),
                                nama: "Roti Tawar Gandum".into(),
                            },
                            Kandidat {
                                id: "k2".into(),
                                nama: "Roti Sobek".into(),
                            },
                        ],
                    }),
                    catatan: "Foto disimpan sebagai bukti · lokasi foto dihapus".into(),
                },
            },
        ],
    }
}

fn tujuan(id: &str, label: &str, dipilih: bool) -> Tujuan {
    Tujuan {
        id: id.into(),
        label: label.into(),
        dipilih,
    }
}

pub fn produk() -> DaftarProduk {
    DaftarProduk {
        total: 24,
        kategori: ["Semua", "Minuman", "Roti", "Bahan", "Draf"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        produk: vec![
            Produk {
                id: "pr1".into(),
                nama: "Kopi Susu Gula Aren 250 ml".into(),
                harga: Some(35_000),
                harga_label: Some("Rp35.000".into()),
                subjudul: None,
                kategori: "Minuman".into(),
                stok_label: "20 stok".into(),
                stok_kondisi: Kondisi::Aman,
                status: "Baru".into(),
                status_penting: false,
                foto_warna: "#EAD9C2".into(),
            },
            Produk {
                id: "pr2".into(),
                nama: "Croissant Butter (isi 4)".into(),
                harga: Some(60_000),
                harga_label: Some("Rp60.000".into()),
                subjudul: None,
                kategori: "Roti".into(),
                stok_label: "14 · exp besok".into(),
                stok_kondisi: Kondisi::Hampir,
                status: "Diskon 17.00".into(),
                status_penting: true,
                foto_warna: "#E8DCC8".into(),
            },
            Produk {
                id: "pr3".into(),
                nama: "Kopi Arabika 1 kg".into(),
                harga: Some(210_000),
                harga_label: Some("Rp210.000".into()),
                subjudul: None,
                kategori: "Bahan".into(),
                stok_label: "4 kg".into(),
                stok_kondisi: Kondisi::Lewat,
                status: "PO dikirim".into(),
                status_penting: false,
                foto_warna: "#DCCFBD".into(),
            },
            Produk {
                id: "pr4".into(),
                nama: "Susu Oat 1 L".into(),
                harga: None,
                harga_label: None,
                subjudul: Some("Bahan · tidak dijual".into()),
                kategori: "Bahan".into(),
                stok_label: "5 L".into(),
                stok_kondisi: Kondisi::Hampir,
                status: "2 lot".into(),
                status_penting: false,
                foto_warna: "#E6E2D6".into(),
            },
            Produk {
                id: "pr5".into(),
                nama: "Donat Gula Aren".into(),
                harga: Some(15_000),
                harga_label: Some("Rp15.000".into()),
                subjudul: None,
                kategori: "Roti".into(),
                stok_label: "12 stok".into(),
                stok_kondisi: Kondisi::Aman,
                status: "Aktif".into(),
                status_penting: false,
                foto_warna: "#EFE0CB".into(),
            },
        ],
    }
}

pub fn stok() -> DaftarStok {
    DaftarStok {
        lokasi: "Gudang utama".into(),
        ringkasan: RingkasanStok {
            nilai: "Rp6,8 jt".into(),
            hampir_habis: 3,
            hampir_kedaluwarsa: 2,
        },
        baris: vec![
            BarisStok {
                id: "st1".into(),
                nama: "Kopi Arabika 1 kg".into(),
                jumlah_label: "4 kg".into(),
                persen: 18,
                kondisi: Kondisi::Lewat,
                kiri: "Habis Kamis · 2,5 hari".into(),
                kanan: "PO-0771 tiba Rabu".into(),
                kanan_penting: true,
            },
            BarisStok {
                id: "st2".into(),
                nama: "Susu Oat 1 L".into(),
                jumlah_label: "5 L".into(),
                persen: 30,
                kondisi: Kondisi::Hampir,
                kiri: "3 L kedaluwarsa Kamis".into(),
                kanan: "2 lot".into(),
                kanan_penting: false,
            },
            BarisStok {
                id: "st3".into(),
                nama: "Croissant Butter".into(),
                jumlah_label: "14 box".into(),
                persen: 70,
                kondisi: Kondisi::Hampir,
                kiri: "Kedaluwarsa besok · diskon 17.00".into(),
                kanan: "lot L-889".into(),
                kanan_penting: false,
            },
            BarisStok {
                id: "st4".into(),
                nama: "Gula Aren 1 kg".into(),
                jumlah_label: "9 kg".into(),
                persen: 62,
                kondisi: Kondisi::Aman,
                kiri: "Aman · 11 hari".into(),
                kanan: "otomatis disetujui".into(),
                kanan_penting: false,
            },
        ],
    }
}

pub fn pengaturan_ai() -> PengaturanAi {
    PengaturanAi {
        mode: vec![
            ModeAsisten {
                id: "hemat".into(),
                label: "Hemat".into(),
                catatan: "jawab singkat, model kecil".into(),
            },
            ModeAsisten {
                id: "seimbang".into(),
                label: "Seimbang".into(),
                catatan: "bawaan".into(),
            },
            ModeAsisten {
                id: "teliti".into(),
                label: "Teliti".into(),
                catatan: "alasan lebih panjang".into(),
            },
        ],
        mode_dipilih: "seimbang".into(),
        model: vec![
            model("Chat & penjelasan", None, "Gemini Flash", false),
            model("Baca foto produk & sisa", None, "Gemini Pro", false),
            model("Ide menu & listing", None, "Gemini Flash", false),
            // Ditandai sistem: angkanya dari TimesFM lewat BigQuery, bukan
            // dari model bahasa. Penjual berhak tahu bedanya.
            model(
                "Prediksi stok",
                Some("angka dihitung sistem, bukan chat"),
                "TimesFM",
                true,
            ),
        ],
        otonomi: vec![
            Otonomi {
                id: "po-kecil".into(),
                label: "Pesanan kecil otomatis".into(),
                catatan: Some("≤Rp500 rb, sudah 3× disetujui tanpa diubah".into()),
                aktif: true,
            },
            Otonomi {
                id: "diskon".into(),
                label: "Diskon kedaluwarsa otomatis".into(),
                catatan: None,
                aktif: false,
            },
        ],
        bahasa: Bahasa {
            pilihan: ["Indonesia · santai", "Indonesia · formal", "English"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            dipilih: "Indonesia · santai".into(),
        },
        biaya: Biaya {
            batas_label: "Rp150 rb/bln".into(),
            terpakai_persen: 25,
            catatan: "Terpakai 25% bulan ini. Jika habis, asisten pakai mode Hemat.".into(),
        },
        privasi: "Data tokomu tidak dipakai untuk melatih model. Foto sisa disimpan 90 hari sebagai bukti."
            .into(),
    }
}

fn model(tugas: &str, catatan: Option<&str>, nama: &str, sistem: bool) -> ModelTugas {
    ModelTugas {
        tugas: tugas.into(),
        catatan: catatan.map(Into::into),
        nama: nama.into(),
        sistem,
    }
}
