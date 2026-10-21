# poznan

Allocation-free, `no_std` streaming access to Poznań waste pickup schedules.
The core uses only the `embedded-storage` traits and forbids unsafe Rust.
All variable buffers belong to the caller. Rust 1.91 or later is required.

The first provider is the city's [waste schedule table](https://www.poznan.pl/mim/odpady/harmonogramy.html).
It exposes street, building-group and service-object lookups, then HTML tables
with dated pickups for seven waste fractions. The parser consumes arbitrary
HTTP body chunks, including one-byte chunks, without collecting the response,
building a DOM, running JavaScript, or processing PDFs.

## Try it

The hosted example uses `curl` for verified HTTPS and streams its stdout through
the Rust core. Only this example needs `std` and the external `curl` executable.
The library accepts body bytes from any embedded or hosted transport.

```sh
cargo run --example waste -- streets Dąbrowskiego
cargo run --example waste -- numbers 'JANA HENRYKA DĄBROWSKIEGO' 1
cargo run --example waste -- objects 'JANA HENRYKA DĄBROWSKIEGO' 12
cargo run --example waste -- schedule 'JANA HENRYKA DĄBROWSKIEGO' 12
cargo run --example waste -- object '1/2275.'
```

Use the exact values returned by lookups. Building groups such as
`104-114/DŁUGOSZA 1-5` are opaque source values. The same address may have
different calendars for `1/2275` and `1/2275.`; punctuation is significant.
An object-only query is supported. A schedule query without an object emits
every returned service object, retaining its property type, sector and address.

Output is newline-delimited JSON. `fraction` uses the city's codes: `ZM`
(mixed), `PAP` (paper), `TWS` (metals/plastics), `SZK` (glass), `BIO`, `WYST`
(bulky waste), and `CHO` (Christmas trees). Dates are local civil dates.

```json
{"type":"pickup","object":"1/2275.","fraction":"ZM","date":"2026-10-19"}
```

Every output record is provisional until the final `complete` record **and a
successful process exit**. A consumer should stage records in flash, a temporary
file, or a transaction and commit only after both. Transfer or parser failure
returns a nonzero status and does not emit `complete`. Large schedules can thus
be written directly to storage without keeping them in RAM.

Parse a saved response or pipe bytes from another transport:

```sh
cargo run --example waste -- parse-schedule tests/fixtures/schedule.html
curl --fail --get 'https://www.poznan.pl/mim/odpady/harmonogramy.html' \
  --data-urlencode action=search --data-urlencode co=waste_schedule \
  --data-urlencode 'ws_object=1/2275.' > /tmp/poznan-waste.html
cargo run --example waste -- parse-schedule /tmp/poznan-waste.html
```

## Embedded use

`Query::path` percent-encodes UTF-8 parameters into a caller buffer for a GET on
`https://www.poznan.pl`. `LookupParser` borrows tag/value buffers;
`ScheduleParser` borrows tag/text/metadata buffers. Each callback borrows the
current object only for that invocation. Consume it immediately or copy selected
values to bounded application storage.

```rust
use poznan::{Error, Event, ScheduleParser, Stats};

fn collect<'a>(chunks: impl IntoIterator<Item = &'a [u8]>) -> Result<Stats, Error> {
    let (mut tag, mut text, mut metadata) = ([0; 512], [0; 256], [0; 512]);
    let mut parser = ScheduleParser::new(&mut tag, &mut text, &mut metadata);
    for chunk in chunks {
        parser.feed(chunk, |event| {
            if let Event::Pickup { object, fraction, date } = event {
                // Filter, render, or stage this event in external storage.
                let _ = (object.id, fraction, date);
            }
            Ok(())
        })?;
    }
    parser.finish() // Also require the transport to report successful completion.
}
```

The example's schedule buffers total 1,280 bytes, independent of response size
and number of objects. Parser state, the body read buffer, and transport cost
are additional. `cargo run --example footprint` reports host ABI storage sizes;
these are not measurements of total device RAM or peak stack usage. `curl` is
a separate hosted process whose HTTP/TLS allocations are outside the core.

The parser skips unrelated long tags, comments and script/style bodies. It
decodes UTF-8 text and common/numeric HTML entities in captured fields, checks
calendar dates, and rejects malformed or duplicate days, unknown fraction codes,
incomplete tables and buffer overflow. Failed or finished parsers are closed.

## Flash cache and the 100 KB ESP32 pipeline

`poznan::cache` streams into two caller-selected NOR flash banks with a 256-byte
write buffer. It publishes only after successful parsing and HTTP completion;
failed or interrupted updates preserve the previous schedule. Reopening checks
both banks and replays from flash using bounded buffers.

The separate `adapters/https` crate supplies verified embedded TLS and framed
HTTP. The ESP32-C3 compiler budget is **90,808 / 100,000 bytes**, including a
24 KiB RSA arena, 24 KiB stack allowance and TCP buffers. A live refresh stored
594 pickups in **3,089 payload bytes**; the whole pipeline's host heap peaked at
22,272 bytes with no arena misses. Physical stack usage still needs board
measurement. The 100 KB covers this pipeline; existing Wi-Fi/application memory
is outside that allowance.

See [ESP32 budget, cache usage and live reproduction](docs/esp32.md).

## Source contract and limits

Observed on 2026-10-18. Public GET endpoints under `/mim/public/odpady/`:

| Endpoint | Parameters |
| --- | --- |
| `waste_street_lookup.html` | `ws_street` (at least three characters) |
| `waste_number_lookup.html` | `ws_street`, `ws_number` (substring/prefix filter) |
| `waste_object_lookup.html` | `ws_street`, `ws_number`, `ws_object` (may be empty) |

Schedules use `/mim/odpady/harmonogramy.html` with `action=search`,
`co=waste_schedule`, `ws_street`, `ws_number`, and optional `ws_object`.
These are the website's observed interfaces, not a promised stable bulk API.
No citywide completeness claim is made: lookup results and published schedules
can be absent or change. The core deliberately targets this HTML shape; it is
not a general HTML5 parser. Metadata consists of the four observed div fields
in order, followed by a `schedule_*` table inside `waste-schedule`.
An empty lookup is valid; an empty or missing schedule is an error.

Data source: **Miasto Poznań, serwis poznan.pl**. Record retrieval time and
describe transformations when redistributing derived data. Code is licensed
under MIT or Apache-2.0, at your option; source data retains its own terms.

## Validation

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --doc
cargo check --locked --lib --target riscv32imc-unknown-none-elf
cargo check --locked --lib --target wasm32-unknown-unknown
cargo test --locked --manifest-path adapters/https/Cargo.toml
cargo test --locked --manifest-path checks/live/Cargo.toml
cargo fetch --locked --manifest-path checks/ram/Cargo.toml
python3 checks/check_ram.py
```

The offline suite covers arbitrary chunk sizes, every truncated schedule prefix,
UTF-8/entities, grouped buildings, distinct object IDs, source changes, invalid
dates, overflow and sink failure. A standalone allocation-counter executable streams
8 MiB of irrelevant body text followed by the fixture, with zero core heap
allocations. HTML fixtures are synthetic; the TLS fixture is the public city
certificate chain. Tests do not contact city services.

See [repository automation](docs/runnerless.md) for CI, Runnerless enrollment and
Codex review enforcement.
