//! Enkripsi/dekripsi secret (private key SSH, token registry) dengan `age`.
//!
//! Kunci `age` (identity x25519) dimuat SEKALI dari file eksternal mode
//! `0600` (invariant PRD §3 nomor 8 — kunci tidak pernah di dalam db atau
//! direktori backup). Ciphertext yang disimpan ke SQLite adalah string
//! armor (`-----BEGIN AGE ENCRYPTED FILE-----...`), aman untuk kolom TEXT.

use std::path::Path;
use std::str::FromStr;

use age::x25519::{Identity, Recipient};
use anyhow::{Context, Result};

/// Pembawa kunci `age` yang dipakai untuk enkripsi (recipient) dan dekripsi
/// (identity). Sengaja TANPA `derive(Debug)` — `Identity` di dalamnya adalah
/// private key; derive otomatis bisa membocorkannya lewat log (invariant 7).
pub struct CryptoKey {
    identity: Identity,
    recipient: Recipient,
}

impl CryptoKey {
    /// Muat kunci `age` dari file. Pemanggil (biasanya `main.rs`) sudah
    /// wajib memverifikasi mode file `0600` sebelum memanggil ini
    /// (`Config::verify_encryption_key_permissions`); fungsi ini tidak
    /// mengulang cek permission supaya tidak ada dua sumber kebenaran.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("baca file kunci enkripsi {}", path.display()))?;

        let identity = Identity::from_str(raw.trim())
            .map_err(|err| anyhow::anyhow!("parsing kunci age gagal: {err}"))
            .context("parse kunci enkripsi")?;

        let recipient = identity.to_public();

        Ok(Self {
            identity,
            recipient,
        })
    }

    /// Enkripsi plaintext, kembalikan string armor (aman untuk kolom TEXT).
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        age::encrypt_and_armor(&self.recipient, plaintext.as_bytes())
            .map_err(|err| anyhow::anyhow!("enkripsi age gagal: {err}"))
            .context("enkripsi secret")
    }

    /// Dekripsi string armor hasil `encrypt`, kembalikan plaintext.
    pub fn decrypt(&self, ciphertext_armor: &str) -> Result<String> {
        let plaintext_bytes = age::decrypt(&self.identity, ciphertext_armor.as_bytes())
            .map_err(|err| anyhow::anyhow!("dekripsi age gagal: {err}"))
            .context("dekripsi secret")?;

        String::from_utf8(plaintext_bytes).context("hasil dekripsi bukan UTF-8 valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bikin file kunci sementara berisi identity x25519 baru, mode 0600.
    fn buat_file_kunci_sementara() -> (std::path::PathBuf, Identity) {
        use age::secrecy::ExposeSecret;
        use std::os::unix::fs::PermissionsExt;

        let identity = Identity::generate();
        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-crypto-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        std::fs::create_dir_all(&dir).expect("bikin direktori sementara harus sukses");
        let path = dir.join("key.txt");
        std::fs::write(&path, identity.to_string().expose_secret())
            .expect("tulis file kunci harus sukses");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("set mode file kunci harus sukses");
        (path, identity)
    }

    fn rand_suffix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("waktu sistem harus valid")
            .as_nanos() as u64
    }

    #[test]
    fn roundtrip_enkripsi_dekripsi_mengembalikan_plaintext_asli() {
        let (path, _identity) = buat_file_kunci_sementara();
        let key = CryptoKey::load_from_file(&path).expect("muat kunci harus sukses");

        let plaintext =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nrahasia\n-----END OPENSSH PRIVATE KEY-----";
        let ciphertext = key.encrypt(plaintext).expect("enkripsi harus sukses");

        assert_ne!(
            ciphertext, plaintext,
            "ciphertext tidak boleh sama dengan plaintext"
        );
        assert!(
            ciphertext.contains("AGE ENCRYPTED FILE"),
            "ciphertext harus berbentuk armor age"
        );

        let decrypted = key.decrypt(&ciphertext).expect("dekripsi harus sukses");
        assert_eq!(decrypted, plaintext);

        let _ = std::fs::remove_dir_all(path.parent().expect("parent harus ada"));
    }

    #[test]
    fn dekripsi_dengan_kunci_berbeda_gagal() {
        let (path1, _id1) = buat_file_kunci_sementara();
        let (path2, _id2) = buat_file_kunci_sementara();
        let key1 = CryptoKey::load_from_file(&path1).expect("muat kunci 1 harus sukses");
        let key2 = CryptoKey::load_from_file(&path2).expect("muat kunci 2 harus sukses");

        let ciphertext = key1.encrypt("data rahasia").expect("enkripsi harus sukses");
        let hasil = key2.decrypt(&ciphertext);

        assert!(hasil.is_err(), "dekripsi dengan kunci berbeda harus gagal");

        let _ = std::fs::remove_dir_all(path1.parent().expect("parent harus ada"));
        let _ = std::fs::remove_dir_all(path2.parent().expect("parent harus ada"));
    }
}
