//! macOS SMB shares, mounted by NetFS and read through the filesystem.
//! Persistent entries keep their smb:// URLs; mount paths are session-local.
use crate::{
    model::{RemoteConnection, is_media},
    network::RemoteEntry,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use url::Url;

#[derive(Debug)]
struct Target {
    share: Url,
    relative: PathBuf,
    share_name: String,
}
impl Target {
    fn parse(address: &str) -> Result<Self, String> {
        let mut url =
            Url::parse(address).map_err(|_| "Enter an SMB URL such as smb://server/share.")?;
        if url.scheme() != "smb"
            || url.host_str().is_none_or(str::is_empty)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "Use smb://server/share and enter credentials in the Username and Password fields."
                    .into(),
            );
        }
        let host = url.host_str().unwrap().to_ascii_lowercase();
        url.set_host(Some(&host))
            .map_err(|_| "Invalid SMB server")?;
        let mut parts = Vec::new();
        for part in url.path_segments().ok_or("Invalid SMB share path")? {
            if part.is_empty() {
                continue;
            }
            let part = percent_encoding::percent_decode_str(part)
                .decode_utf8()
                .map_err(|_| "Invalid SMB filename encoding")?
                .into_owned();
            if matches!(part.as_str(), "." | "..") || part.contains(['/', '\\', '\0']) {
                return Err("Invalid SMB path component.".into());
            }
            parts.push(part);
        }
        let share_name = parts
            .first()
            .cloned()
            .ok_or("Include a share name, for example smb://server/Music.")?;
        let relative = parts.iter().skip(1).collect::<PathBuf>();
        url.path_segments_mut()
            .map_err(|_| "Invalid SMB URL")?
            .clear()
            .push(&share_name)
            .push("");
        Ok(Self {
            share: url,
            relative,
            share_name,
        })
    }
}

pub fn valid_url(address: &str) -> bool {
    Target::parse(address).is_ok()
}
pub fn same_share(left: &str, right: &str) -> bool {
    match (Target::parse(left), Target::parse(right)) {
        (Ok(a), Ok(b)) => {
            a.share.host_str() == b.share.host_str()
                && a.share.port().unwrap_or(445) == b.share.port().unwrap_or(445)
                && a.share_name.eq_ignore_ascii_case(&b.share_name)
        }
        _ => false,
    }
}
pub fn parent(address: &str) -> Option<String> {
    let target = Target::parse(address).ok()?;
    if target.relative.as_os_str().is_empty() {
        None
    } else {
        Url::parse(address).ok()?.join("../").ok().map(Into::into)
    }
}

type MountKey = (String, String);
static MOUNTS: OnceLock<Mutex<HashMap<MountKey, PathBuf>>> = OnceLock::new();

#[cfg(target_os = "macos")]
fn mount(share: &Url, auth: Option<&(String, String)>) -> Result<PathBuf, String> {
    use std::ffi::{CString, c_char};
    unsafe extern "C" {
        fn KRSMBMount(
            address: *const c_char,
            username: *const c_char,
            password: *const c_char,
            path: *mut c_char,
            path_size: usize,
            error: *mut c_char,
            error_size: usize,
        ) -> i32;
    }
    let address = CString::new(share.as_str()).map_err(|_| "Invalid SMB address")?;
    let user =
        CString::new(auth.map_or("", |a| a.0.as_str())).map_err(|_| "Invalid SMB username")?;
    let password =
        CString::new(auth.map_or("", |a| a.1.as_str())).map_err(|_| "Invalid SMB password")?;
    let mut path = [0u8; 4096];
    let mut error = [0u8; 1024];
    // SAFETY: all strings and output buffers live for the synchronous native call.
    let status = unsafe {
        KRSMBMount(
            address.as_ptr(),
            user.as_ptr(),
            password.as_ptr(),
            path.as_mut_ptr().cast(),
            path.len(),
            error.as_mut_ptr().cast(),
            error.len(),
        )
    };
    let decode = |bytes: &[u8]| {
        String::from_utf8_lossy(&bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())])
            .into_owned()
    };
    if status == 0 {
        Ok(PathBuf::from(decode(&path)))
    } else {
        Err(decode(&error))
    }
}
#[cfg(not(target_os = "macos"))]
fn mount(_: &Url, _: Option<&(String, String)>) -> Result<PathBuf, String> {
    Err("SMB support currently requires macOS.".into())
}

fn mounted_target(
    address: &str,
    auth: Option<&(String, String)>,
) -> Result<(PathBuf, PathBuf), String> {
    let target = Target::parse(address)?;
    let key = (
        target.share.to_string(),
        auth.map_or("", |a| a.0.as_str()).to_owned(),
    );
    // Serialize mounts so browsing, thumbnails, and playback reuse one OS session.
    let mut mounts = MOUNTS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "SMB connection state unavailable")?;
    let root = if let Some(path) = mounts.get(&key).filter(|path| path.is_dir()) {
        path.clone()
    } else {
        let path = mount(&target.share, auth)?;
        mounts.insert(key, path.clone());
        path
    };
    drop(mounts);
    let root = root
        .canonicalize()
        .map_err(|_| "SMB share is unavailable. Reconnect to the server.")?;
    let path = contained_path(&root, &target.relative)?;
    Ok((root, path))
}
fn contained_path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|_| "SMB folder or file is unavailable. Check the share permissions.")?;
    if !path.starts_with(root) {
        return Err("SMB path is outside the connected share.".into());
    }
    Ok(path)
}

pub fn resolve(address: &str, auth: Option<&(String, String)>) -> Result<String, String> {
    let (_, path) = mounted_target(address, auth)?;
    Url::from_file_path(path)
        .map(String::from)
        .map_err(|_| "Invalid SMB file path".into())
}

pub fn browse(
    connection: &RemoteConnection,
    password: &str,
    address: &str,
) -> Result<Vec<RemoteEntry>, String> {
    if !same_share(&connection.url, address) {
        return Err("Directory must be on the configured SMB share.".into());
    }
    let auth = (connection.username.clone(), password.to_owned());
    let (root, path) = mounted_target(address, Some(&auth))?;
    list_directory(&root, &path, address)
}
fn list_directory(root: &Path, path: &Path, address: &str) -> Result<Vec<RemoteEntry>, String> {
    let mut base = Url::parse(address).map_err(|_| "Invalid SMB directory")?;
    if !base.path().ends_with('/') {
        base.path_segments_mut()
            .map_err(|_| "Invalid SMB directory")?
            .push("");
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)
        .map_err(|_| "Could not read SMB folder. Check share permissions.")?
        .flatten()
    {
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|name| !name.starts_with('.')) else {
            continue;
        };
        let Ok(real_path) = entry.path().canonicalize() else {
            continue;
        };
        if !real_path.starts_with(root) {
            continue;
        }
        let Ok(metadata) = std::fs::metadata(&real_path) else {
            continue;
        };
        let directory = metadata.is_dir();
        if !directory && !(metadata.is_file() && is_media(Path::new(name))) {
            continue;
        }
        let mut url = base.clone();
        {
            let mut parts = url
                .path_segments_mut()
                .map_err(|_| "Invalid SMB directory")?;
            parts.pop_if_empty().push(name);
            if directory {
                parts.push("");
            }
        }
        entries.push(RemoteEntry {
            name: name.to_owned(),
            url: url.to_string(),
            directory,
            modified_at: metadata
                .modified()
                .ok()
                .and_then(|date| date.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|date| date.as_secs() as i64),
        });
    }
    entries.sort_by_key(|entry| (!entry.directory, entry.name.to_lowercase()));
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn validates_share_urls_and_decodes_unicode_paths() {
        let target =
            Target::parse("smb://SERVER/Sandisk/PLEX/MUSIC%20VIDEOS/Bj%C3%B6rk%20%23one.mp4")
                .unwrap();
        assert_eq!(target.share.as_str(), "smb://server/Sandisk/");
        assert_eq!(
            target.relative,
            Path::new("PLEX/MUSIC VIDEOS/Björk #one.mp4")
        );
        for bad in [
            "smb://server",
            "smb://server/",
            "smb://user:secret@server/share",
            "smb://server/share?password=secret",
            "smb://server/share#fragment",
            "smb://server/share/escape%2Foutside",
            "smb://server/share/escape%5Coutside",
            "smb://server/share/a%00b",
            "smb://server/share/%FF",
            "http://server/share",
        ] {
            assert!(!valid_url(bad), "{bad}");
        }
    }
    #[test]
    fn navigation_stays_within_the_connected_share() {
        assert!(same_share(
            "smb://SERVER/Share",
            "smb://server:445/share/Music/"
        ));
        for other in [
            "smb://other/Share/a",
            "smb://server/Other/a",
            "smb://server:1445/Share/a",
            "smb://server/Share/../Other",
        ] {
            assert!(!same_share("smb://server/Share", other), "{other}");
        }
        assert!(parent("smb://server/Share/").is_none());
        assert_eq!(
            parent("smb://server/Share/A%20B/").as_deref(),
            Some("smb://server/Share/")
        );
    }
    #[test]
    fn mounted_directory_lists_media_with_stable_encoded_smb_urls() {
        let root = std::env::temp_dir().join(format!("kaleido-smb-{}", Uuid::new_v4()));
        let outside = std::env::temp_dir().join(format!("kaleido-outside-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Album")).unwrap();
        for name in [
            "A # song.mp3",
            "Björk video.mp4",
            "notes.txt",
            ".hidden.mp3",
        ] {
            std::fs::write(root.join(name), []).unwrap();
        }
        std::fs::write(&outside, []).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("outside.mp3")).unwrap();
        let canonical = root.canonicalize().unwrap();
        let entries = list_directory(&canonical, &canonical, "smb://server/Share/").unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries[0].directory);
        assert_eq!(entries[0].url, "smb://server/Share/Album/");
        assert!(
            entries
                .iter()
                .any(|entry| entry.url == "smb://server/Share/A%20%23%20song.mp3")
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.url == "smb://server/Share/Bj%C3%B6rk%20video.mp4")
        );
        for entry in entries {
            assert!(crate::model::Station::new(&entry.name, &entry.url).is_file());
        }
        #[cfg(unix)]
        assert!(contained_path(&canonical, Path::new("outside.mp3")).is_err());
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_file(outside).unwrap();
    }
    #[test]
    fn remote_validation_keeps_webdav_and_accepts_smb() {
        for good in [
            "https://example.com/webdav",
            "http://server:2027/",
            "smb://server/Share",
            "SMB://server/Share/A%20B",
        ] {
            assert!(crate::network::valid_remote_url(good), "{good}");
        }
        for bad in [
            "ftp://server/Share",
            "smb://server",
            "smb://user:secret@server/Share",
            "https://user:secret@example.com/",
        ] {
            assert!(!crate::network::valid_remote_url(bad), "{bad}");
        }
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    #[test]
    #[ignore = "requires KALEIDO_SMB_TEST_URL and an existing authorized SMB mount"]
    fn live_smb_browse_and_resolve() {
        let address = std::env::var("KALEIDO_SMB_TEST_URL").expect("Set the SMB test share URL");
        let username = std::env::var("KALEIDO_SMB_TEST_USER").unwrap_or_default();
        let connection = RemoteConnection { name: "Live SMB test".into(), url: address.clone(), username: username.clone() };
        let entries = browse(&connection, "", &address).unwrap();
        assert!(!entries.is_empty());
        let mut pending = vec![address.clone()];
        let mut visited = std::collections::HashSet::new();
        let mut media = None;
        while let Some(directory) = pending.pop() {
            if visited.len() >= 32 || !visited.insert(directory.clone()) { continue; }
            let items = browse(&connection, "", &directory).unwrap();
            if let Some(file) = items.iter().find(|entry| !entry.directory) {
                media = Some(file.url.clone());
                break;
            }
            pending.extend(items.into_iter().filter(|entry| entry.directory).map(|entry| entry.url));
        }
        let media = media.expect("Test share needs a supported media file within 32 folders");
        let local = resolve(&media, Some(&(username, String::new()))).unwrap();
        let url = Url::parse(&local).unwrap();
        assert_eq!(url.scheme(), "file");
        assert!(url.to_file_path().unwrap().is_file());
        println!("Live SMB browse passed: {} entries; media file resolves through the mounted share.", entries.len());
    }
}
