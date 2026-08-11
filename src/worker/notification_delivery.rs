use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper::body::Bytes;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::{HttpConnector, dns::Name};
use hyper_util::rt::TokioExecutor;
use tokio::sync::watch;
use tower_service::Service;

use crate::notifications::model;
use crate::notifications::repo::Delivery;
use crate::state::AppState;

use super::WorkerHandle;

const TICK_INTERVAL: Duration = Duration::from_secs(5);
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_PAYLOAD_BYTES: usize = 256 * 1024;
const MAX_ATTEMPTS: i64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryFailure {
    Config,
    DnsInternal,
    Timeout,
    Transport,
    HttpRetryable,
    HttpRejected,
    Payload,
}

impl DeliveryFailure {
    fn kind(self) -> &'static str {
        match self {
            Self::Config => "konfigurasi_tidak_valid",
            Self::DnsInternal => "target_internal",
            Self::Timeout => "timeout_delivery",
            Self::Transport => "transport_delivery",
            Self::HttpRetryable => "webhook_sementara_menolak",
            Self::HttpRejected => "webhook_menolak",
            Self::Payload => "payload_tidak_valid",
        }
    }

    fn retryable(self) -> bool {
        matches!(self, Self::Timeout | Self::Transport | Self::HttpRetryable)
    }
}

/// Resolver yang hanya mengembalikan alamat yang sudah melewati pemeriksaan
/// SSRF. Dengan ini request tidak melakukan lookup DNS kedua yang dapat
/// terkena DNS rebinding antara validasi dan koneksi.
#[derive(Clone, Debug)]
struct FixedResolver {
    addresses: Vec<SocketAddr>,
}

impl Service<Name> for FixedResolver {
    type Response = std::vec::IntoIter<SocketAddr>;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _name: Name) -> Self::Future {
        std::future::ready(Ok(self.addresses.clone().into_iter()))
    }
}

async fn jalankan_satu_siklus(state: &AppState) {
    let delivery = match crate::notifications::repo::claim_next(&state.db_write).await {
        Ok(Some(delivery)) => delivery,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(error = %err, "gagal klaim delivery webhook");
            return;
        }
    };

    let outcome = kirim_delivery(state, &delivery).await;
    match outcome {
        Ok(status_code) => {
            if let Err(err) = crate::notifications::repo::mark_delivered(
                &state.db_write,
                &delivery.id,
                i64::from(status_code),
            )
            .await
            {
                tracing::warn!(error = %err, delivery_id = %delivery.id, "gagal menandai webhook terkirim");
            }
        }
        Err(failure) if failure.retryable() && delivery.attempts < MAX_ATTEMPTS => {
            if let Err(err) = crate::notifications::repo::mark_retry(
                &state.db_write,
                &delivery.id,
                failure.kind(),
                delivery.attempts,
            )
            .await
            {
                tracing::warn!(error = %err, delivery_id = %delivery.id, "gagal menjadwalkan ulang delivery webhook");
            }
        }
        Err(failure) => {
            if let Err(err) = crate::notifications::repo::mark_failed(
                &state.db_write,
                &delivery.id,
                failure.kind(),
                failure.retryable(),
            )
            .await
            {
                tracing::warn!(error = %err, delivery_id = %delivery.id, "gagal menandai delivery webhook gagal");
            }
        }
    }
}

async fn kirim_delivery(state: &AppState, delivery: &Delivery) -> Result<u16, DeliveryFailure> {
    let settings = crate::notifications::repo::load_settings(&state.db_read, &state.crypto)
        .await
        .map_err(|_| DeliveryFailure::Config)?;
    if !settings.enabled
        || !settings
            .events
            .iter()
            .any(|event| event == &delivery.event_type)
    {
        return Err(DeliveryFailure::Config);
    }
    let Some(url) = settings.url.as_deref() else {
        return Err(DeliveryFailure::Config);
    };
    let Some(secret) = settings.secret.as_deref() else {
        return Err(DeliveryFailure::Config);
    };
    let uri: hyper::Uri = url.parse().map_err(|_| DeliveryFailure::Config)?;
    if uri.scheme_str() != Some("https") || uri.host().is_none() {
        return Err(DeliveryFailure::Config);
    }
    let addresses = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::lookup_host((
            uri.host().unwrap_or_default(),
            uri.port_u16().unwrap_or(443),
        )),
    )
    .await
    .map_err(|_| DeliveryFailure::Timeout)?
    .map_err(|_| DeliveryFailure::Transport)?
    .collect::<Vec<SocketAddr>>();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| alamat_internal(address.ip()))
    {
        return Err(DeliveryFailure::DnsInternal);
    }

    let payload = delivery.payload_json.as_bytes();
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(DeliveryFailure::Payload);
    }
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp().to_string();
    let signed_payload = format!("{timestamp}.").into_bytes();
    let mut signed_payload = signed_payload;
    signed_payload.extend_from_slice(payload);
    let signature = model::sign_payload(secret.as_bytes(), &signed_payload);
    let mut http = HttpConnector::new_with_resolver(FixedResolver { addresses });
    http.set_connect_timeout(Some(Duration::from_secs(5)));
    let https = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_only()
        .enable_http1()
        .wrap_connector(http);
    let client: Client<_, Full<Bytes>> = Client::builder(TokioExecutor::new()).build(https);
    let request = Request::post(uri)
        .header("content-type", "application/json")
        .header("x-mengdep-event", &delivery.event_type)
        .header("x-mengdep-signature", signature)
        .header("x-mengdep-timestamp", timestamp)
        .body(Full::new(Bytes::copy_from_slice(payload)))
        .map_err(|_| DeliveryFailure::Config)?;
    let response = tokio::time::timeout(DELIVERY_TIMEOUT, client.request(request))
        .await
        .map_err(|_| DeliveryFailure::Timeout)?
        .map_err(|_| DeliveryFailure::Transport)?;
    let status = response.status().as_u16();
    let _ = tokio::time::timeout(Duration::from_secs(3), response.into_body().collect()).await;
    if (200..300).contains(&status) {
        Ok(status)
    } else if status == 408 || status == 429 || status >= 500 {
        Err(DeliveryFailure::HttpRetryable)
    } else {
        Err(DeliveryFailure::HttpRejected)
    }
}

fn alamat_internal(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip == Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (ip.segments()[0] & 0xfe00 == 0xfc00)
                || ip.segments()[0] == 0xfe80
        }
    }
}

pub fn spawn(state: AppState) -> WorkerHandle {
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let join_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => jalankan_satu_siklus(&state).await,
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() { break; }
                }
            }
        }
    });
    WorkerHandle {
        shutdown_tx,
        join_handle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::model::WebhookEnvelope;
    use crate::notifications::repo::retry_delay_secs;

    #[test]
    fn retry_delay_bertambah_dan_dibatasi() {
        assert_eq!(retry_delay_secs(1), 2);
        assert_eq!(retry_delay_secs(3), 8);
        assert_eq!(retry_delay_secs(99), 256);
    }

    #[test]
    fn alamat_internal_menolak_loopback_private_link_local_dan_metadata() {
        for value in [
            "127.0.0.1".parse().unwrap(),
            "10.0.0.1".parse().unwrap(),
            "169.254.169.254".parse().unwrap(),
            "::1".parse().unwrap(),
            "fc00::1".parse().unwrap(),
        ] {
            assert!(alamat_internal(value));
        }
        assert!(!alamat_internal("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn resolver_tidak_mengubah_daftar_alamat_yang_sudah_divalidasi() {
        let resolver = FixedResolver {
            addresses: vec![SocketAddr::from(([8, 8, 8, 8], 443))],
        };
        assert_eq!(resolver.addresses.len(), 1);
        assert_eq!(
            resolver.addresses[0].ip(),
            "8.8.8.8".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn timestamp_mengikat_signature_dan_timestamp_lama_ditolak() {
        let payload =
            br#"{"event_id":"e","event_type":"deployment.failed","occurred_at":1,"data":{}}"#;
        let timestamp = time::OffsetDateTime::now_utc().unix_timestamp().to_string();
        let mut canonical = format!("{timestamp}.").into_bytes();
        canonical.extend_from_slice(payload);
        let signature = model::sign_payload(b"secret-uji", &canonical);
        assert!(model::verify_signature_at(
            b"secret-uji",
            &timestamp,
            payload,
            &signature
        ));
        assert!(!model::verify_signature_at(
            b"secret-uji",
            "1",
            payload,
            &signature
        ));
    }

    #[test]
    fn tipe_envelope_hanya_metadata() {
        let envelope = WebhookEnvelope {
            event_id: "event-1",
            event_type: "deployment.failed",
            occurred_at: 1,
            data: serde_json::json!({"status": "failed"}),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("environment"));
    }
}
