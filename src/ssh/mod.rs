//! Konektivitas SSH: koneksi ControlMaster, eksekusi perintah remote, dan
//! host key TOFU (trust-on-first-use).
//!
//! Modul ini HANYA menangani transport SSH. Ia tidak tahu apa pun soal
//! database, enkripsi `age`, atau HTTP — pemanggil (`servers/verify.rs`,
//! sub-blok 3d) yang mendekripsi private key sebelum diserahkan ke sini,
//! dan yang memutuskan bagaimana kategori error dipetakan ke pesan Bahasa
//! Indonesia untuk pengguna (`docs/design/tambah-server.md`).
//!
//! Pembagian modul:
//! - `session` — bangun koneksi ControlMaster (`Session::connect`),
//!   timeout 10 detik wajib, mode TOFU vs ketat.
//! - `exec` — jalankan satu perintah remote lewat sesi yang sudah terbuka,
//!   memisahkan exit code dari error transport.
//! - `hostkey` — known_hosts milik aplikasi, ambil fingerprint, simpan
//!   entri setelah pengguna konfirmasi TOFU.

mod exec;
mod hostkey;
mod session;

pub use exec::{ExecResult, SshExecError, exec, exec_bounded, exec_with_stdin};
pub use hostkey::{HostKeyError, HostKeyProbe, confirm_and_store, fetch_fingerprint_via_keyscan};
pub use session::{ConnectOutcome, HostKeyMode, SshConnectError, SshSession, connect};
