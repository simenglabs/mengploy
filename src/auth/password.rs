//! Hash dan verifikasi password dengan Argon2.

use anyhow::{Context, Result};
use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};

/// Hash password mentah menjadi string PHC (`$argon2id$...`) yang aman
/// disimpan di kolom `settings.value`.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| anyhow::anyhow!("hash argon2 gagal: {err}"))
        .context("hash password")?;

    Ok(hash.to_string())
}

/// Verifikasi password mentah terhadap hash PHC tersimpan. Mengembalikan
/// `true` kalau cocok, `false` kalau tidak — tidak pernah membedakan alasan
/// spesifik ke pemanggil (invariant 7: pesan gagal login harus generik).
pub fn verify_password(password: &str, stored_hash: &str) -> Result<bool> {
    let parsed_hash = PasswordHash::new(stored_hash).context("parsing hash password tersimpan")?;

    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_lalu_verify_dengan_password_benar_cocok() {
        let hash = hash_password("kata-sandi-rahasia").expect("hash harus sukses");
        assert!(verify_password("kata-sandi-rahasia", &hash).expect("verify harus sukses"));
    }

    #[test]
    fn verify_dengan_password_salah_tidak_cocok() {
        let hash = hash_password("kata-sandi-rahasia").expect("hash harus sukses");
        assert!(!verify_password("password-salah", &hash).expect("verify harus sukses"));
    }

    #[test]
    fn hash_yang_sama_menghasilkan_output_berbeda_karena_salt_acak() {
        let hash1 = hash_password("sama").expect("hash harus sukses");
        let hash2 = hash_password("sama").expect("hash harus sukses");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn verify_dengan_hash_tidak_valid_mengembalikan_error() {
        let result = verify_password("apa saja", "bukan-hash-phc-valid");
        assert!(result.is_err());
    }
}
