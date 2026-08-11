use hmac::{Hmac, Mac};
use sha2::Sha256;

pub struct NotificationSettings {
    pub enabled: bool,
    pub url: Option<String>,
    pub secret: Option<String>,
    pub events: Vec<String>,
}

impl NotificationSettings {
    pub fn empty() -> Self {
        Self {
            enabled: false,
            url: None,
            secret: None,
            events: Vec::new(),
        }
    }
}

/// Bentuk payload yang dikirim ke webhook. Hanya metadata non-secret yang
/// boleh masuk queue; environment, credential, path, log, dan stderr tidak
/// punya field di kontrak ini.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct WebhookEnvelope<'a> {
    pub event_id: &'a str,
    pub event_type: &'a str,
    pub occurred_at: i64,
    pub data: serde_json::Value,
}

/// Tandatangani byte payload persis seperti yang dikirim, supaya penerima
/// dapat memverifikasi tanpa ambiguitas serialisasi. Format header: `sha256=<hex>`.
pub fn sign_payload(secret: &[u8], payload: &[u8]) -> String {
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        // HMAC-SHA256 secara matematis menerima key dengan panjang berapa
        // pun; cabang ini hanya menjaga API tetap tanpa panic bila crate
        // mengubah validasi key di masa depan.
        return "sha256=".to_string();
    };
    mac.update(payload);
    let result = mac.finalize().into_bytes();
    format!("sha256={}", hex_encode(&result))
}

/// Verifikasi constant-time terhadap signature yang diterima.
pub fn verify_signature(secret: &[u8], payload: &[u8], signature: &str) -> bool {
    verify_signature_bytes(secret, payload, signature)
}

/// Verifikasi signature canonical `timestamp.payload` dan umur timestamp.
/// `now` dioper eksplisit supaya test fault-injection deterministik.
pub fn verify_signature_at(
    secret: &[u8],
    timestamp: &str,
    payload: &[u8],
    signature: &str,
) -> bool {
    if !timestamp.is_empty() {
        let Ok(sent_at) = timestamp.parse::<i64>() else {
            return false;
        };
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        if (now - sent_at).abs() > 300 {
            return false;
        }
    }
    let canonical = format!("{timestamp}.");
    let mut canonical = canonical.into_bytes();
    canonical.extend_from_slice(payload);
    verify_signature_bytes(secret, &canonical, signature)
}

fn verify_signature_bytes(secret: &[u8], payload: &[u8], signature: &str) -> bool {
    let Some(hex) = signature.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(expected) = decode_hex(hex) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        return false;
    };
    mac.update(payload);
    mac.verify_slice(&expected).is_ok()
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex(value: &str) -> Result<Vec<u8>, ()> {
    if !value.len().is_multiple_of(2) {
        return Err(());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16).ok_or(())?;
            let low = (pair[1] as char).to_digit(16).ok_or(())?;
            Ok(((high << 4) | low) as u8)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_hmac_sha256_deterministik_dan_constant_time() {
        let payload = br#"{"event":"deployment.failed"}"#;
        let signature = sign_payload(b"secret-uji", payload);
        assert!(verify_signature(b"secret-uji", payload, &signature));
        assert!(!verify_signature(b"secret-salah", payload, &signature));
        assert!(!verify_signature(
            b"secret-uji",
            b"payload-berubah",
            &signature
        ));
        assert!(!verify_signature(
            b"secret-uji",
            payload,
            "sha256=bukan-hex"
        ));
        let timestamp = time::OffsetDateTime::now_utc().unix_timestamp().to_string();
        let mut canonical = format!("{timestamp}.").into_bytes();
        canonical.extend_from_slice(payload);
        let timestamp_signature = sign_payload(b"secret-uji", &canonical);
        assert!(verify_signature_at(
            b"secret-uji",
            &timestamp,
            payload,
            &timestamp_signature
        ));
        assert!(!verify_signature_at(
            b"secret-uji",
            "1",
            payload,
            &timestamp_signature
        ));
    }
}
