use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    h_flex, v_flex,
};
use gpui_kit::{
    App, ParentElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use sha2::{Digest as _, Sha256};

use crate::host_keys::detail;

const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);
const REJECTION_TTL: Duration = Duration::from_secs(8);

pub struct CertificateRequest {
    host: String,
    port: u16,
    fingerprint: String,
    reason: String,
    stored: Vec<String>,
    reply: async_channel::Sender<bool>,
}

impl CertificateRequest {
    fn answer(&self, trusted: bool) {
        let _ = self.reply.try_send(trusted);
    }
}

type Prompts = (
    async_channel::Sender<CertificateRequest>,
    async_channel::Receiver<CertificateRequest>,
);

static PROMPTS: OnceLock<Prompts> = OnceLock::new();
static PROMPT_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static REJECTIONS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
static STORE_LOCK: Mutex<()> = Mutex::new(());
static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();

fn prompts() -> &'static Prompts {
    PROMPTS.get_or_init(async_channel::unbounded)
}

pub fn requests() -> async_channel::Receiver<CertificateRequest> {
    prompts().1.clone()
}

type Store = BTreeMap<String, Vec<String>>;

fn store_path() -> PathBuf {
    crate::settings::config_dir().join("trusted_certificates.json")
}

fn store_key(host: &str, port: u16) -> String {
    format!("{}:{port}", host.to_lowercase())
}

fn read_store(path: &std::path::Path) -> Store {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

fn stored_for(path: &std::path::Path, host: &str, port: u16) -> Vec<String> {
    read_store(path)
        .remove(&store_key(host, port))
        .unwrap_or_default()
}

fn remember(
    path: &std::path::Path,
    host: &str,
    port: u16,
    fingerprint: &str,
) -> Result<(), String> {
    let _guard = STORE_LOCK.lock().map_err(|e| e.to_string())?;
    let mut store = read_store(path);
    let entry = store.entry(store_key(host, port)).or_default();
    if !entry.iter().any(|stored| stored == fingerprint) {
        entry.push(fingerprint.to_string());
    }
    let contents = serde_json::to_string_pretty(&store).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    crate::settings::write_private(path, &contents)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))
}

pub fn fingerprint(certificate: &[u8]) -> String {
    Sha256::digest(certificate)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn was_rejected(id: &str) -> bool {
    REJECTIONS
        .get_or_init(Default::default)
        .lock()
        .is_ok_and(|rejected| {
            rejected
                .get(id)
                .is_some_and(|at| at.elapsed() < REJECTION_TTL)
        })
}

fn remember_rejection(id: String) {
    if let Ok(mut rejected) = REJECTIONS.get_or_init(Default::default).lock() {
        rejected.retain(|_, at| at.elapsed() < REJECTION_TTL);
        rejected.insert(id, Instant::now());
    }
}

fn refused_message(host: &str, port: u16) -> String {
    format!("The TLS certificate of {host}:{port} was not trusted, so the connection was refused")
}

pub async fn confirm(host: &str, port: u16, failure: &CertificateFailure) -> Result<(), String> {
    let path = store_path();
    let _guard = PROMPT_LOCK.get_or_init(Default::default).lock().await;
    let stored = stored_for(&path, host, port);
    if stored.contains(&failure.fingerprint) {
        return Ok(());
    }

    let id = format!("{}/{}", store_key(host, port), failure.fingerprint);
    if was_rejected(&id) {
        return Err(refused_message(host, port));
    }

    let (reply, answer) = async_channel::bounded(1);
    let request = CertificateRequest {
        host: host.to_string(),
        port,
        fingerprint: failure.fingerprint.clone(),
        reason: failure.reason.clone(),
        stored,
        reply,
    };
    if prompts().0.send(request).await.is_err() {
        return Err("Could not ask about the untrusted certificate".to_string());
    }

    let trusted = matches!(
        tokio::time::timeout(PROMPT_TIMEOUT, answer.recv()).await,
        Ok(Ok(true))
    );
    if !trusted {
        remember_rejection(id);
        return Err(refused_message(host, port));
    }
    remember(&path, host, port, &failure.fingerprint)
}

#[derive(Clone, Debug)]
pub struct CertificateFailure {
    pub fingerprint: String,
    pub reason: String,
}

fn describe(error: &rustls::Error) -> String {
    match error {
        rustls::Error::InvalidCertificate(error) => match error {
            CertificateError::UnknownIssuer => {
                "It is self-signed or issued by an authority this computer doesn't trust."
                    .to_string()
            }
            CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
                "It has expired.".to_string()
            }
            CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                "It isn't valid yet.".to_string()
            }
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
                "It was issued for a different host name.".to_string()
            }
            CertificateError::Revoked => "It has been revoked.".to_string(),
            other if format!("{other:?}").contains("CaUsedAsEndEntity") => {
                "It is self-signed.".to_string()
            }
            other => format!("It failed verification ({other:?})."),
        },
        other => format!("It failed verification ({other})."),
    }
}

fn roots() -> Arc<RootCertStore> {
    ROOTS
        .get_or_init(|| {
            let mut roots = RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            let native = rustls_native_certs::load_native_certs();
            roots.add_parsable_certificates(native.certs);
            Arc::new(roots)
        })
        .clone()
}

#[derive(Debug)]
pub struct PinningVerifier {
    inner: Arc<WebPkiServerVerifier>,
    host: String,
    port: u16,
    store: PathBuf,
    pins: std::sync::OnceLock<Vec<String>>,
    failure: Mutex<Option<CertificateFailure>>,
}

impl PinningVerifier {
    pub fn take_failure(&self) -> Option<CertificateFailure> {
        self.failure
            .lock()
            .ok()
            .and_then(|mut failure| failure.take())
    }
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let error = match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) => return Ok(verified),
            Err(error) => error,
        };
        let fingerprint = fingerprint(end_entity);
        let pins = self
            .pins
            .get_or_init(|| stored_for(&self.store, &self.host, self.port));
        if pins.contains(&fingerprint) {
            return Ok(ServerCertVerified::assertion());
        }
        if let Ok(mut failure) = self.failure.lock() {
            *failure = Some(CertificateFailure {
                fingerprint,
                reason: describe(&error),
            });
        }
        Err(error)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

pub fn client_config(
    host: &str,
    port: u16,
) -> anyhow::Result<(Arc<ClientConfig>, Arc<PinningVerifier>)> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let inner = WebPkiServerVerifier::builder_with_provider(roots(), provider.clone()).build()?;
    let verifier = Arc::new(PinningVerifier {
        inner,
        host: host.to_string(),
        port,
        store: store_path(),
        pins: std::sync::OnceLock::new(),
        failure: Mutex::new(None),
    });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    Ok((Arc::new(config), verifier))
}

pub fn open_prompt(request: CertificateRequest, window: &mut Window, cx: &mut App) {
    let request = Rc::new(request);
    let changed = !request.stored.is_empty();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let request = request.clone();
        let muted = cx.theme().muted_foreground;
        let tone = if changed {
            cx.theme().danger
        } else {
            cx.theme().warning
        };
        let summary = if changed {
            format!(
                "{}:{} presented a certificate that differs from the one you trusted before. \
                 {} If something is intercepting this connection, this is what it would look \
                 like.",
                request.host, request.port, request.reason
            )
        } else {
            format!(
                "The TLS certificate of {}:{} can't be verified. {}",
                request.host, request.port, request.reason
            )
        };

        dialog
            .w(px(540.))
            .title(if changed {
                "Certificate has changed"
            } else {
                "Untrusted certificate"
            })
            .child(
                v_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_start()
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .small()
                                    .text_color(tone)
                                    .flex_none(),
                            )
                            .child(div().flex_1().min_w_0().text_sm().child(summary)),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .p_3()
                            .rounded_md()
                            .bg(cx.theme().muted)
                            .child(detail("SHA-256", &request.fingerprint, muted)),
                    )
                    .when(changed, |this| {
                        this.child(
                            v_flex()
                                .gap_1()
                                .p_3()
                                .rounded_md()
                                .bg(cx.theme().muted)
                                .children(request.stored.iter().enumerate().map(|(ix, stored)| {
                                    detail(if ix == 0 { "Trusted" } else { "" }, stored, muted)
                                })),
                        )
                    })
                    .child(div().text_xs().text_color(muted).child(
                        "Continue only if this fingerprint matches the server's certificate. \
                         Trusting it remembers this exact certificate for this host and port, \
                         and any other one will be questioned again.",
                    )),
            )
            .footer(
                DialogFooter::new()
                    .child(Button::new("reject").primary().label("Reject").on_click({
                        let request = request.clone();
                        move |_, window, cx| {
                            request.answer(false);
                            window.close_dialog(cx);
                        }
                    }))
                    .child(
                        Button::new("trust")
                            .map(|button| {
                                if changed {
                                    button.danger()
                                } else {
                                    button.outline()
                                }
                            })
                            .label("Trust this certificate")
                            .on_click({
                                let request = request.clone();
                                move |_, window, cx| {
                                    request.answer(true);
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
    });
}

#[cfg(test)]
#[path = "../tests/cert_trust.rs"]
mod tests;
