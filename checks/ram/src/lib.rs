//! Cross-target ABI storage of the actual HTTPS/cache future with an Embassy
//! TCP socket. Stack and RSA heap are separate explicit reservations.
#![no_std]
#![forbid(unsafe_code)]
use core::mem::{size_of, size_of_val};
use embedded_storage::nor_flash::{ErrorType, NorFlash, NorFlashErrorKind, ReadNorFlash};
use embedded_tls::TlsClock;
use poznan::{Query, cache::Cache};
use poznan_https::{Workspace, refresh};

pub struct Clock;
impl TlsClock for Clock {
    fn now() -> Option<u64> {
        Some(1792368000)
    }
}
// These types exist only for size_of_val; the workflow is never executed.
pub struct Rng;
impl rand_core::RngCore for Rng {
    fn next_u32(&mut self) -> u32 {
        unreachable!()
    }
    fn next_u64(&mut self) -> u64 {
        unreachable!()
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        unreachable!()
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        unreachable!()
    }
}
impl rand_core::CryptoRng for Rng {}
pub struct Flash;
impl ErrorType for Flash {
    type Error = NorFlashErrorKind;
}
impl ReadNorFlash for Flash {
    const READ_SIZE: usize = 4;
    fn read(&mut self, _: u32, _: &mut [u8]) -> Result<(), Self::Error> {
        unreachable!()
    }
    fn capacity(&self) -> usize {
        65536
    }
}
impl NorFlash for Flash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;
    fn write(&mut self, _: u32, _: &[u8]) -> Result<(), Self::Error> {
        unreachable!()
    }
    fn erase(&mut self, _: u32, _: u32) -> Result<(), Self::Error> {
        unreachable!()
    }
}
#[inline(never)]
pub fn workspace_bytes() -> usize {
    size_of::<Workspace<Clock>>()
}
#[inline(never)]
pub fn task_bytes(
    socket: embassy_net::tcp::TcpSocket<'_>,
    flash: &mut Flash,
    rng: &mut Rng,
    work: &mut Workspace<Clock>,
) -> usize {
    size_of_val(&refresh(
        socket,
        flash,
        Cache::new([0, 32768], 32768),
        Query::Schedule {
            street: "TESTOWA",
            number: "1",
            object: None,
        },
        rng,
        work,
    ))
}
#[inline(never)]
pub fn reserves_bytes() -> usize {
    poznan_https::RSA_HEAP_BYTES
        + poznan_https::STACK_RESERVE
        + poznan_https::TCP_RX
        + poznan_https::TCP_TX
        + 1024 // RNG, flash adapter, executor metadata and socket bookkeeping allowance
}
