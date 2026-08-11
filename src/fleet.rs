//! Domain Fase 7: operasi armada dan pintu darurat.
//! Tidak ada akses jaringan atau HTML di modul ini; fungsi murni di sini
//! menjadi pagar validasi sebelum operasi remote dijalankan.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

pub const OUTPUT_MAX_BYTES: usize = 256 * 1024;
pub const COMMAND_MAX_BYTES: usize = 4096;
pub const OPERATION_TIMEOUT_SECS: u64 = 120;
pub const EXEC_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetOperationKind {
    Command,
    Prune,
    Exec,
}

impl FleetOperationKind {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Prune => "prune",
            Self::Exec => "exec",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetResultStatus {
    Succeeded,
    Failed,
    Skipped,
}

impl FleetResultStatus {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiskSummary {
    pub server_id: String,
    pub server_name: String,
    pub status: String,
    pub used_bytes: Option<i64>,
    pub total_bytes: Option<i64>,
    pub sampled_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct FleetOperationSummary {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub targets: Vec<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct FleetOperationResultSummary {
    pub operation_id: String,
    pub server_id: String,
    pub exit_code: Option<i64>,
    pub output_path: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetJobPayload {
    pub operation_id: String,
}

#[derive(Debug, Clone)]
pub struct FleetEvent {
    pub operation_id: String,
    pub status: String,
    pub server_id: Option<String>,
    pub message: Option<String>,
}

pub fn validate_command(command: &str) -> Result<String> {
    let command = command.trim();
    if command.is_empty() {
        return Err(anyhow!("perintah wajib diisi"));
    }
    if command.len() > COMMAND_MAX_BYTES {
        return Err(anyhow!("perintah terlalu panjang"));
    }
    if command.bytes().any(|byte| byte == 0) {
        return Err(anyhow!("perintah mengandung karakter yang tidak valid"));
    }
    Ok(command.to_string())
}

pub fn validate_targets(targets: &[String]) -> Result<Vec<String>> {
    if targets.is_empty() {
        return Err(anyhow!("pilih minimal satu server"));
    }
    let mut unique = HashSet::new();
    let mut result = Vec::with_capacity(targets.len());
    for target in targets {
        let target = target.trim();
        if target.is_empty() || !unique.insert(target.to_string()) {
            return Err(anyhow!("daftar server tidak valid"));
        }
        result.push(target.to_string());
    }
    Ok(result)
}

pub fn validate_exec_command(command: &str) -> Result<String> {
    validate_command(command)
}

pub fn bounded_output(bytes: &[u8]) -> (String, bool) {
    if bytes.len() <= OUTPUT_MAX_BYTES {
        return (String::from_utf8_lossy(bytes).into_owned(), false);
    }
    let output = String::from_utf8_lossy(&bytes[..OUTPUT_MAX_BYTES]).into_owned();
    (output, true)
}

pub fn output_path_is_safe(path: &str, base: &Path) -> bool {
    let candidate = Path::new(path);
    candidate.is_absolute()
        && candidate.starts_with(base)
        && !candidate
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
}

pub fn parse_disk_output(output: &str) -> Result<(i64, i64)> {
    let line = output
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .context("keluaran disk kosong")?;
    let mut parts = line.split_whitespace();
    let used = parts
        .next()
        .context("pemakaian disk tidak tersedia")?
        .parse::<i64>()
        .context("pemakaian disk bukan angka")?;
    let total = parts
        .next()
        .context("total disk tidak tersedia")?
        .parse::<i64>()
        .context("total disk bukan angka")?;
    if used < 0 || total <= 0 || used > total {
        return Err(anyhow!("rentang disk tidak valid"));
    }
    Ok((used, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_kosong_ditolak_dan_command_valid_ditrim() {
        assert!(validate_command("  ").is_err());
        assert_eq!(validate_command("  uptime  ").expect("valid"), "uptime");
    }

    #[test]
    fn command_nul_ditolak() {
        assert!(validate_command("echo\0rahasia").is_err());
    }

    #[test]
    fn output_dipotong_dan_diberi_penanda() {
        let input = vec![b'x'; OUTPUT_MAX_BYTES + 1];
        let (output, truncated) = bounded_output(&input);
        assert!(truncated);
        assert_eq!(output.len(), OUTPUT_MAX_BYTES);
    }

    #[test]
    fn output_kecil_tidak_dipotong() {
        assert_eq!(bounded_output(b"ok"), ("ok".to_string(), false));
    }

    #[test]
    fn disk_output_harus_valid_dan_tidak_boleh_menebak() {
        assert_eq!(parse_disk_output("100 1000\n").expect("valid"), (100, 1000));
        assert!(parse_disk_output("disk rusak").is_err());
        assert!(parse_disk_output("100 10").is_err());
    }

    #[test]
    fn path_output_hanya_boleh_di_direktori_dasar() {
        let base = Path::new("/var/lib/mengdep/operations");
        assert!(output_path_is_safe(
            "/var/lib/mengdep/operations/a.out",
            base
        ));
        assert!(!output_path_is_safe("/tmp/a.out", base));
        assert!(!output_path_is_safe(
            "/var/lib/mengdep/operations/../a.out",
            base
        ));
    }
}
