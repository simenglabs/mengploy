use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use axum::Form;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::auth::session::Session;
use crate::error::AppError;
use crate::notifications::model::NotificationSettings;
use crate::notifications::repo;
use crate::state::AppState;
use crate::web;

use super::servers::fleet_strip;

const PESAN_CSRF_TIDAK_VALID: &str =
    "Sesi tidak valid atau kedaluwarsa. Silakan muat ulang halaman dan coba lagi.";
const EVENTS: [&str; 4] = [
    "deployment.failed",
    "deployment.recovered",
    "reconciliation.drift_detected",
    "reconciliation.drift_resolved",
];

#[derive(Deserialize)]
pub struct NotificationForm {
    csrf_token: String,
    enabled: Option<String>,
    url: String,
    secret: String,
    events: String,
}

fn url_webhook_valid(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if scheme != "https" || rest.is_empty() || rest.contains('@') {
        return false;
    }
    let host_port = rest.split('/').next().unwrap_or_default();
    let host = host_port
        .rsplit_once(':')
        .map_or(host_port, |(host, port)| {
            if port.parse::<u16>().is_ok() {
                host
            } else {
                host_port
            }
        });
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty() || host == "localhost" || host.ends_with(".localhost") {
        return false;
    }
    let Ok(ip) = host.parse::<IpAddr>() else {
        return true;
    };
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_loopback()
                && !ip.is_private()
                && !ip.is_link_local()
                && ip != Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(ip) => !ip.is_loopback() && !ip.is_unspecified() && !is_ipv6_private(ip),
    }
}

fn is_ipv6_private(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00 || ip.segments()[0] == 0xfe80
}

async fn webhook_url_ssrf_safe(url: &str) -> bool {
    if !url_webhook_valid(url) {
        return false;
    }
    let Some((_, rest)) = url.split_once("://") else {
        return false;
    };
    let host_port = rest.split('/').next().unwrap_or_default();
    let (host, port) = host_port
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().ok().map(|port| (host, port)))
        .unwrap_or((host_port, 443));
    let Ok(mut addresses) = tokio::net::lookup_host((host, port)).await else {
        return false;
    };
    addresses.all(|address| match address.ip() {
        IpAddr::V4(ip) => {
            !ip.is_loopback()
                && !ip.is_private()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && ip != Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast() && !is_ipv6_private(ip)
        }
    })
}

fn mask_url(url: Option<&str>) -> Option<String> {
    url.map(|value| {
        if let Some((scheme, rest)) = value.split_once("://") {
            let host = rest.split('/').next().unwrap_or_default();
            format!("{scheme}://{host}/••••")
        } else {
            "••••".to_string()
        }
    })
}

pub async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Result<Response, AppError> {
    let settings = repo::load_settings(&state.db_read, &state.crypto).await?;
    let strip = fleet_strip(&state).await?;
    Ok(web::render_notification_settings(
        settings.enabled,
        mask_url(settings.url.as_deref()),
        &settings.events,
        None,
        &session.csrf_token,
        Some(strip),
    )
    .into_response())
}

pub async fn save(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Form(form): Form<NotificationForm>,
) -> Result<Response, AppError> {
    if form.csrf_token != session.csrf_token {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }
    if !form.url.is_empty() && !webhook_url_ssrf_safe(&form.url).await {
        return Err(AppError::BadRequest(
            "URL webhook harus HTTPS dan tidak boleh menunjuk ke alamat internal.".to_string(),
        ));
    }
    let events: Vec<String> = form
        .events
        .split(',')
        .map(str::trim)
        .filter(|event| !event.is_empty())
        .map(str::to_string)
        .collect();
    if events.iter().any(|event| !EVENTS.contains(&event.as_str())) {
        return Err(AppError::BadRequest(
            "Event webhook tidak dikenal.".to_string(),
        ));
    }
    let old = repo::load_settings(&state.db_read, &state.crypto).await?;
    let settings = NotificationSettings {
        enabled: form.enabled.is_some(),
        url: (!form.url.is_empty()).then_some(form.url),
        secret: (!form.secret.is_empty())
            .then_some(form.secret)
            .or(old.secret),
        events,
    };
    repo::save_settings(&state.db_write, &state.crypto, &settings).await?;
    let strip = fleet_strip(&state).await?;
    Ok(web::render_notification_settings(
        settings.enabled,
        mask_url(settings.url.as_deref()),
        &settings.events,
        Some("Pengaturan webhook disimpan tanpa menampilkan ulang secret."),
        &session.csrf_token,
        Some(strip),
    )
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::url_webhook_valid;

    #[test]
    fn url_webhook_menolak_target_internal() {
        assert!(!url_webhook_valid("http://example.invalid/hook"));
        assert!(!url_webhook_valid("https://127.0.0.1/hook"));
        assert!(!url_webhook_valid("https://10.0.0.5/hook"));
        assert!(!url_webhook_valid("https://169.254.169.254/latest"));
        assert!(!url_webhook_valid("https://[::1]/hook"));
        assert!(!url_webhook_valid("https://user:pass@example.invalid/hook"));
    }

    #[test]
    fn url_webhook_menerima_https_public() {
        assert!(url_webhook_valid("https://hooks.example.invalid/incoming"));
    }
}
