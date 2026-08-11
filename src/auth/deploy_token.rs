//! Token deploy — kredensial `POST /api/v1/deploy`, satu per app (`docs/plan.md`
//! Fase 2 Q1). Pola SAMA seperti password (`auth/password.rs`): token acak
//! ditampilkan SEKALI saat dibuat, disimpan sebagai hash argon2, dicocokkan
//! satu arah — TIDAK PERNAH didekripsi kembali (beda kelas dari secret
//! `age` seperti private key SSH atau token registry).

use anyhow::{Context, Result};
use rand::RngExt;
use rand::distr::Alphanumeric;

use super::password::{hash_password, verify_password};

/// Panjang token — sama seperti token sesi/CSRF (`auth/session.rs`), token
/// buram 32 karakter alfanumerik ~190 bit entropi.
const TOKEN_LEN: usize = 32;

/// Prefiks tampilan supaya token deploy gampang dikenali dari token
/// lain kalau operator menaruhnya di banyak tempat (mis. secret CI) —
/// murni kosmetik, tidak mengurangi entropi (32 karakter acak tetap penuh
/// SETELAH prefiks).
const TOKEN_PREFIX: &str = "mengdep_deploy_";

/// Generate token baru dalam bentuk PLAINTEXT — dikembalikan ke pemanggil
/// (route handler) untuk ditampilkan SEKALI ke pengguna. Pemanggil WAJIB
/// memanggil `hash` sebelum menyimpan; nilai plaintext ini tidak pernah
/// disimpan di db (invariant §3 no.11 — secret tidak pernah dikembalikan
/// API setelah disimpan; di sini berlaku simetris: begitu dibuat, hanya
/// hash-nya yang menetap).
pub fn generate() -> String {
    let acak: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(TOKEN_LEN)
        .map(char::from)
        .collect();
    format!("{TOKEN_PREFIX}{acak}")
}

/// Hash token plaintext untuk disimpan ke `deploy_tokens.token_hash`.
/// Reuse `hash_password` (Argon2id) — token deploy dan password sama-sama
/// kredensial satu-arah, tidak ada alasan memakai skema hash berbeda.
pub fn hash(token: &str) -> Result<String> {
    hash_password(token).context("hash token deploy")
}

/// Verifikasi token plaintext dari header `Authorization: Bearer` terhadap
/// hash tersimpan. `Ok(false)` untuk tidak cocok (bukan error) — pemanggil
/// memetakan ke 401 generik, tidak pernah membedakan "token tidak ada" dari
/// "token salah".
pub fn verify(token: &str, stored_hash: &str) -> Result<bool> {
    verify_password(token, stored_hash).context("verifikasi token deploy")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_menghasilkan_token_dengan_prefiks_dan_panjang_yang_diharapkan() {
        let token = generate();
        assert!(token.starts_with(TOKEN_PREFIX));
        assert_eq!(
            token.len(),
            TOKEN_PREFIX.len() + TOKEN_LEN,
            "panjang bagian acak harus tetap {TOKEN_LEN} karakter"
        );
    }

    #[test]
    fn generate_dua_kali_menghasilkan_token_berbeda() {
        assert_ne!(generate(), generate());
    }

    #[test]
    fn hash_lalu_verify_dengan_token_benar_cocok() {
        let token = generate();
        let hashed = hash(&token).expect("hash harus sukses");
        assert!(verify(&token, &hashed).expect("verify harus sukses"));
    }

    #[test]
    fn verify_dengan_token_salah_tidak_cocok() {
        let token = generate();
        let hashed = hash(&token).expect("hash harus sukses");
        assert!(!verify("token-ngawur-sama-sekali", &hashed).expect("verify harus sukses"));
    }
}
