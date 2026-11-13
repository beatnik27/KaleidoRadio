use crate::model::{RemoteConnection, Station};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::time::Duration;
pub fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent("KaleidoRadio-Rust/0.1")
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}
#[derive(Deserialize)]
struct RadioStation {
    stationuuid: String,
    name: String,
    url: String,
    url_resolved: Option<String>,
    homepage: Option<String>,
    favicon: Option<String>,
    tags: Option<String>,
    countrycode: Option<String>,
    codec: Option<String>,
    bitrate: Option<u32>,
}
pub fn search(query: &str) -> Result<Vec<Station>, String> {
    let client = client()?;
    let mut last_error = String::new();
    for server in ["de1", "de2", "at1"] {
        let result = (|| {
            let path = if query.trim().is_empty() {
                "topclick/40"
            } else {
                "search"
            };
            let response = client
                .get(format!(
                    "https://{server}.api.radio-browser.info/json/stations/{path}"
                ))
                .query(&[
                    ("name", query),
                    ("hidebroken", "true"),
                    ("order", "votes"),
                    ("reverse", "true"),
                ])
                .send()
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?;
            let stations: Vec<RadioStation> = response.json().map_err(|e| e.to_string())?;
            let mut seen = std::collections::HashSet::new();
            Ok(stations
                .into_iter()
                .filter_map(|r| {
                    let address = r
                        .url_resolved
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or(r.url);
                    let mut s = Station::new(r.name.trim(), address);
                    if s.validate().is_err() || !seen.insert(s.stream_url.clone()) {
                        return None;
                    }
                    s.station_uuid = Some(r.stationuuid);
                    s.homepage = r.homepage;
                    s.favicon_url = r.favicon;
                    s.tags = r.tags;
                    s.country_code = r.countrycode;
                    s.codec = r.codec;
                    s.bitrate = r.bitrate;
                    Some(s)
                })
                .collect())
        })();
        match result {
            Ok(stations) => return Ok(stations),
            Err(e) => last_error = e,
        }
    }
    Err(format!("Radio Browser is unavailable: {last_error}"))
}
#[derive(Clone, Debug)]
pub struct RemoteEntry {
    pub name: String,
    pub url: String,
    pub directory: bool,
    pub modified_at: Option<i64>,
}
fn webdav_connection_error(error: reqwest::Error) -> String {
    use std::error::Error;
    let error = error.without_url();
    let mut details = vec![error.to_string()];
    let mut cause = error.source();
    while let Some(source) = cause {
        let detail = source.to_string();
        if details.last() != Some(&detail) { details.push(detail); }
        cause = source.source();
    }
    format!("Could not connect to WebDAV server: {}. Check the server address and macOS Local Network access.", details.join(": "))
}

pub fn valid_remote_url(address: &str) -> bool {
    let Ok(url) = url::Url::parse(address) else { return false; };
    if url.scheme() == "smb" { return crate::smb::valid_url(address); }
    if url.scheme() == "sftp" { return crate::sftp::valid_url(address); }
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
        && url.username().is_empty() && url.password().is_none()
}

pub fn browse(
    connection: &RemoteConnection,
    password: &str,
    address: &str,
) -> Result<Vec<RemoteEntry>, String> {
    if connection.url.starts_with("sftp:") {
        return crate::sftp::browse(connection, password, address);
    }
    if url::Url::parse(&connection.url).is_ok_and(|url| url.scheme() == "smb") {
        return crate::smb::browse(connection, password, address);
    }
    let base = url::Url::parse(&connection.url).map_err(|_| "Invalid server URL")?;
    let target = url::Url::parse(address).map_err(|_| "Invalid directory URL")?;
    if base.origin() != target.origin() {
        return Err("Directory must be on the configured server.".into());
    }
    let response = client()?.request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(),target.clone()).basic_auth(&connection.username,Some(password)).header("Depth","1").header("Content-Type","application/xml").body(r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:displayname/><d:resourcetype/><d:getcontenttype/><d:getlastmodified/></d:prop></d:propfind>"#).send().map_err(webdav_connection_error)?.error_for_status().map_err(|e|format!("WebDAV returned {}",e.status().map_or(0,|s|s.as_u16())))?.text().map_err(|_|"Invalid WebDAV response")?;
    parse_webdav(&target, &response)
}
pub fn parse_webdav(base: &url::Url, xml: &str) -> Result<Vec<RemoteEntry>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|_| "Invalid WebDAV XML")?;
    let mut entries = Vec::new();
    for response in doc
        .descendants()
        .filter(|n| n.tag_name().name() == "response")
    {
        let Some(href) = response
            .children()
            .find(|n| n.tag_name().name() == "href")
            .and_then(|n| n.text())
        else {
            continue;
        };
        let Ok(url) = base.join(href) else {
            continue;
        };
        if url.origin() != base.origin()
            || url.path().trim_end_matches('/') == base.path().trim_end_matches('/')
            || !url.username().is_empty()
            || url.password().is_some()
        {
            continue;
        }
        let directory = response
            .descendants()
            .any(|n| n.tag_name().name() == "collection");
        if !directory && !crate::model::is_media(std::path::Path::new(url.path())) {
            continue;
        }
        let name = response
            .descendants()
            .find(|n| n.tag_name().name() == "displayname")
            .and_then(|n| n.text())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                url.path()
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("Media")
                    .to_owned()
            });
        let modified_at = response
            .descendants()
            .find(|n| n.tag_name().name() == "getlastmodified")
            .and_then(|n| n.text())
            .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
            .map(|d| d.timestamp());
        entries.push(RemoteEntry {
            name,
            url: url.to_string(),
            directory,
            modified_at,
        });
    }
    entries.sort_by_key(|e| (!e.directory, e.name.to_lowercase()));
    Ok(entries)
}
// Match macOS: only artist/track metadata that differs from the station identity
// should trigger a song artwork search.
pub fn should_lookup_artwork(title: &str, station: &Station) -> bool {
    let has_song = [" - ", " – ", " — "].iter().any(|separator| {
        title
            .trim()
            .split_once(separator)
            .is_some_and(|(artist, track)| !artist.trim().is_empty() && !track.trim().is_empty())
    });
    if !has_song {
        return false;
    }
    fn normalize(value: &str) -> String {
        value
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
    fn tokens(value: &str) -> std::collections::HashSet<String> {
        normalize(value)
            .split_whitespace()
            .filter(|s| {
                s.chars().count() > 2
                    && !["radio", "live", "stream", "fm", "am", "the", "and"].contains(s)
            })
            .map(str::to_owned)
            .collect()
    }
    let metadata = tokens(title);
    let identity = tokens(&format!(
        "{} {} {}",
        station.name,
        station.homepage.as_deref().unwrap_or(""),
        station.stream_url
    ));
    let title = normalize(title);
    let name = normalize(&station.name);
    !metadata.is_empty()
        && !metadata.is_subset(&identity)
        && !(title == name || name.contains(&title) || title.contains(&name))
}

pub fn artwork(title: &str, fallback: Option<&str>) -> Result<Vec<u8>, String> {
    let client = client()?;
    if !title.trim().is_empty() {
        let song = (|| {
            let value: serde_json::Value = client
                .get("https://itunes.apple.com/search")
                .query(&[("term", title), ("entity", "song"), ("limit", "1")])
                .send()
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json()
                .map_err(|e| e.to_string())?;
            let address = value["results"][0]["artworkUrl100"]
                .as_str()
                .ok_or("No song artwork available")?
                .replace("100x100bb", "600x600bb");
            download_artwork(&client, &address)
        })();
        if song.is_ok() {
            return song;
        }
    }
    download_artwork(&client, fallback.ok_or("No artwork available")?)
}

fn download_artwork(client: &Client, address: &str) -> Result<Vec<u8>, String> {
    crate::artwork_cache::download(client, address)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artwork_lookup_requires_song_metadata() {
        let station = Station::new("Example Radio", "https://example.org/live");
        for title in [
            "",
            "   ",
            "Example Radio",
            "Live stream",
            "Song title",
            " - Song",
            "Artist - ",
            "Example Radio - Live",
        ] {
            assert!(!should_lookup_artwork(title, &station), "{title:?}");
        }
        for title in ["Artist - Song", "Artist – Song", "Artist — Song"] {
            assert!(should_lookup_artwork(title, &station), "{title:?}");
        }
    }

    #[test]
    fn dav_namespaces_and_origin() {
        let entries=parse_webdav(&url::Url::parse("https://example.org/music/").unwrap(),r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/music/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response><d:response><d:href>/music/album/</d:href><d:propstat><d:prop><d:displayname>Album</d:displayname><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response><d:response><d:href>/music/song.mp3</d:href></d:response><d:response><d:href>https://other.org/secret.mp3</d:href></d:response></d:multistatus>"#).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].directory);
        assert_eq!(entries[0].name, "Album");
    }
}

#[cfg(test)]
mod local_artwork_tests {
    #[test]
    fn reads_embedded_cover_from_encoded_file_url() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cover.mp3");
        let url = url::Url::from_file_path(path).unwrap();
        let bytes = crate::file_metadata::read(url.as_str(), None).unwrap().artwork.unwrap();
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (8, 8));
    }

    #[test]
    fn rejects_non_local_and_missing_files() {
        assert!(crate::file_metadata::read("invalid-url", None).is_err());
        assert!(crate::file_metadata::read("file:///nonexistent-kaleido-song.mp3", None).is_err());
    }
}

#[cfg(test)]
mod video_discovery_tests {
    #[test]
    fn webdav_lists_supported_video_files() {
        let xml = r#"<d:multistatus xmlns:d="DAV:">
            <d:response><d:href>/media/clip.mp4</d:href></d:response>
            <d:response><d:href>/media/portrait.MOV</d:href></d:response>
            <d:response><d:href>/media/movie.m4v</d:href></d:response>
            <d:response><d:href>/media/stream.m3u8</d:href></d:response>
            <d:response><d:href>/media/song.mp3</d:href></d:response>
            <d:response><d:href>/media/notes.txt</d:href></d:response>
        </d:multistatus>"#;
        let entries = super::parse_webdav(&url::Url::parse("https://example.com/media/").unwrap(), xml).unwrap();
        assert_eq!(entries.len(), 5);
        assert!(entries.iter().all(|entry| !entry.directory));
    }
}

#[cfg(test)]
mod webdav_transport_tests {
    use super::*;
    use std::{io::{Read, Write}, net::TcpListener};

    #[test]
    fn authenticated_propfind_lists_audio_and_video() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/media/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 2048];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length = headers.lines().find_map(|line| line.strip_prefix("content-length:").and_then(|value| value.trim().parse::<usize>().ok())).unwrap_or(0);
                    if request.len() >= end + 4 + length { break; }
                }
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("PROPFIND /media/ HTTP/1.1"));
            assert!(request.to_lowercase().contains("depth: 1\r\n"));
            assert!(request.to_lowercase().contains("authorization: basic dxnlcjpwyxnz\r\n"));
            let xml = r#"<d:multistatus xmlns:d="DAV:">
                <d:response><d:href>/media/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
                <d:response><d:href>/media/Album/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
                <d:response><d:href>/media/song.mp3</d:href></d:response>
                <d:response><d:href>/media/video.mp4</d:href></d:response>
            </d:multistatus>"#;
            write!(stream, "HTTP/1.1 207 Multi-Status\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", xml.len(), xml).unwrap();
        });
        let connection = RemoteConnection { name: "Test".into(), url: base.clone(), username: "user".into() };
        let result = browse(&connection, "pass", &base);
        server.join().unwrap();
        let entries = result.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries.iter().filter(|entry| entry.directory).count(), 1);
        assert!(entries.iter().any(|entry| entry.url.ends_with("song.mp3")));
        assert!(entries.iter().any(|entry| entry.url.ends_with("video.mp4")));
    }

    #[test]
    fn transport_errors_include_the_cause_without_request_urls() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = format!("http://{}/?token=private-test-token", listener.local_addr().unwrap());
        drop(listener);
        let error = Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().unwrap().get(&target).send().unwrap_err();
        let message = webdav_connection_error(error);
        assert!(message.contains("Local Network access"));
        assert!(message.contains("connect"));
        assert!(!message.contains("private-test-token"));
        assert!(!message.contains(&target));
    }
}
