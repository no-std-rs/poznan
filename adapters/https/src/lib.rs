//! Verified TLS 1.3 + framed HTTP + streaming parse + two-bank NOR cache.
//! This adapter needs an allocator for RSA verification; the `poznan` core does
//! not. Give RSA its own bounded arena (see the host probe) and a valid clock.
//! Socket/DNS setup, deadlines, radio and network stack are caller-owned.
#![no_std]
#![forbid(unsafe_code)]
use embedded_io_async::{BufRead, ErrorType, Read, Write};
use embedded_storage::nor_flash::NorFlash;
use embedded_tls::{
    Aes128GcmSha256, Certificate, CryptoProvider, CryptoRngCore, TlsClock, TlsConfig,
    TlsConnection, TlsContext, TlsError, TlsVerifier, pki::CertVerifier,
};
use poznan::{
    Query,
    cache::{Buffers, Cache, Snapshot},
};
use reqwless::{
    TryBufRead,
    request::{Method, Request, RequestBuilder},
    response::Response,
};

pub const HOST: &str = "www.poznan.pl";
pub const ROOT: &[u8] = include_bytes!("../certs/digicert-global-root-g2.der");
/// Fixed first-fit arena tested against the observed city certificate chain.
pub const RSA_HEAP_BYTES: usize = 24 * 1024;
/// Explicit allowance, NOT a measured device stack high-water mark.
pub const STACK_RESERVE: usize = 24 * 1024;
pub const TCP_RX: usize = 4096;
pub const TCP_TX: usize = 2048;

#[derive(Debug)]
pub enum Error {
    Clock,
    Query(poznan::Error),
    Tls(TlsError),
    Http(reqwless::Error),
    Status(u16),
    Encoding,
    Framing,
    Cache(poznan::cache::Error),
}

/// Allocate this once in static storage on the device, rather than constructing
/// it on a small task stack. All arrays remain in place across refresh awaits.
pub struct Workspace<C: TlsClock> {
    rx: [u8; 16640],
    tx: [u8; 2304],
    headers: [u8; 2048],
    path: [u8; 1024],
    tag: [u8; 512],
    text: [u8; 256],
    metadata: [u8; 512],
    write: [u8; 256],
    verifier: Option<CertVerifier<'static, Aes128GcmSha256, C, 8192>>,
}
impl<C: TlsClock> Default for Workspace<C> {
    fn default() -> Self {
        Self::new()
    }
}
impl<C: TlsClock> Workspace<C> {
    pub const fn new() -> Self {
        Self {
            rx: [0; 16640],
            tx: [0; 2304],
            headers: [0; 2048],
            path: [0; 1024],
            tag: [0; 512],
            text: [0; 256],
            metadata: [0; 512],
            write: [0; 256],
            verifier: None,
        }
    }
}
struct Provider<'a, R, C: TlsClock> {
    rng: &'a mut R,
    verifier: &'a mut CertVerifier<'static, Aes128GcmSha256, C, 8192>,
}
impl<R: CryptoRngCore, C: TlsClock> CryptoProvider for Provider<'_, R, C> {
    type CipherSuite = Aes128GcmSha256;
    type Signature = [u8; 64];
    fn rng(&mut self) -> impl CryptoRngCore {
        &mut *self.rng
    }
    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Self::CipherSuite>, TlsError> {
        Ok(&mut *self.verifier)
    }
}
struct Io<S>(S);
impl<S: ErrorType> ErrorType for Io<S> {
    type Error = S::Error;
}
impl<S: Read> Read for Io<S> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(out).await
    }
}
impl<S: Write> Write for Io<S> {
    async fn write(&mut self, data: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(data).await
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}
impl<S: Read> TryBufRead for Io<S> {}

/// Refresh a single exact schedule query over an already connected TCP socket.
/// The clock must stay valid during verification. Use a hardware CSPRNG and
/// cancel this future on a deadline. Cancellation never commits partial data.
/// No redirects, compression or EOF-delimited bodies are accepted.
pub async fn refresh<S: Read + Write, F: NorFlash, R: CryptoRngCore, C: TlsClock>(
    socket: S,
    flash: &mut F,
    cache: Cache,
    query: Query<'_>,
    rng: &mut R,
    work: &mut Workspace<C>,
) -> Result<Snapshot, Error> {
    let now = C::now().ok_or(Error::Clock)?;
    if !matches!(query, Query::Schedule { .. }) {
        return Err(Error::Query(poznan::Error::InvalidQuery));
    }
    let path = query.path(&mut work.path).map_err(Error::Query)?;
    let mut tls = TlsConnection::new(socket, &mut work.rx, &mut work.tx);
    let config = TlsConfig::new()
        .with_server_name(HOST)
        .enable_rsa_signatures();
    let verifier = work
        .verifier
        .insert(CertVerifier::new(Certificate::X509(ROOT)));
    tls.open(TlsContext::new(&config, &mut Provider { rng, verifier }))
        .await
        .map_err(Error::Tls)?;
    let mut stream = Io(&mut tls);
    Request::get(path)
        .host(HOST)
        .headers(&[("Accept-Encoding", "identity"), ("Connection", "close")])
        .build()
        .write_header(&mut stream)
        .await
        .map_err(Error::Http)?;
    stream.flush().await.map_err(Error::Tls)?;
    let response = Response::read(&mut stream, Method::GET, &mut work.headers)
        .await
        .map_err(Error::Http)?;
    store_response(
        response,
        flash,
        cache,
        Buffers {
            tag: &mut work.tag,
            text: &mut work.text,
            metadata: &mut work.metadata,
            write: &mut work.write,
        },
        now,
    )
    .await
}

async fn store_response<S: Read + TryBufRead, F: NorFlash>(
    response: Response<'_, '_, S>,
    flash: &mut F,
    cache: Cache,
    buffers: Buffers<'_>,
    now: u64,
) -> Result<Snapshot, Error> {
    if response.status.0 != 200 {
        return Err(Error::Status(response.status.0));
    }
    for (name, value) in response.headers() {
        if name.eq_ignore_ascii_case("content-encoding") && value != b"identity" {
            return Err(Error::Encoding);
        }
    }
    // A fixed length or validated chunk terminator must establish completeness.
    // reqwless's Read path can return early EOF for fixed-length bodies; BufRead
    // rejects it. Reject ambiguous/unsupported transfer encodings explicitly.
    let mut chunked = false;
    let mut length = None;
    for (name, value) in response.headers() {
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if chunked || value != b"chunked" {
                return Err(Error::Framing);
            }
            chunked = true;
        }
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() || value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
                return Err(Error::Framing);
            }
            length = Some(
                core::str::from_utf8(value)
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
                    .ok_or(Error::Framing)?,
            );
        }
    }
    if chunked == length.is_some() {
        return Err(Error::Framing);
    }
    let mut body = response.body().reader();
    let mut update = cache.update(flash, buffers, now).map_err(Error::Cache)?;
    let mut total = 0usize;
    loop {
        let bytes = body.fill_buf().await.map_err(Error::Http)?;
        if bytes.is_empty() {
            break;
        }
        total = total.checked_add(bytes.len()).ok_or(Error::Framing)?;
        update.feed(bytes).map_err(Error::Cache)?;
        let n = bytes.len();
        body.consume(n);
    }
    if length.is_some_and(|n| n != total) {
        return Err(Error::Framing);
    }
    update
        .finish()
        .map_err(Error::Cache)?
        .commit()
        .map_err(Error::Cache)
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;
