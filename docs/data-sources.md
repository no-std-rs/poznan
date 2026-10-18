# Poznań source inventory

Checked on 2026-10-19. Use official producers and retain their attribution and
retrieval time. A working endpoint does not establish completeness or freshness.

| Data | Producer / source | State in this repository |
| --- | --- | --- |
| Waste pickup by exact building group and service object | [City schedule](https://www.poznan.pl/mim/odpady/harmonogramy.html) | Streaming HTML, lookups, verified HTTPS and transactional NOR cache |
| ZTM stop points, zones and line labels | [City map API](https://egov.psnc.pl/node/29) | Streaming GeoJSON, verified HTTPS and hosted atomic file cache |
| Parking meters, paid zones and payment support | [City map API](https://egov.psnc.pl/node/29) | Streaming GeoJSON, verified HTTPS and hosted atomic file cache |
| Air monitoring stations, sensors and measurements | [GIOŚ current API](https://powietrze.gios.gov.pl/pjp/content/api?lang=en) | Source inspected; provider not yet implemented |
| Scheduled transit trips and calendars | [ZTM developer portal](https://www.ztm.poznan.pl/otwarte-dane/dla-deweloperow/) | Existing neighboring `timetable`/`gtfs` implementation; not duplicated here |
| Further municipal datasets | [City catalog](https://www.poznan.pl/opendata/) | Discovery source; no blanket ingestion claim |

## Next candidate: air measurements

GIOŚ's supported paths are under `https://api.gios.gov.pl/pjp-api/v1/rest/`:
`station/findAll`, `station/sensors/{stationId}`, `data/getData/{sensorId}` and
`aqindex/getIndex/{stationId}`. The older unversioned service was withdrawn in
2025. The new JSON-LD response uses Polish field names and pagination; do not
apply an old English-key parser or assume one page contains the whole country.

The live `station/findAll?size=500` response reported one page and five stations
with city `Poznań` (IDs 931, 932, 944, 10148, 16493). These are discovery results,
not a permanent list to hardcode. Presence in the station list does not imply a
live automatic measurement. Respect published endpoint rate limits, null
measurements and source timestamps. The station directory currently has a much
lower request allowance than measurement endpoints; discover once, cache the
IDs and refresh on a controlled cadence. A future adapter needs its own TLS
trust/heap validation because it uses a different origin.
