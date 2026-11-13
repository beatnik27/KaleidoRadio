use lofty::{
    config::ParseOptions, file::TaggedFileExt, picture::PictureType, probe::Probe, tag::Accessor,
};
use std::io::{self, Read, Seek, SeekFrom};

#[derive(Clone, Default)]
pub struct Metadata {
    pub title: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub artwork: Option<Vec<u8>>,
}

pub fn read(address: &str, auth: Option<&(String, String)>) -> Result<Metadata, String> {
    read_options(address, auth, true)
}

pub fn read_summary(address: &str, auth: Option<&(String, String)>) -> Result<Metadata, String> {
    read_options(address, auth, false)
}

fn read_options(address: &str, auth: Option<&(String, String)>, cover: bool) -> Result<Metadata, String> {
    if address.starts_with("sftp:") {
        let local = crate::sftp::resolve(address, auth)?;
        return read_options(&local, None, cover);
    }
    if address.starts_with("smb:") {
        let local = crate::smb::resolve(address, auth)?;
        return read_options(&local, None, cover);
    }
    let is_video = crate::video::is_video_url(address);
    let mut metadata = match read_tags(address, auth, cover) {
        Ok(metadata) => metadata,
        Err(_) if is_video => Metadata::default(),
        Err(error) => return Err(error),
    };
    if cover && is_video && metadata.artwork.is_none() {
        metadata.artwork = crate::video::thumbnail(address, auth).ok();
    }
    Ok(metadata)
}

fn read_tags(
    address: &str,
    auth: Option<&(String, String)>,
    cover: bool,
) -> Result<Metadata, String> {
    let file = if address.starts_with("file:") {
        let path = url::Url::parse(address)
            .map_err(|e| e.to_string())?
            .to_file_path()
            .map_err(|_| "Invalid local file URL")?;
        Probe::open(path)
            .map_err(|e| e.to_string())?
            .options(
                ParseOptions::new()
                    .read_properties(false)
                    .read_cover_art(cover),
            )
            .read()
    } else {
        let reader = RemoteReader::new(address, auth).map_err(|e| e.to_string())?;
        Probe::new(std::io::BufReader::new(reader))
            .guess_file_type()
            .map_err(|e| e.to_string())?
            .options(
                ParseOptions::new()
                    .read_properties(false)
                    .read_cover_art(cover),
            )
            .read()
    }
    .map_err(|e| e.to_string())?;
    let tags: Vec<_> = file
        .primary_tag()
        .into_iter()
        .chain(file.tags().iter())
        .collect();
    let title = tags
        .iter()
        .find_map(|tag| tag.title())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let artist = tags
        .iter()
        .find_map(|tag| tag.artist())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let album = tags
        .iter()
        .find_map(|tag| tag.album())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let pictures: Vec<_> = tags.iter().flat_map(|tag| tag.pictures()).collect();
    let artwork = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first())
        .map(|p| p.data().to_vec());
    Ok(Metadata {
        title: title.map(|title| {
            artist.map_or_else(|| title.clone(), |artist| format!("{artist} - {title}"))
        }),
        album,
        track_number: tags.iter().find_map(|tag| tag.track()),
        disc_number: tags.iter().find_map(|tag| tag.disk()),
        artwork,
    })
}

// Seek through tags using byte ranges, without downloading whole music files.
const BLOCK: u64 = 64 * 1024;
const BUDGET: u64 = 16 * 1024 * 1024;
struct RemoteReader {
    client: reqwest::blocking::Client,
    address: String,
    auth: Option<(String, String)>,
    length: u64,
    position: u64,
    blocks: std::collections::BTreeMap<u64, Vec<u8>>,
    transferred: u64,
    started: std::time::Instant,
}
impl RemoteReader {
    fn new(address: &str, auth: Option<&(String, String)>) -> io::Result<Self> {
        let mut reader = Self {
            client: crate::network::client().map_err(io::Error::other)?,
            address: address.to_owned(),
            auth: auth.cloned(),
            length: 0,
            position: 0,
            blocks: Default::default(),
            transferred: 0,
            started: std::time::Instant::now(),
        };
        reader.fetch(0)?;
        Ok(reader)
    }
    fn fetch(&mut self, start: u64) -> io::Result<()> {
        if self.transferred >= BUDGET || self.started.elapsed().as_secs() >= 30 {
            return Err(io::Error::other("Remote metadata read limit reached"));
        }
        let mut request = self.client.get(&self.address).header(
            reqwest::header::RANGE,
            format!("bytes={start}-{}", start + BLOCK - 1),
        );
        if let Some((user, password)) = &self.auth {
            request = request.basic_auth(user, Some(password));
        }
        let response = request
            .send()
            .map_err(|_| io::Error::other("Metadata request failed"))?
            .error_for_status()
            .map_err(|_| io::Error::other("Metadata request rejected"))?;
        let partial = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if partial {
            let range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|h| h.to_str().ok())
                .and_then(|h| h.strip_prefix("bytes "))
                .and_then(|h| h.split_once('/'))
                .ok_or_else(|| io::Error::other("Invalid byte range"))?;
            let range_start = range
                .0
                .split_once('-')
                .and_then(|(s, _)| s.parse::<u64>().ok());
            let length = range.1.parse::<u64>().map_err(io::Error::other)?;
            if range_start != Some(start) || (self.length != 0 && self.length != length) {
                return Err(io::Error::other("Inconsistent byte range"));
            }
            self.length = length;
        } else if start != 0 {
            return Err(io::Error::other("Server does not support byte ranges"));
        }
        let limit = if partial { BLOCK } else { BUDGET };
        let mut bytes = Vec::new();
        response.take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit || self.transferred + bytes.len() as u64 > BUDGET {
            return Err(io::Error::other("Remote metadata exceeds read limit"));
        }
        if !partial {
            self.length = bytes.len() as u64;
        }
        self.transferred += bytes.len() as u64;
        for (i, chunk) in bytes.chunks(BLOCK as usize).enumerate() {
            self.blocks.insert(start + i as u64 * BLOCK, chunk.to_vec());
        }
        Ok(())
    }
}
impl Read for RemoteReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.length || out.is_empty() {
            return Ok(0);
        }
        let start = self.position / BLOCK * BLOCK;
        if !self.blocks.contains_key(&start) {
            self.fetch(start)?;
        }
        let block = self
            .blocks
            .get(&start)
            .ok_or_else(|| io::Error::other("Empty byte range"))?;
        let offset = (self.position - start) as usize;
        let remaining = block
            .get(offset..)
            .ok_or_else(|| io::Error::other("Truncated byte range"))?;
        let count = out.len().min(remaining.len());
        if count == 0 {
            return Err(io::Error::other("Truncated byte range"));
        }
        out[..count].copy_from_slice(&remaining[..count]);
        self.position += count as u64;
        Ok(count)
    }
}
impl Seek for RemoteReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.position as i128 + n as i128,
            SeekFrom::End(n) => self.length as i128 + n as i128,
        };
        self.position = u64::try_from(position).map_err(io::Error::other)?;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    fn tagged_fixture() -> Vec<u8> {
        use lofty::tag::{ItemKey, TagExt};
        let path =
            std::env::temp_dir().join(format!("kaleido-metadata-{}.mp3", uuid::Uuid::new_v4()));
        std::fs::write(&path, include_bytes!("../tests/fixtures/cover.mp3")).unwrap();
        let mut file = lofty::read_from_path(&path).unwrap();
        let tag = file.primary_tag_mut().unwrap();
        tag.insert_text(ItemKey::TrackTitle, "Remote Song".into());
        tag.insert_text(ItemKey::TrackArtist, "Remote Artist".into());
        tag.insert_text(ItemKey::AlbumTitle, "Remote Album".into());
        tag.set_track(10);
        tag.set_disk(2);
        tag.save_to_path(&path, Default::default()).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        bytes
    }

    fn server(
        bytes: Vec<u8>,
        ranges: bool,
        requests: usize,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/song.mp3", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    headers.push_str(&line.to_lowercase());
                }
                assert!(headers.contains("authorization: basic dxnlcjpwyxnz"));
                let start = headers
                    .split("range: bytes=")
                    .nth(1)
                    .unwrap()
                    .split('-')
                    .next()
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                let (status, range, body) = if ranges {
                    let end = (start + BLOCK as usize).min(bytes.len());
                    (
                        "206 Partial Content",
                        format!(
                            "Content-Range: bytes {}-{}/{}\r\n",
                            start,
                            end - 1,
                            bytes.len()
                        ),
                        &bytes[start..end],
                    )
                } else {
                    ("200 OK", String::new(), bytes.as_slice())
                };
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\n{range}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(body).unwrap();
            }
        });
        (address, handle)
    }

    #[test]
    fn remote_tags_and_cover_with_and_without_range_support() {
        for ranges in [true, false] {
            let (address, server) = server(tagged_fixture(), ranges, 1);
            let metadata = read(&address, Some(&("user".into(), "pass".into()))).unwrap();
            assert_eq!(
                metadata.title.as_deref(),
                Some("Remote Artist - Remote Song")
            );
            assert_eq!(metadata.album.as_deref(), Some("Remote Album"));
            assert!(image::load_from_memory(&metadata.artwork.unwrap()).is_ok());
            server.join().unwrap();
        }
    }

    #[test]
    fn list_summary_reads_tags_without_retaining_cover_art() {
        let (address, server) = server(tagged_fixture(), true, 1);
        let metadata = read_summary(&address, Some(&("user".into(), "pass".into()))).unwrap();
        assert_eq!(
            metadata.title.as_deref(),
            Some("Remote Artist - Remote Song")
        );
        assert_eq!(metadata.album.as_deref(), Some("Remote Album"));
        assert_eq!(metadata.track_number, Some(10));
        assert_eq!(metadata.disc_number, Some(2));
        assert!(metadata.artwork.is_none());
        server.join().unwrap();
    }

    #[test]
    fn remote_reader_seeks_to_end_and_reuses_cached_ranges() {
        let bytes: Vec<_> = (0..BLOCK * 3).map(|i| (i % 251) as u8).collect();
        let expected = bytes[bytes.len() - 8..].to_vec();
        let (address, server) = server(bytes, true, 2);
        let mut reader =
            RemoteReader::new(&address, Some(&("user".into(), "pass".into()))).unwrap();
        reader.seek(SeekFrom::End(-8)).unwrap();
        let mut result = [0; 8];
        reader.read_exact(&mut result).unwrap();
        assert_eq!(result.as_slice(), expected);
        reader.seek(SeekFrom::Start(0)).unwrap();
        reader.read_exact(&mut result).unwrap();
        assert_eq!(result, [0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(reader.seek(SeekFrom::Current(-9)).is_err());
        server.join().unwrap();
    }
}
