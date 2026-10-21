# Public TLS fixture

`city-chain.bin` is the public leaf + intermediate served by `www.poznan.pl:443`,
retrieved with OpenSSL on 2026-10-19. It contains no private key. It is encoded as
a TLS 1.3 Certificate body: empty context, u24 entry-list length, then each u24
DER certificate length, certificate and empty u16 extensions.

The leaf CN is `www.poznan.pl`, issued by GeoTrust TLS RSA CA G1, valid 2026-05-12
through 2026-11-26. Its intermediate chains to DigiCert Global Root G2. The root
in `adapters/https/certs/` was extracted from the system CA trust bundle and
matched against this chain using OpenSSL. Tests use fixed Unix time 1792368000
rather than the current date, so expiration does not make offline CI flaky.

To refresh, inspect the live chain with `openssl s_client -connect
www.poznan.pl:443 -servername www.poznan.pl -showcerts`, verify it with the local
trust bundle, convert each certificate with `openssl x509 -outform DER`, and
encode the body above. Review a changed trust anchor separately from a changed
leaf fixture. Never disable hostname, validity, chain or signature checks to
make a changed server work.
