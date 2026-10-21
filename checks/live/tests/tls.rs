#[path = "../src/arena.rs"]
mod arena;
use embedded_tls::{
    Aes128GcmSha256, Certificate, CertificateRef, Sha256, TlsClock, TlsVerifier, pki::CertVerifier,
};
use std::sync::atomic::{AtomicU64, Ordering};
#[global_allocator]
static ALLOC: arena::Allocator = arena::Allocator;
static TIME: AtomicU64 = AtomicU64::new(1792368000);
struct Clock;
impl TlsClock for Clock {
    fn now() -> Option<u64> {
        Some(TIME.load(Ordering::Relaxed))
    }
}
const CHAIN: &[u8] = include_bytes!("fixtures/city-chain.bin");
fn verify(host: &str, bytes: &[u8]) -> (bool, usize, usize) {
    arena::measure(|| {
        let mut v = CertVerifier::<Aes128GcmSha256, Clock, 8192>::new(Certificate::X509(
            poznan_https::ROOT,
        ));
        v.set_hostname_verification(host).unwrap();
        v.verify_certificate(
            &Sha256::default(),
            CertificateRef::parse(&mut bytes.into()).unwrap(),
        )
        .is_ok()
    })
}
fn main() {
    for _ in 0..3 {
        let (ok, peak, misses) = verify(poznan_https::HOST, CHAIN);
        assert!(ok);
        assert_eq!(misses, 0);
        println!(
            "city RSA chain: {peak} / {} B first-fit arena; no retained allocations",
            poznan_https::RSA_HEAP_BYTES
        );
    }
    let (ok, _, misses) = verify("wrong.example", CHAIN);
    assert!(!ok);
    assert_eq!(misses, 0);
    TIME.store(1700000000, Ordering::Relaxed);
    assert!(!verify(poznan_https::HOST, CHAIN).0);
    TIME.store(1792368000, Ordering::Relaxed);
    let mut corrupt = CHAIN.to_vec();
    let n = corrupt.len();
    corrupt[n - 4] ^= 1;
    assert!(!verify(poznan_https::HOST, &corrupt).0);
}
