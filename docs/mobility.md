# City mobility feeds

The city provides ZTM stop points and parking meter locations through its
[documented map API](https://egov.psnc.pl/node/29). Both use `www.poznan.pl`,
so the existing verified embedded HTTPS adapter and trust anchor apply.

| Feed | Request path | Records observed on 2026-10-19 |
| --- | --- | ---: |
| ZTM stop points | `/mim/plan/map_service.html?mtype=pub_transport&co=cluster` | 3,334 |
| Parking meters | `/mim/plan/map_service.html?mtype=pub_transport&co=parking_meters` | 691 |

Attribution: **Miasto Poznań, serwis poznan.pl**. The city's
[open-data terms](https://www.poznan.pl/mim/plan/-%2Cp%2C33903%2C33906.html)
require source attribution. Record retrieval time when caching or distributing
derived data. Source records can change; these counts describe the observed
responses and do not prove permanent or citywide completeness.

## Use and cache

```sh
cargo run --release --locked --manifest-path checks/live/Cargo.toml -- \
  stops /tmp/poznan-stops.ndjson
cargo run --release --locked --manifest-path checks/live/Cargo.toml -- \
  meters /tmp/poznan-meters.ndjson

# Offline input uses the same parser and cache publication path.
cargo run --release --locked --manifest-path checks/live/Cargo.toml -- \
  parse-stops tests/fixtures/stops.json /tmp/test-stops.ndjson
```

The hosted probe streams verified HTTPS into a temporary NDJSON file in the
destination directory. It syncs the complete file, renames it over the previous
cache, then syncs the directory. A parser, HTTP, write or capacity error before
publication removes the temporary file and preserves the previous cache. The
final record contains the item count and retrieval time. Offline readers can
open the published file directly; no request is needed.

The live stop response was 674,671 bytes and produced 514,604 cached bytes.
The meter response was 148,176 bytes and produced 125,873 cached bytes. An
independent Python JSON decoder compared every emitted field, ID and coordinate
against both original responses: all 3,334 stops and 691 meters matched.
The host probe's embedded HTTPS/parser/output path peaked at **22,272 bytes**
in its 24 KiB first-fit arena for each feed, with zero misses. These are host
allocation measurements, not physical ESP32 stack measurements.

A complete city dataset needs persistent capacity proportional to its output;
RAM stays bounded. The waste-specific 64 KiB NOR cache format is not used for
mobility records. Embedded applications can filter records by ID, zone or
location before staging selected records in flash, or stream all records to a
larger reserved partition or SD storage. Publication must wait until the entire
parse and transfer succeed. The supplied atomic file cache is a hosted adapter;
there is no mobility-specific NOR writer in the core.

## Core contract

`mobility::Parser::new(feed, feature, strings)` borrows one JSON value buffer
(suggested 2,048 bytes) and decoded-string storage (512 bytes). It frames each
feature, validates its complete JSON grammar, then calls back with a borrowed
`Record`. The callback must consume or copy that record before returning.
There is no DOM, heap allocation, collection-sized index, or whole-response copy.
The same feature buffer validates top-level metadata and unknown values; every
individual value must fit. All records remain provisional until `finish()` and
the transport succeed. A failing or finished parser is closed.

- Stop records retain exact ID, name, zone, raw route-type code and line labels.
  They describe stop locations and listed routes; they contain no departure ETA.
  The ZTM feed includes the surrounding service area as published by the city.
- Meter records retain ID, street, zone and payment flags: `bilon` (coins),
  `karta` (card), `blik`, and `peka`. `TAK`/`NIE` become true/false. Missing,
  blank or null flags remain unknown (`None` / JSON null). Other values fail.
  Meter location and payment support do not establish parking availability,
  fees, or whether a particular space is legal at the current time.
- Coordinates are finite longitude/latitude pairs with geographic range checks.
  Geometry must be `Point`. The observed legacy CRS declaration must say
  `EPSG` / `4326`; omission uses GeoJSON's longitude/latitude convention.
  An incompatible CRS fails even when it appears after the features.
- Root type must be `FeatureCollection`; a `features` array is mandatory.
  Empty collections are valid. Error envelopes, missing fields, duplicate
  requested fields, wrong types, trailing data/commas, malformed numbers,
  invalid UTF-8/escapes and more than 16 nested containers fail visibly.

`poznan_https::mobility` combines this parser with the same TLS/HTTP validation
as waste refresh. It returns the count only after validated framing and parser
EOF. `HttpWorkspace` is const-initializable for static storage; the output sink
is caller-owned and may stage to external storage. Run only one pipeline at a
time unless budgeting another full workspace and TLS arena.

## Memory and validation

`python3 checks/check_ram.py` now gates both actual ESP32-C3 futures:

| Pipeline | Workspace + caller buffers | Future | Reserves | Total / 100,000 B |
| --- | ---: | ---: | ---: | ---: |
| Waste | 32,216 | 2,472 | 56,320 | **91,008** |
| Mobility | 32,216 | 2,288 | 56,320 | **90,824** |

Reserves include the 24 KiB RSA arena, 24 KiB stack allowance, TCP buffers and
1 KiB for bounded output/RNG/adapter/executor overhead. The mobility probe uses a
stateless sink; any retained application records or a sink larger than that
allowance need their own budget. Existing Wi-Fi/application memory remains
outside the available pipeline budget. Physical stack high-water validation is
still pending; see [the ESP32 assumptions](esp32.md).

The shared transport increases the waste future by 200 bytes over the first
cache implementation (90,808 total before this change). Both profiles remain
below the same hard limit. LLVM may merge equal size functions into aliases;
the gate follows those symbols instead of substituting hardcoded sizes.

Offline fixtures are synthetic. Tests cover every chunk size, all truncated
prefixes, metadata after records, JSON escapes, coordinates/CRS, nullable payment
flags, malformed schemas, overflow, sink failure and 20,000 streamed features.
The standalone allocation probe feeds another 20,000 one-byte-chunk records
without allocating. Hosted tests verify successful cache replacement and that
truncated or incompatible-CRS refreshes retain the old file. Live feeds are
smoke tests only and are never a CI dependency.
