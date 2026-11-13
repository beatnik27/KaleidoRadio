use crate::{
    file_metadata,
    icons::{self, Icon},
    model::Station,
    network, theme,
};
use eframe::egui::{self, ColorImage, TextureHandle};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

pub const SIZE: f32 = 40.0;
const CACHE_LIMIT: usize = 512;

#[derive(Clone)]
pub enum Source {
    Image(String),
    File {
        address: String,
        auth: Option<(String, String)>,
    },
}
#[derive(Clone, Hash, PartialEq, Eq)]
enum Key {
    Image(String),
    File(String, String),
}
impl Source {
    pub fn station(station: &Station) -> Option<Self> {
        if crate::youtube::is_url(&station.stream_url) {
            station.favicon_url.clone().map(Self::Image)
        } else if station.is_file() || crate::video::is_video_url(&station.stream_url) {
            Some(Self::File {
                address: station.stream_url.clone(),
                auth: None,
            })
        } else {
            station
                .favicon_url
                .as_ref()
                .filter(|url| !url.trim().is_empty())
                .cloned()
                .map(Self::Image)
        }
    }
    fn key(&self) -> Key {
        match self {
            Self::Image(url) => Key::Image(url.clone()),
            Self::File { address, auth } => Key::File(
                address.clone(),
                auth.as_ref().map(|a| a.0.clone()).unwrap_or_default(),
            ),
        }
    }
    fn load(&self) -> Option<ColorImage> {
        let bytes = match self {
            Self::Image(url) => network::artwork("", Some(url)).ok()?,
            Self::File { address, auth } => {
                file_metadata::read(address, auth.as_ref()).ok()?.artwork?
            }
        };
        thumbnail(&bytes)
    }
}
fn thumbnail(bytes: &[u8]) -> Option<ColorImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .ok()?
        .resize_to_fill(96, 96, image::imageops::FilterType::Triangle)
        .to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([96, 96], image.as_raw()))
}
struct Job {
    source: Source,
    ctx: egui::Context,
}
struct Entry {
    texture: Option<TextureHandle>,
    used: u64,
    loaded: Instant,
}
pub struct Cache {
    jobs: mpsc::SyncSender<Job>,
    results: mpsc::Receiver<(Key, Option<ColorImage>)>,
    pending: HashSet<Key>,
    entries: HashMap<Key, Entry>,
    tick: u64,
}
impl Default for Cache {
    fn default() -> Self {
        let (jobs, receiver) = mpsc::sync_channel::<Job>(24);
        let receiver = Arc::new(Mutex::new(receiver));
        let (sender, results) = mpsc::channel();
        for _ in 0..2 {
            let receiver = receiver.clone();
            let sender = sender.clone();
            std::thread::spawn(move || {
                loop {
                    let job = receiver.lock().unwrap().recv();
                    let Ok(job) = job else {
                        break;
                    };
                    let key = job.source.key();
                    let image = job.source.load();
                    if sender.send((key, image)).is_err() {
                        break;
                    }
                    job.ctx.request_repaint();
                }
            });
        }
        Self {
            jobs,
            results,
            pending: Default::default(),
            entries: Default::default(),
            tick: 0,
        }
    }
}
impl Cache {
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((key, image)) = self.results.try_recv() {
            self.pending.remove(&key);
            let texture = image
                .map(|image| ctx.load_texture("card-artwork", image, egui::TextureOptions::LINEAR))
                .or_else(|| {
                    self.entries
                        .get(&key)
                        .and_then(|entry| entry.texture.clone())
                });
            self.entries.insert(
                key,
                Entry {
                    texture,
                    used: self.tick,
                    loaded: Instant::now(),
                },
            );
            if self.entries.len() > CACHE_LIMIT {
                let oldest = self
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.used)
                    .map(|(key, _)| key.clone());
                if let Some(key) = oldest {
                    self.entries.remove(&key);
                }
            }
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        source: Option<Source>,
        fallback: Icon,
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(SIZE, SIZE), egui::Sense::click());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        self.tick += 1;
        if let Some(source) = source {
            let key = source.key();
            if self.entries.get(&key).is_some_and(|entry| {
                entry.texture.is_none() && entry.loaded.elapsed() > Duration::from_secs(300)
            }) {
                self.entries.remove(&key);
            }
            let refresh = self.entries.get(&key).is_none_or(|entry| {
                matches!(&source, Source::Image(_))
                    && entry.loaded.elapsed() > Duration::from_secs(900)
            });
            if refresh
                && !self.pending.contains(&key)
                && self
                    .jobs
                    .try_send(Job {
                        source,
                        ctx: ui.ctx().clone(),
                    })
                    .is_ok()
            {
                self.pending.insert(key.clone());
            }
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.used = self.tick;
                if let Some(texture) = &entry.texture {
                    egui::Image::new(texture)
                        .fit_to_exact_size(rect.size())
                        .corner_radius(6.0)
                        .paint_at(ui, rect);
                    return response;
                }
            }
        }
        ui.painter()
            .rect_filled(rect, 6.0, theme::DEFAULT_ARTWORK_COLOR);
        icons::draw(
            ui.painter(),
            fallback,
            egui::Rect::from_center_size(rect.center(), egui::vec2(20.0, 20.0)),
            theme::MUTED,
        );
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cover_thumbnails_are_square_and_invalid_images_fall_back() {
        let file = file_metadata::read(
            url::Url::from_file_path(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cover.mp3"),
            )
            .unwrap()
            .as_str(),
            None,
        )
        .unwrap();
        assert_eq!(thumbnail(&file.artwork.unwrap()).unwrap().size, [96, 96]);
        assert!(thumbnail(b"not artwork").is_none());
    }
    #[test]
    fn webp_artwork_decodes_for_cards() {
        let mut bytes = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
            .encode(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let image = thumbnail(&bytes).unwrap();
        assert_eq!(image.size, [96, 96]);
        assert_eq!(image.pixels[0], egui::Color32::RED);
    }

    #[test]
    fn sources_use_logos_for_radio_and_embedded_covers_for_files() {
        let mut station = Station::new("Radio", "https://example.com/live");
        assert!(Source::station(&station).is_none());
        station.favicon_url = Some("https://example.com/logo.png".into());
        assert!(matches!(Source::station(&station), Some(Source::Image(_))));
        station.media_source = Some("remoteFile".into());
        assert!(matches!(
            Source::station(&station),
            Some(Source::File { .. })
        ));
        assert!(
            Source::File {
                address: station.stream_url.clone(),
                auth: Some(("a".into(), "secret".into()))
            }
            .key()
                != Source::File {
                    address: station.stream_url,
                    auth: Some(("b".into(), "secret".into()))
                }
                .key()
        );
    }
}

#[cfg(test)]
mod video_card_tests {
    use super::*;
    #[test]
    fn favorites_use_video_posters_for_direct_video_urls() {
        for address in ["https://example.com/movie.mp4", "https://example.com/live.m3u8"] {
            let station = Station::new("Video", address);
            assert!(matches!(Source::station(&station), Some(Source::File { .. })));
        }
    }
}
