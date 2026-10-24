//! Streaming multipart upload into the open show pack.
//!
//! multer pulls the body in chunks. Each file is written to `assets/.incoming`
//! and renamed into the library only after the field completes.

use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_core::Stream;

use crate::media::library::{self, MAX_UPLOAD_BYTES, UploadReport};

pub fn peer_is_loopback(peer: SocketAddr) -> bool {
    peer.ip().is_loopback()
}

pub fn show_pack_root() -> Option<std::path::PathBuf> {
    let show_json = crate::show::SHOW_PACK_ARG.get()?;
    show_json.parent().map(std::path::Path::to_path_buf)
}

struct ByteStream<R> {
    reader: R,
    left: u64,
}

impl<R: Read + Unpin> Stream for ByteStream<R> {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.left == 0 {
            return Poll::Ready(None);
        }
        let mut buf = vec![0u8; 64 * 1024];
        let want = (this.left as usize).min(buf.len());
        match this.reader.read(&mut buf[..want]) {
            Ok(0) => Poll::Ready(Some(Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "upload ended early",
            )))),
            Ok(n) => {
                this.left -= n as u64;
                buf.truncate(n);
                Poll::Ready(Some(Ok(Bytes::from(buf))))
            }
            Err(err) => Poll::Ready(Some(Err(err))),
        }
    }
}

pub struct PrefixedRead<R> {
    prefix: Vec<u8>,
    pos: usize,
    inner: R,
}

impl<R> PrefixedRead<R> {
    pub fn new(prefix: Vec<u8>, inner: R) -> Self {
        Self {
            prefix,
            pos: 0,
            inner,
        }
    }
}

impl<R: Read + Unpin> Read for PrefixedRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos < self.prefix.len() {
            let n = (self.prefix.len() - self.pos).min(buf.len());
            buf[..n].copy_from_slice(&self.prefix[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        } else {
            self.inner.read(buf)
        }
    }
}

/// Read one multipart body and store each `file` field in the show library.
pub fn receive_multipart<R: Read + Unpin + Send>(
    reader: R,
    content_type: &str,
    content_length: u64,
    pack_root: &Path,
) -> Result<Vec<UploadReport>, String> {
    if content_length == 0 || content_length > MAX_UPLOAD_BYTES.saturating_mul(8) {
        return Err("upload is empty or over the request limit".into());
    }
    let boundary = multer::parse_boundary(content_type).map_err(|e| e.to_string())?;
    let stream = ByteStream {
        reader,
        left: content_length,
    };
    let constraints = multer::Constraints::new().size_limit(
        multer::SizeLimit::new()
            .whole_stream(MAX_UPLOAD_BYTES.saturating_mul(8))
            .per_field(MAX_UPLOAD_BYTES),
    );
    pollster::block_on(async move {
        let mut multipart = multer::Multipart::with_constraints(stream, boundary, constraints);
        let mut reports = Vec::new();
        while let Some(mut field) = multipart.next_field().await.map_err(|e| e.to_string())? {
            if field.name() != Some("file") {
                continue;
            }
            let original = field
                .file_name()
                .map(str::to_string)
                .ok_or_else(|| "missing file name".to_string())?;
            let incoming = pack_root.join("assets/.incoming");
            std::fs::create_dir_all(&incoming).map_err(|e| e.to_string())?;
            let partial = incoming.join(format!("up-{}", library::incoming_token()));
            let mut file = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
            let mut written = 0u64;
            let result = async {
                while let Some(chunk) = field.chunk().await.map_err(|e| e.to_string())? {
                    written = written.saturating_add(chunk.len() as u64);
                    if written > MAX_UPLOAD_BYTES {
                        return Err("file is over 512 MB".to_string());
                    }
                    file.write_all(&chunk).map_err(|e| e.to_string())?;
                }
                file.sync_all().map_err(|e| e.to_string())?;
                drop(file);
                library::commit_upload(pack_root, &partial, &original)
            }
            .await;
            if let Err(err) = &result {
                let _ = std::fs::remove_file(&partial);
                return Err(err.clone());
            }
            reports.push(result.unwrap());
            if reports.len() > 64 {
                return Err("too many files in one upload".into());
            }
        }
        if reports.is_empty() {
            return Err("no files in upload".into());
        }
        Ok(reports)
    })
}

/// A raw body with a known length, used by the single-file path.
pub fn receive_raw<R: Read>(
    mut reader: R,
    content_length: u64,
    pack_root: &Path,
    original_name: &str,
) -> Result<UploadReport, String> {
    if content_length == 0 || content_length > MAX_UPLOAD_BYTES {
        return Err("file is empty or over 512 MB".into());
    }
    let incoming = pack_root.join("assets/.incoming");
    std::fs::create_dir_all(&incoming).map_err(|e| e.to_string())?;
    let partial = incoming.join(format!("raw-{}", library::incoming_token()));
    let mut file = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
    let mut left = content_length;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        let want = (left as usize).min(buf.len());
        match reader.read(&mut buf[..want]) {
            Ok(0) => {
                let _ = std::fs::remove_file(&partial);
                return Err("upload ended early".into());
            }
            Ok(n) => {
                if file.write_all(&buf[..n]).is_err() {
                    let _ = std::fs::remove_file(&partial);
                    return Err("could not store file".into());
                }
                left -= n as u64;
            }
            Err(_) => {
                let _ = std::fs::remove_file(&partial);
                return Err("upload interrupted".into());
            }
        }
    }
    let _ = file.sync_all();
    drop(file);
    match library::commit_upload(pack_root, &partial, original_name) {
        Ok(report) => Ok(report),
        Err(err) => {
            let _ = std::fs::remove_file(&partial);
            Err(err)
        }
    }
}

pub fn reports_json(reports: &[UploadReport]) -> String {
    let files: Vec<serde_json::Value> = reports
        .iter()
        .map(|report| {
            serde_json::json!({
                "ok": true,
                "id": report.id,
                "name": report.name,
                "duplicate": report.duplicate,
            })
        })
        .collect();
    serde_json::json!({ "ok": true, "files": files }).to_string()
}
