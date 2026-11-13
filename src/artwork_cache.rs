//! Persistent HTTP image cache shared by library cards and now-playing artwork.
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{blocking::Client, header};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_IMAGE: u64 = 10_000_000;
const MAX_RECORD: u64 = 14_000_000;
const CACHE_BYTES: u64 = 256 * 1024 * 1024;
const CACHE_FILES: usize = 1024;

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    address: String,
    etag: Option<String>,
    last_modified: Option<String>,
    checked_at: u64,
    max_age: u64,
    image: String,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn cache_file(directory: &Path, address: &str) -> PathBuf {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    address.hash(&mut hash);
    directory.join(format!("image-{:016x}.json", hash.finish()))
}
fn read_record(path: &Path, address: &str) -> Option<(Record, Vec<u8>)> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_RECORD {
        return None;
    }
    let record: Record = serde_json::from_reader(file.take(MAX_RECORD + 1)).ok()?;
    // Verify the URL as well as its hash, and reject damaged cache data.
    if record.address != address {
        return None;
    }
    let bytes = STANDARD.decode(&record.image).ok()?;
    validate(&bytes).ok()?;
    Some((record, bytes))
}
fn validate(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_IMAGE {
        return Err("Artwork is too large".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|e| e.to_string())?;
    Ok(())
}
fn max_age(control: &str) -> u64 {
    let directives: Vec<_> = control.split(',').map(str::trim).collect();
    if directives
        .iter()
        .any(|d| d.eq_ignore_ascii_case("no-cache") || d.eq_ignore_ascii_case("no-store"))
    {
        return 0;
    }
    directives
        .iter()
        .find_map(|d| {
            let (key, value) = d.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case("max-age")
                .then(|| value.trim().trim_matches('"').parse::<u64>().ok())
                .flatten()
        })
        .unwrap_or(0)
        .min(86400)
}
fn no_store(control: &str) -> bool {
    control
        .split(',')
        .any(|d| d.trim().eq_ignore_ascii_case("no-store"))
}
fn write_record(directory: &Path, path: &Path, record: &Record) {
    let Ok(bytes) = serde_json::to_vec(record) else {
        return;
    };
    if fs::create_dir_all(directory).is_err() {
        return;
    }
    let temporary = directory.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    if fs::write(&temporary, bytes).is_ok() {
        let _ = fs::rename(&temporary, path);
    }
    let _ = fs::remove_file(temporary);
    prune(directory);
}
fn prune(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("image-") || !name.ends_with(".json") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            Some((
                metadata.modified().unwrap_or(UNIX_EPOCH),
                metadata.len(),
                entry.path(),
            ))
        })
        .collect();
    files.sort_by_key(|file| file.0);
    let mut total: u64 = files.iter().map(|file| file.1).sum();
    let mut count = files.len();
    for (_, length, path) in files {
        if total <= CACHE_BYTES && count <= CACHE_FILES {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(length);
            count -= 1;
        }
    }
}

pub fn download(client: &Client, address: &str) -> Result<Vec<u8>, String> {
    let directory = crate::model::Library::path().with_file_name("artwork-cache");
    download_in(client, address, &directory)
}
fn download_in(client: &Client, address: &str, directory: &Path) -> Result<Vec<u8>, String> {
    let parsed = url::Url::parse(address).map_err(|_| "Invalid artwork URL")?;
    if !["http", "https"].contains(&parsed.scheme()) {
        return Err("Unsupported artwork URL".into());
    }
    let path = cache_file(directory, address);
    let cached = read_record(&path, address);
    if let Some((record, bytes)) = &cached
        && now() < record.checked_at.saturating_add(record.max_age)
    {
        return Ok(bytes.clone());
    }
    let result = (|| {
        let mut request = client.get(parsed);
        if let Some((record, _)) = &cached {
            if let Some(etag) = &record.etag {
                request = request.header(header::IF_NONE_MATCH, etag);
            }
            if let Some(modified) = &record.last_modified {
                request = request.header(header::IF_MODIFIED_SINCE, modified);
            }
        }
        let mut response = request
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        let control = response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            let (record, bytes) = cached
                .as_ref()
                .ok_or("Artwork server returned 304 without a cached image")?;
            let mut record = record.clone();
            let bytes = bytes.clone();
            record.checked_at = now();
            if let Some(etag) = response
                .headers()
                .get(header::ETAG)
                .and_then(|h| h.to_str().ok())
            {
                record.etag = Some(etag.to_owned());
            }
            if let Some(modified) = response
                .headers()
                .get(header::LAST_MODIFIED)
                .and_then(|h| h.to_str().ok())
            {
                record.last_modified = Some(modified.to_owned());
            }
            if let Some(control) = &control {
                record.max_age = max_age(control);
            }
            if control.as_deref().is_some_and(no_store) {
                let _ = fs::remove_file(&path);
            } else {
                write_record(directory, &path, &record);
            }
            return Ok(bytes);
        }
        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        let modified = response
            .headers()
            .get(header::LAST_MODIFIED)
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        if response.content_length().is_some_and(|n| n > MAX_IMAGE) {
            return Err("Artwork is too large".into());
        }
        let mut bytes = Vec::new();
        (&mut response)
            .take(MAX_IMAGE + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        validate(&bytes)?;
        if control.as_deref().is_some_and(no_store) {
            let _ = fs::remove_file(&path);
        } else {
            write_record(
                directory,
                &path,
                &Record {
                    address: address.to_owned(),
                    etag,
                    last_modified: modified,
                    checked_at: now(),
                    max_age: control.as_deref().map(max_age).unwrap_or(0),
                    image: STANDARD.encode(&bytes),
                },
            );
        }
        Ok(bytes)
    })();
    // Preserve known-good artwork when offline or when a replacement is invalid.
    result.or_else(|error| cached.map(|(_, bytes)| bytes).ok_or(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };
    struct Reply {
        status: &'static str,
        headers: &'static str,
        body: Vec<u8>,
    }
    struct Server {
        address: String,
        stop: mpsc::Sender<()>,
        handle: thread::JoinHandle<Vec<String>>,
    }
    impl Server {
        fn new(replies: Vec<Reply>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = format!("http://{}/cover.webp", listener.local_addr().unwrap());
            let (stop, stopped) = mpsc::channel();
            let handle = thread::spawn(move || {
                let mut requests = Vec::new();
                let mut replies = replies.into_iter();
                let start = Instant::now();
                while stopped.try_recv().is_err() && start.elapsed() < Duration::from_secs(10) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(connection) => connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(error) => panic!("{error}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        request.push_str(&line.to_lowercase());
                    }
                    requests.push(request);
                    let reply = replies.next().expect("Unexpected artwork download");
                    write!(
                        stream,
                        "HTTP/1.1 {}\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
                        reply.status,
                        reply.headers,
                        reply.body.len()
                    )
                    .unwrap();
                    stream.write_all(&reply.body).unwrap();
                }
                assert!(
                    replies.next().is_none(),
                    "Expected artwork request was not received"
                );
                requests
            });
            Self {
                address,
                stop,
                handle,
            }
        }
        fn finish(self) -> Vec<String> {
            let _ = self.stop.send(());
            self.handle.join().unwrap()
        }
    }
    fn webp(color: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
            .encode(&color, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }
    fn directory() -> PathBuf {
        std::env::temp_dir().join(format!("kaleido-artwork-cache-{}", uuid::Uuid::new_v4()))
    }
    fn client() -> Client {
        Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
    }
    #[test]
    fn persistent_cache_revalidates_etags_updates_changed_images_and_works_offline() {
        let first = webp([255, 0, 0, 255]);
        let changed = webp([0, 255, 0, 255]);
        let server = Server::new(vec![
            Reply {
                status: "200 OK",
                headers: "ETag: \"v1\"\r\nLast-Modified: Wed, 01 Oct 2025 10:00:00 GMT\r\n",
                body: first.clone(),
            },
            Reply {
                status: "304 Not Modified",
                headers: "ETag: \"v1\"\r\n",
                body: vec![],
            },
            Reply {
                status: "200 OK",
                headers: "ETag: \"v2\"\r\n",
                body: changed.clone(),
            },
        ]);
        let directory = directory();
        let address = server.address.clone();
        assert_eq!(download_in(&client(), &address, &directory).unwrap(), first);
        assert_eq!(download_in(&client(), &address, &directory).unwrap(), first);
        assert_eq!(
            download_in(&client(), &address, &directory).unwrap(),
            changed
        );
        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert!(!requests[0].contains("if-none-match"));
        assert!(requests[1].contains("if-none-match: \"v1\""));
        assert!(requests[1].contains("if-modified-since: wed, 01 oct 2025 10:00:00 gmt"));
        assert!(requests[2].contains("if-none-match: \"v1\""));
        assert_eq!(
            download_in(&client(), &address, &directory).unwrap(),
            changed
        );
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn fresh_cache_avoids_requests_and_last_modified_revalidates_without_etags() {
        let bytes = webp([0, 0, 255, 255]);
        for fresh in [true, false] {
            let mut replies = vec![Reply {
                status: "200 OK",
                headers: if fresh {
                    "Cache-Control: public, max-age=3600\r\n"
                } else {
                    "Last-Modified: Wed, 01 Oct 2025 10:00:00 GMT\r\n"
                },
                body: bytes.clone(),
            }];
            if !fresh {
                replies.push(Reply {
                    status: "304 Not Modified",
                    headers: "",
                    body: vec![],
                });
            }
            let server = Server::new(replies);
            let directory = directory();
            assert_eq!(
                download_in(&client(), &server.address, &directory).unwrap(),
                bytes
            );
            assert_eq!(
                download_in(&client(), &server.address, &directory).unwrap(),
                bytes
            );
            let requests = server.finish();
            assert_eq!(requests.len(), if fresh { 1 } else { 2 });
            if !fresh {
                assert!(requests[1].contains("if-modified-since:"));
                assert!(!requests[1].contains("if-none-match:"));
            }
            fs::remove_dir_all(directory).unwrap();
        }
    }
    #[test]
    fn no_store_is_respected_and_broken_cache_is_replaced() {
        let bytes = webp([255, 255, 0, 255]);
        let server = Server::new(vec![Reply {
            status: "200 OK",
            headers: "Cache-Control: no-store\r\n",
            body: bytes.clone(),
        }]);
        let directory = directory();
        fs::create_dir_all(&directory).unwrap();
        let path = cache_file(&directory, &server.address);
        fs::write(&path, b"broken record").unwrap();
        assert_eq!(
            download_in(&client(), &server.address, &directory).unwrap(),
            bytes
        );
        assert!(!path.exists());
        server.finish();
        fs::remove_dir_all(directory).unwrap();
    }
}
