# ESP32-C3: a 100,000-byte pipeline

The budget is **100,000 bytes available to the download/parse/cache pipeline**,
not the entire Wi-Fi firmware. The supplied adapter uses real `embedded-tls`
0.19 and `reqwless` 0.14 code, not curl or a desktop TLS implementation.

`python3 checks/check_ram.py` compiles the actual refresh future with an
`embassy_net::tcp::TcpSocket` for `riscv32imc-unknown-none-elf`, then reads
compiler-folded sizes from LLVM IR. It fails above 100,000 bytes.

| Concurrent storage | Bytes | Evidence |
| --- | ---: | --- |
| Workspace: TLS records, certificate verifier, HTTP headers, URL, parser and flash buffers | 32,216 | ESP32-C3 ABI size |
| Refresh future, including socket and nested handshake/HTTP futures | 2,272 | ESP32-C3 ABI size |
| RSA first-fit arena | 24,576 | Fixed reservation; host-tested |
| Call stack | 24,576 | Allowance; requires board high-water measurement |
| TCP receive / send buffers | 6,144 | 4,096 + 2,048 configuration |
| RNG, flash adapter, executor and socket bookkeeping | 1,024 | Allowance |
| **Total / remaining** | **90,808 / 9,192** | Compiler gate checks current code |

These are concurrent reservations, not a physical board RAM measurement.
Wi-Fi driver buffers, shared network-stack tables, DNS task storage and the
application's normal baseline must exist **outside** the available pipeline
budget. A larger socket, verifier, TLS record, stack, or executor configuration
needs a new budget. Only one refresh may run at once. No ESP32 board was attached
for this validation; stack high-water and on-device allocator behavior remain
to be checked before calling this a measured hardware limit.

Keep `Workspace` in static storage. Its constructor is `const`, so firmware can
use `static_cell::ConstStaticCell::new(Workspace::<Clock>::new())` and `.take()`;
this avoids constructing a 32 KB temporary on a small task stack. The verifier
is initialized inside that workspace for each handshake. Firmware supplies a
valid UTC clock, hardware CSPRNG, connected socket, cancellation deadline and
`critical-section` implementation. Portable atomics are enabled for C3's
instruction set. RSA needs an allocator; use a dedicated 24 KiB arena and reject
unavailable capacity before starting the handshake. The parser/cache library
itself needs neither `alloc` nor `std`.

## Live evidence and reproduction

On 2026-10-19, a verified TLS 1.3 refresh of
`JANA HENRYKA DĄBROWSKIEGO`, building group `12`, returned **594 pickups across
two distinct service objects**, `1/2275` and `1/2275.`. About 149 KB of HTML became
**3,089 payload bytes** in flash, plus the 64-byte header and write padding.
A separate process reopened the file and replayed every pickup; all 594 tuples
matched the independently checked parser output.

The host probe runs the same embedded HTTPS/parser/cache code with blocking TCP
and a file-backed NOR adapter. During the whole refresh, allocations are routed
to a **24,576-byte first-fit arena**. The observed peak was **22,272 bytes**, with
zero arena misses and no retained allocations. A miss falls back to the system
allocator only to let this diagnostic print a failure; the probe then fails.
This is host allocation evidence, not an ESP32 heap or stack measurement.

```sh
# Network access needed only for refresh. File is a 64 KiB NOR image.
cargo run --release --locked --manifest-path checks/live/Cargo.toml -- \
  refresh /tmp/poznan-cache.bin 'JANA HENRYKA DĄBROWSKIEGO' 12

# Restart and replay without any network access.
cargo run --release --locked --manifest-path checks/live/Cargo.toml -- \
  read /tmp/poznan-cache.bin

# Test the same flash path with a local response.
cargo run --release --locked --manifest-path checks/live/Cargo.toml -- \
  import /tmp/poznan-cache.bin tests/fixtures/schedule.html
```

The adapter verifies the hostname, chain, validity time and handshake signature
against DigiCert Global Root G2, stored in flash. It sends `Accept-Encoding:
identity`, accepts status 200, and requires an unambiguous Content-Length or
complete chunked framing. It rejects redirects, compressed data, premature EOF
and malformed framing. Merely reaching the closing HTML tag does not commit.
The supplied CA is the current city's root, not a pinned leaf certificate;
server trust-chain changes require updating this adapter's trust anchor.

The checked-in public chain is a deterministic offline verifier fixture, with a
fixed valid time, wrong-host, expired and corrupted-chain checks. It is not
fetched by CI. See [fixture provenance](../checks/live/tests/fixtures/README.md).

## Cache contract

`poznan::cache` accepts `embedded_storage::nor_flash::NorFlash`, compatible with
an ESP32 flash partition adapter (4-byte reads/writes, 4,096-byte erases).
The example reserves **two 32 KiB banks, 64 KiB total flash**; firmware must choose
reserved, nonoverlapping partition offsets. Offsets are not hardcoded device
addresses. Each cache holds the complete result of one exact schedule query;
use a separate bank pair per retained query, or replay its object metadata to
identify the retained schedule. Fetch time is caller-supplied Unix seconds;
the application decides freshness and can show stale data while offline.

```rust
use poznan::cache::{Buffers, Cache};
# fn example<F: embedded_storage::nor_flash::NorFlash>(flash: &mut F, chunks: &[&[u8]], http_succeeded: bool) -> Result<(), poznan::cache::Error> {
let cache = Cache::new([0, 32768], 32768); // offsets relative to a reserved partition
let (mut tag, mut text, mut metadata, mut page) =
    ([0; 512], [0; 256], [0; 512], [0; 256]);
let mut update = cache.update(flash, Buffers {
    tag: &mut tag, text: &mut text, metadata: &mut metadata, write: &mut page,
}, 1792368000)?;
for chunk in chunks { update.feed(chunk)?; }
let prepared = update.finish()?; // parser valid; payload still unpublished
if http_succeeded { prepared.commit()?; } // otherwise drop it
# Ok(()) }
```

An update erases the inactive bank, streams five bytes per pickup plus UTF-8
object metadata, and writes a checksummed versioned header last. No word is
written twice between erases. Reopen streams CRCs over both banks and chooses
the newest valid generation, falling back to the older bank after a torn or
corrupt update. CRCs detect accidental corruption; they are not authentication.
No third-party writer may modify these banks concurrently.

Dropping/cancelling, unknown source layout, insufficient field buffers, full
flash, or transport failure preserves the previous generation. All capacity
limits produce errors rather than truncation. Finishing parsing produces a
`Prepared` value; only the explicit commit publishes it. The HTTPS adapter makes
that decision after validated HTTP completion. Replay uses a read buffer plus
one object's metadata, checks dates/counts/CRC, and preserves exact IDs.

The offline suite injects interrupted erase/payload/header writes at every byte
prefix of each mutation, tests full-cache and malformed-input recovery, and
replays after reopening. The standalone allocation probe streams 8 MiB of
irrelevant HTML through parsing and another 8 MiB through parse/write/reopen/
replay with **zero heap allocations**.
