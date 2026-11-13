//! Public YouTube Music catalogue browsing. Playback remains owned by yt-dlp.
use crate::model::Station;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    io::Read,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub thumbnail: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub thumbnail: Option<String>,
    pub params: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Profile {
    pub artist: Artist,
    pub albums: Vec<Album>,
    pub shuffle: Option<Value>,
    pub radio: Option<Value>,
}
#[derive(Clone, Debug)]
pub struct RadioPage {
    pub tracks: Vec<Station>,
    pub body: Value,
    pub continuation: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mix {
    Shuffle,
    Radio,
}
/// A whole artist saved as a favorite: clicking it opens the artist's
/// albums in YouTube Music instead of playing a stream.
pub fn artist_station(artist: &Artist) -> Station {
    let mut station = Station::new(artist.name.clone(), format!("https://music.youtube.com/channel/{}", artist.id));
    station.tags = if artist.subtitle.trim().is_empty() { None } else { Some(artist.subtitle.clone()) };
    station.media_source = Some("youtubeMusicArtist".into());
    station.homepage = Some(station.stream_url.clone());
    station.favicon_url = artist.thumbnail.clone();
    station
}
/// A whole album saved as a favorite: clicking it opens the album's
/// tracks in YouTube Music instead of playing a stream.
pub fn album_station(album: &Album) -> Station {
    let mut station = Station::new(album.title.clone(), format!("https://music.youtube.com/browse/{}", album.id));
    station.tags = if album.subtitle.trim().is_empty() { None } else { Some(album.subtitle.clone()) };
    station.media_source = Some("youtubeMusicAlbum".into());
    station.homepage = Some(station.stream_url.clone());
    station.favicon_url = album.thumbnail.clone();
    station.youtube_music_album_params = album.params.clone();
    station
}
pub fn artist_for_station(station: &Station) -> Option<Artist> {
    if !station.is_music_artist() {
        return None;
    }
    let id = station.stream_url.strip_prefix("https://music.youtube.com/channel/")?.split(['/', '?']).next()?.to_owned();
    if id.is_empty() {
        return None;
    }
    Some(Artist { id, name: station.name.clone(), subtitle: station.tags.clone().unwrap_or_default(), thumbnail: station.favicon_url.clone() })
}
pub fn album_for_station(station: &Station) -> Option<Album> {
    if !station.is_music_album() {
        return None;
    }
    let id = station.stream_url.strip_prefix("https://music.youtube.com/browse/")?.split(['/', '?']).next()?.to_owned();
    if id.is_empty() {
        return None;
    }
    Some(Album { id, title: station.name.clone(), subtitle: station.tags.clone().unwrap_or_default(), thumbnail: station.favicon_url.clone(), params: station.youtube_music_album_params.clone() })
}
pub enum Request {
    Search(String, u32),
    Artist(Artist),
    Album(Album),
    Mix(Value, Mix),
    More(RadioPage),
}
pub enum ResultPage {
    Artists(Vec<Artist>),
    Artist(Profile),
    Album(Album, Vec<Station>),
    Mix(RadioPage, Mix),
    More(RadioPage),
}

pub fn execute(request: Request, cancel: &AtomicBool) -> Result<ResultPage, String> {
    let client = Client::new()?;
    match request {
        Request::Search(query, limit) => client
            .artists(&query, limit, cancel)
            .map(ResultPage::Artists),
        Request::Artist(artist) => client.artist(artist, cancel).map(ResultPage::Artist),
        Request::Album(album) => client
            .album(&album, cancel)
            .map(|tracks| ResultPage::Album(album, tracks)),
        Request::Mix(endpoint, mix) => client
            .mix(&endpoint, None, cancel)
            .map(|page| ResultPage::Mix(page, mix)),
        Request::More(page) => client
            .mix(&page.body, page.continuation.as_deref(), cancel)
            .map(ResultPage::More),
    }
}

struct Client {
    http: reqwest::blocking::Client,
    version: String,
}
impl Client {
    fn new() -> Result<Self, String> {
        static VERSION: OnceLock<String> = OnceLock::new();
        let http = reqwest::blocking::Client::builder()
            .user_agent("Mozilla/5.0")
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?;
        let version = VERSION
            .get_or_init(|| {
                let mut page = String::new();
                if let Ok(response) = http.get("https://music.youtube.com/").send() {
                    let _ = response.take(4 * 1024 * 1024).read_to_string(&mut page);
                }
                page.split("\"INNERTUBE_CLIENT_VERSION\"")
                    .nth(1)
                    .and_then(|tail| tail.split(':').nth(1))
                    .map(str::trim_start)
                    .and_then(|tail| tail.strip_prefix('"'))
                    .and_then(|tail| tail.split('"').next())
                    .filter(|version| !version.is_empty())
                    .unwrap_or("1.20261020.01.00")
                    .to_owned()
            })
            .clone();
        Ok(Self { http, version })
    }
    fn request(
        &self,
        endpoint: &str,
        mut body: Value,
        continuation: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        check_cancel(cancel)?;
        body["context"] = json!({"client":{"clientName":"WEB_REMIX","clientVersion":self.version,"hl":"en","gl":"US"}});
        let mut request = self
            .http
            .post(format!("https://music.youtube.com/youtubei/v1/{endpoint}"))
            .query(&[("prettyPrint", "false")])
            .header("Origin", "https://music.youtube.com")
            .header("Referer", "https://music.youtube.com/")
            .json(&body);
        if let Some(token) = continuation {
            request = request.query(&[("continuation", token), ("ctoken", token)]);
        }
        let response = request
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|e| format!("YouTube Music request failed: {e}"))?;
        let response: Value = serde_json::from_reader(response.take(16 * 1024 * 1024))
            .map_err(|e| format!("Invalid YouTube Music response: {e}"))?;
        check_cancel(cancel)?;
        if let Some(error) = response.pointer("/error/message").and_then(Value::as_str) {
            return Err(error.into());
        }
        Ok(response)
    }
    fn artists(&self, query: &str, limit: u32, cancel: &AtomicBool) -> Result<Vec<Artist>, String> {
        let body = json!({"query":query,"params":"EgWKAQIgAWoKEAoQAxAEEAkQBQ=="});
        let mut results = Vec::new();
        let mut seen = HashSet::new();
        let mut tokens = HashSet::new();
        let mut token = None;
        loop {
            let response = self.request("search", body.clone(), token.as_deref(), cancel)?;
            for row in named(&response, "musicResponsiveListItemRenderer") {
                if let Some(artist) = parse_artist(row) {
                    if seen.insert(artist.id.clone()) {
                        results.push(artist);
                    }
                }
                if limit > 0 && results.len() >= limit as usize {
                    results.truncate(limit as usize);
                    return Ok(results);
                }
            }
            token = continuation(&response);
            if !token
                .as_ref()
                .is_some_and(|token| tokens.insert(token.clone()))
            {
                break;
            }
        }
        Ok(results)
    }
    fn artist(&self, artist: Artist, cancel: &AtomicBool) -> Result<Profile, String> {
        let response = self.request("browse", json!({"browseId":artist.id}), None, cancel)?;
        let header = named(&response, "musicImmersiveHeaderRenderer")
            .into_iter()
            .chain(named(&response, "musicVisualHeaderRenderer"))
            .chain(named(&response, "musicResponsiveHeaderRenderer"))
            .next();
        let shuffle = header
            .and_then(|header| header.pointer("/playButton/buttonRenderer/navigationEndpoint"))
            .and_then(watch_endpoint)
            .map(|mut endpoint| {
                endpoint["_artistId"] = Value::String(artist.id.clone());
                endpoint
            });
        let radio = header
            .and_then(|header| {
                header.pointer("/startRadioButton/buttonRenderer/navigationEndpoint")
            })
            .and_then(watch_endpoint);
        let mut albums = Vec::new();
        let mut seen = HashSet::new();
        for shelf in named(&response, "musicCarouselShelfRenderer") {
            let title = shelf
                .pointer("/header/musicCarouselShelfBasicHeaderRenderer/title")
                .map(text)
                .unwrap_or_default();
            if !matches!(title.as_str(), "Albums" | "Singles & EPs") {
                continue;
            }
            let mut pages = vec![shelf.clone()];
            let endpoint = shelf.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title/runs/0/navigationEndpoint/browseEndpoint")
                .or_else(|| shelf.pointer("/header/musicCarouselShelfBasicHeaderRenderer/moreContentButton/buttonRenderer/navigationEndpoint/browseEndpoint"));
            if let Some(endpoint) = endpoint {
                let mut token = None;
                let mut tokens = HashSet::new();
                loop {
                    let page =
                        self.request("browse", browse_body(endpoint)?, token.as_deref(), cancel)?;
                    token = continuation(&page);
                    pages.push(page);
                    if !token
                        .as_ref()
                        .is_some_and(|token| tokens.insert(token.clone()))
                    {
                        break;
                    }
                }
            }
            for page in pages {
                for row in named(&page, "musicTwoRowItemRenderer") {
                    if let Some(mut album) = parse_album(row) {
                        if seen.insert(album.id.clone()) {
                            album.subtitle = format!(
                                "{} · {}",
                                if title == "Albums" {
                                    "Album"
                                } else {
                                    "Single / EP"
                                },
                                album.subtitle
                            )
                            .trim_end_matches(" · ")
                            .to_owned();
                            albums.push(album);
                        }
                    }
                }
            }
        }
        Ok(Profile {
            artist,
            albums,
            shuffle,
            radio,
        })
    }
    fn album(&self, album: &Album, cancel: &AtomicBool) -> Result<Vec<Station>, String> {
        let body = json!({"browseId":album.id,"params":album.params});
        let mut tracks = Vec::new();
        let mut album_artist = None;
        let mut ids = HashSet::new();
        let mut token = None;
        let mut tokens = HashSet::new();
        loop {
            let response = self.request("browse", body.clone(), token.as_deref(), cancel)?;
            if album_artist.is_none() {
                album_artist = named(&response, "musicResponsiveHeaderRenderer")
                    .into_iter()
                    .chain(named(&response, "musicDetailHeaderRenderer"))
                    .find_map(|header| {
                        header
                            .get("straplineTextOne")
                            .and_then(artist_runs)
                            .or_else(|| header.get("subtitle").and_then(artist_runs))
                    });
            }
            for row in named(&response, "musicResponsiveListItemRenderer") {
                if let Some(mut track) = parse_track(row) {
                    if ids.insert(track.stream_url.clone()) {
                        if track.artist.is_none() {
                            track.artist = album_artist.clone();
                        }
                        track.track_number = Some((tracks.len() + 1) as u32);
                        if track.favicon_url.is_none() {
                            track.favicon_url = album.thumbnail.clone();
                        }
                        tracks.push(track);
                    }
                }
            }
            token = continuation(&response);
            if !token
                .as_ref()
                .is_some_and(|token| tokens.insert(token.clone()))
            {
                break;
            }
        }
        if tracks.is_empty() {
            return Err("This album did not return any playable songs.".into());
        }
        Ok(tracks)
    }
    fn mix(
        &self,
        endpoint: &Value,
        token: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<RadioPage, String> {
        let mut body = json!({"enablePersistentPlaylistPanel":true,"isAudioOnly":true,"tunerSettingValue":"AUTOMIX_SETTING_NORMAL"});
        for key in ["videoId", "playlistId", "params", "playerParams"] {
            if let Some(value) = endpoint.get(key) {
                body[key] = value.clone();
            }
        }
        if body.get("playlistId").is_none() {
            return Err("This artist does not expose a shuffle or radio queue.".into());
        }
        let response = self.request("next", body.clone(), token, cancel)?;
        let mut tracks = Vec::new();
        let mut seen = HashSet::new();
        for row in named(&response, "playlistPanelVideoRenderer") {
            if let Some(artist_id) = endpoint.get("_artistId").and_then(Value::as_str) {
                if !track_has_artist(row, artist_id) {
                    continue;
                }
            }
            if let Some(track) = parse_track(row) {
                if seen.insert(track.stream_url.clone()) {
                    tracks.push(track);
                }
            }
        }
        if tracks.is_empty() {
            return Err("YouTube Music did not return any songs for this mix.".into());
        }
        Ok(RadioPage {
            tracks,
            body,
            continuation: continuation(&response),
        })
    }
}
/// Preserve the current queue and skip overlapping entries from radio pages.
pub fn fresh_tracks(queue: &[Station], incoming: &[Station]) -> Vec<Station> {
    let mut seen: HashSet<_> = queue.iter().map(|track| track.stream_url.clone()).collect();
    incoming
        .iter()
        .filter(|track| seen.insert(track.stream_url.clone()))
        .cloned()
        .collect()
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Cancelled.".into())
    } else {
        Ok(())
    }
}
fn named<'a>(value: &'a Value, key: &str) -> Vec<&'a Value> {
    let mut values = Vec::new();
    match value {
        Value::Object(object) => {
            if let Some(value) = object.get(key) {
                values.push(value);
            }
            for value in object.values() {
                values.extend(named(value, key));
            }
        }
        Value::Array(array) => {
            for value in array {
                values.extend(named(value, key));
            }
        }
        _ => {}
    }
    values
}
fn text(value: &Value) -> String {
    value
        .get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect()
        })
        .or_else(|| {
            value
                .get("simpleText")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}
fn thumbnail(value: &Value) -> Option<String> {
    named(value, "thumbnails").into_iter().find_map(|array| {
        array
            .as_array()?
            .iter()
            .rev()
            .find_map(|value| value.get("url")?.as_str().map(str::to_owned))
    })
}
fn watch_endpoint(value: &Value) -> Option<Value> {
    value
        .get("watchEndpoint")
        .or_else(|| value.get("watchPlaylistEndpoint"))
        .cloned()
}
fn browse_body(endpoint: &Value) -> Result<Value, String> {
    let id = endpoint
        .get("browseId")
        .and_then(Value::as_str)
        .ok_or("Missing catalogue identifier")?;
    let mut body = json!({"browseId":id});
    if let Some(params) = endpoint.get("params") {
        body["params"] = params.clone();
    }
    Ok(body)
}
fn parse_artist(row: &Value) -> Option<Artist> {
    let id = row
        .pointer("/navigationEndpoint/browseEndpoint/browseId")?
        .as_str()?;
    if !id.starts_with("UC") {
        return None;
    }
    let name = text(row.pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text")?);
    if name.is_empty() {
        return None;
    }
    Some(Artist {
        id: id.into(),
        name,
        subtitle: row
            .pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text")
            .map(text)
            .unwrap_or_default(),
        thumbnail: thumbnail(row),
    })
}
fn parse_album(row: &Value) -> Option<Album> {
    let endpoint = row
        .pointer("/navigationEndpoint/browseEndpoint")
        .or_else(|| row.pointer("/title/runs/0/navigationEndpoint/browseEndpoint"))?;
    let id = endpoint.get("browseId")?.as_str()?;
    if !id.starts_with("MPRE") {
        return None;
    }
    Some(Album {
        id: id.into(),
        title: text(row.get("title")?),
        subtitle: row.get("subtitle").map(text).unwrap_or_default(),
        thumbnail: thumbnail(row),
        params: endpoint
            .get("params")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}
fn track_has_artist(row: &Value, artist_id: &str) -> bool {
    let byline = row
        .get("longBylineText")
        .or_else(|| row.pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text"));
    byline.is_some_and(|byline| {
        named(byline, "browseEndpoint")
            .into_iter()
            .any(|endpoint| endpoint.get("browseId").and_then(Value::as_str) == Some(artist_id))
    })
}
fn artist_runs(value: &Value) -> Option<String> {
    let mut names = Vec::new();
    for run in value.get("runs")?.as_array()? {
        if run
            .pointer("/navigationEndpoint/browseEndpoint/browseId")
            .and_then(Value::as_str)
            .is_some_and(|id| id.starts_with("UC"))
        {
            if let Some(name) = run
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    (!names.is_empty()).then(|| names.join(", "))
}
fn parse_track(row: &Value) -> Option<Station> {
    let id = row
        .get("videoId")
        .or_else(|| row.pointer("/playlistItemData/videoId"))?
        .as_str()?;
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return None;
    }
    let title = row
        .get("title")
        .or_else(|| row.pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text"))
        .map(text)?;
    if title.is_empty() {
        return None;
    }
    let mut track = Station::new(title, format!("https://music.youtube.com/watch?v={id}"));
    track.media_source = Some("youtubeMusic".into());
    track.homepage = Some(track.stream_url.clone());
    track.favicon_url = thumbnail(row);
    let byline = row
        .get("longBylineText")
        .or_else(|| row.pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text"))
        .map(text)
        .unwrap_or_default();
    let byline_value = row
        .get("longBylineText")
        .or_else(|| row.pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text"));
    track.artist = byline_value.and_then(artist_runs);
    track.tags = Some(byline);
    Some(track)
}
fn continuation(value: &Value) -> Option<String> {
    for key in [
        "nextRadioContinuationData",
        "nextContinuationData",
        "reloadContinuationData",
    ] {
        if let Some(token) = named(value, key)
            .into_iter()
            .find_map(|data| data.get("continuation")?.as_str().map(str::to_owned))
        {
            return Some(token);
        }
    }
    named(value, "continuationCommand")
        .into_iter()
        .find_map(|data| data.get("token")?.as_str().map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artist_and_album_stations_round_trip_to_favorites() {
        let artist = Artist { id: "UCartist".into(), name: "Artist".into(), subtitle: "Artist".into(), thumbnail: Some("https://example.org/a.jpg".into()) };
        let station = artist_station(&artist);
        assert!(station.is_music_artist());
        assert_eq!(station.favorite_category(), crate::model::FavoriteCategory::YouTubeMusic);
        let back = artist_for_station(&station).unwrap();
        assert_eq!(back.id, "UCartist");
        assert_eq!(back.name, "Artist");
        let album = Album { id: "MPREalbum".into(), title: "Album".into(), subtitle: "Album · 2020".into(), thumbnail: None, params: Some("params".into()) };
        let station = album_station(&album);
        assert!(station.is_music_album());
        assert_eq!(station.favorite_category(), crate::model::FavoriteCategory::YouTubeMusic);
        let back = album_for_station(&station).unwrap();
        assert_eq!(back.id, "MPREalbum");
        assert_eq!(back.params.as_deref(), Some("params"));
        assert!(artist_for_station(&Station::new("Song", "https://music.youtube.com/watch?v=abc")).is_none());
    }
    #[test]
    fn artist_metadata_excludes_album_and_year_from_byline() {
        let row = json!({"videoId":"abc", "title":{"runs":[{"text":"Song - Live"}]}, "longBylineText":{"runs":[
            {"text":"Artist", "navigationEndpoint":{"browseEndpoint":{"browseId":"UCartist"}}},
            {"text":" • "}, {"text":"Album", "navigationEndpoint":{"browseEndpoint":{"browseId":"MPREbalbum"}}},
            {"text":" • 2026"}
        ]}});
        let track = parse_track(&row).unwrap();
        assert_eq!(track.artist.as_deref(), Some("Artist"));
        assert_eq!(track.name, "Song - Live");
    }
    #[test]
    fn radio_continuations_preserve_order_and_skip_duplicates() {
        let existing = Station::new("First", "https://music.youtube.com/watch?v=first");
        let repeated = Station::new("First again", existing.stream_url.clone());
        let next = Station::new("Second", "https://music.youtube.com/watch?v=second");
        let third = Station::new("Third", "https://music.youtube.com/watch?v=third");
        let added = fresh_tracks(
            &[existing],
            &[repeated, next.clone(), next.clone(), third.clone()],
        );
        assert_eq!(
            added.iter().map(|track| track.id).collect::<Vec<_>>(),
            vec![next.id, third.id]
        );
    }
    #[test]
    fn shuffle_filters_by_artist_identity_while_radio_can_include_others() {
        let track = json!({"longBylineText":{"runs":[{"text":"Name","navigationEndpoint":{"browseEndpoint":{"browseId":"UCartist"}}}]}});
        assert!(track_has_artist(&track, "UCartist"));
        assert!(!track_has_artist(&track, "UCother"));
    }
    #[test]
    fn parser_keeps_album_order_and_music_links() {
        let row = json!({"playlistItemData":{"videoId":"abc_123"},"flexColumns":[{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"Song"}]}}}]});
        let track = parse_track(&row).unwrap();
        assert_eq!(
            track.stream_url,
            "https://music.youtube.com/watch?v=abc_123"
        );
        assert!(track.is_file());
        assert!(parse_album(&json!({"title":{"runs":[{"text":"Artist"}]},"navigationEndpoint":{"browseEndpoint":{"browseId":"UCnotAlbum"}}})).is_none());
    }
    #[test]
    #[ignore = "requires internet access to YouTube Music"]
    fn live_artist_albums_songs_shuffle_and_radio() {
        let cancel = AtomicBool::new(false);
        let client = Client::new().unwrap();
        let artists = client.artists("Luttrell", 3, &cancel).unwrap();
        assert_eq!(artists[0].name, "Luttrell");
        let profile = client.artist(artists[0].clone(), &cancel).unwrap();
        assert!(
            profile
                .albums
                .iter()
                .any(|album| album.title == "Life at Full Speed")
        );
        let album = profile
            .albums
            .iter()
            .find(|album| album.title == "Life at Full Speed")
            .unwrap();
        let tracks = client.album(album, &cancel).unwrap();
        assert!(tracks.len() > 5);
        assert!(tracks.iter().all(|track| {
            track
                .artist
                .as_deref()
                .is_some_and(|artist| artist.contains("Luttrell"))
        }));
        assert_eq!(tracks[0].track_number, Some(1));
        let shuffle = client
            .mix(profile.shuffle.as_ref().unwrap(), None, &cancel)
            .unwrap();
        assert!(shuffle.tracks.len() > 5);
        let radio = client
            .mix(profile.radio.as_ref().unwrap(), None, &cancel)
            .unwrap();
        assert!(radio.tracks.len() > 5);
        let more = client
            .mix(&radio.body, radio.continuation.as_deref(), &cancel)
            .unwrap();
        assert!(!more.tracks.is_empty());
        eprintln!(
            "{} releases, {} album songs, {} shuffle songs, {} radio songs + {} continuation songs",
            profile.albums.len(),
            tracks.len(),
            shuffle.tracks.len(),
            radio.tracks.len(),
            more.tracks.len()
        );
    }
}
