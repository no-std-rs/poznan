use super::*;
use std::{
    format,
    future::Future,
    string::String,
    task::{Context, Poll, Waker},
    vec::Vec,
};
#[path = "../../../tests/support/mod.rs"]
mod support;
struct Source<'a>(&'a [u8]);
impl ErrorType for Source<'_> {
    type Error = embedded_io_async::ErrorKind;
}
impl Read for Source<'_> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        let n = out.len().min(self.0.len()).min(37);
        out[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        Ok(n)
    }
}
impl TryBufRead for Source<'_> {}
fn block_on<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("test source unexpectedly suspended"),
    }
}
fn response(flash: &mut support::Flash<'_>, bytes: &[u8]) -> Result<Snapshot, Error> {
    let (mut headers, mut tag, mut text, mut metadata, mut write) =
        ([0; 2048], [0; 512], [0; 256], [0; 512], [0; 256]);
    let mut source = Source(bytes);
    block_on(async {
        let response = Response::read(&mut source, Method::GET, &mut headers)
            .await
            .map_err(Error::Http)?;
        store_response(
            response,
            flash,
            Cache::new([0, 4096], 4096),
            Buffers {
                tag: &mut tag,
                text: &mut text,
                metadata: &mut metadata,
                write: &mut write,
            },
            123,
        )
        .await
    })
}
fn fixed(headers: &str, html: &[u8], length: usize) -> Vec<u8> {
    let mut bytes =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\n{headers}\r\n").into_bytes();
    bytes.extend_from_slice(html);
    bytes
}
#[test]
fn framed_download_commits_and_truncated_or_ambiguous_downloads_preserve_old() {
    let html = include_bytes!("../../../tests/fixtures/schedule.html");
    let mut bytes = [0xff; 8192];
    let mut flash = support::Flash::new(&mut bytes);
    let old = response(&mut flash, &fixed("", html, html.len())).unwrap();
    let chunked = format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
        html.len(),
        core::str::from_utf8(html).unwrap()
    );
    let mut failures = Vec::new();
    failures.push(fixed("", html, html.len() + 1)); // Valid HTML, incomplete transfer.
    failures.push(fixed("Content-Encoding: gzip\r\n", html, html.len()));
    failures.push(fixed("Transfer-Encoding: chunked\r\n", html, html.len()));
    failures.push(fixed("Content-Length: 10\r\n", html, html.len()));
    failures.push(chunked.as_bytes()[..chunked.len() - 5].to_vec());
    failures.push(
        String::from_utf8(fixed("", html, html.len()))
            .unwrap()
            .replacen("200 OK", "302 Found", 1)
            .into_bytes(),
    );
    failures.push(
        format!(
            "HTTP/1.1 200 OK\r\n\r\n{}",
            core::str::from_utf8(html).unwrap()
        )
        .into_bytes(),
    );
    for failure in failures {
        assert!(response(&mut flash, &failure).is_err());
        assert_eq!(
            Cache::new([0, 4096], 4096)
                .open(&mut flash, &mut [0; 256])
                .unwrap(),
            Some(old)
        );
    }
    let new = response(&mut flash, chunked.as_bytes()).unwrap();
    assert_eq!(new.generation(), old.generation() + 1);
    assert_eq!(new.stats(), old.stats());
}
