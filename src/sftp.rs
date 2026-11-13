use crate::{model::{RemoteConnection, is_media}, network::RemoteEntry};
use std::{collections::HashMap, net::{TcpStream, ToSocketAddrs}, path::PathBuf, sync::{Mutex, OnceLock}, time::Duration};
use url::Url;

fn target(address: &str) -> Result<(Url, PathBuf), String> {
    let url = Url::parse(address).map_err(|error| error.to_string())?;
    if url.scheme() != "sftp" || url.host_str().is_none() || !url.username().is_empty()
        || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("Use sftp://server/path with separate username and password fields.".into());
    }
    let path = percent_encoding::percent_decode_str(url.path()).decode_utf8().map_err(|error| error.to_string())?;
    if path.contains('\0') { return Err("Invalid SFTP path.".into()); }
    let path = PathBuf::from(if path.is_empty() { "/" } else { &path });
    Ok((url, path))
}

pub fn valid_url(address: &str) -> bool { target(address).is_ok() }

pub fn same_server(left: &str, right: &str) -> bool {
    match (target(left), target(right)) {
        (Ok((left, _)), Ok((right, _))) => left.host_str() == right.host_str()
            && left.port().unwrap_or(22) == right.port().unwrap_or(22),
        _ => false,
    }
}

fn prefer_known_host_keys(session: &ssh2::Session, host: &str, port: u16, path: &std::path::Path) -> Result<(), ssh2::Error> {
    let lookup = if port == 22 { host.to_owned() } else { format!("[{host}]:{port}") };
    let output = std::process::Command::new("ssh-keygen").arg("-F").arg(lookup).arg("-f").arg(path).output();
    let Ok(output) = output else { return Ok(()); };
    if !output.status.success() { return Ok(()); }
    let text = String::from_utf8_lossy(&output.stdout);
    let keys: Vec<_> = text.lines().filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_whitespace().nth(1)).collect();
    let mut algorithms = session.supported_algs(ssh2::MethodType::HostKey)?;
    algorithms.sort_by_key(|algorithm| !keys.iter().any(|key| {
        *key == *algorithm || (*key == "ssh-rsa" && algorithm.starts_with("rsa-sha2-"))
    }));
    session.method_pref(ssh2::MethodType::HostKey, &algorithms.join(","))
}

fn connect(url: &Url, auth: &(String, String)) -> Result<ssh2::Sftp, String> {
    let result = (|| -> Result<ssh2::Sftp, Box<dyn std::error::Error>> {
        let host = url.host_str().ok_or("Missing SFTP host")?;
        let port = url.port().unwrap_or(22);
        let addresses = (host, port).to_socket_addrs()?;
        let mut socket = None;
        for address in addresses {
            if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(20)) {
                socket = Some(stream);
                break;
            }
        }
        let mut session = ssh2::Session::new()?;
        session.set_tcp_stream(socket.ok_or("Could not reach SFTP server")?);
        session.set_timeout(20_000);
        let known_hosts = PathBuf::from(std::env::var_os("HOME").ok_or("Missing home directory")?).join(".ssh/known_hosts");
        prefer_known_host_keys(&session, host, port, &known_hosts)?;
        session.handshake()?;
        let mut hosts = session.known_hosts()?;
        hosts.read_file(&known_hosts, ssh2::KnownHostFileKind::OpenSSH)
            .map_err(|_| "Verify this server with ssh first to add it to ~/.ssh/known_hosts")?;
        let (key, kind) = session.host_key().ok_or("Server did not provide a host key")?;
        match hosts.check_port(host, port, key) {
            ssh2::CheckResult::Match => {},
            ssh2::CheckResult::NotFound => return Err(format!("No trusted host key for {host}:{port} ({kind:?}) in {}. Verify this exact address with ssh first.", known_hosts.display()).into()),
            ssh2::CheckResult::Mismatch => return Err(format!("The {kind:?} host key for {host}:{port} does not match {}. Verify the server identity before updating the saved key.", known_hosts.display()).into()),
            ssh2::CheckResult::Failure => return Err(format!("Could not verify the host key for {host}:{port} against {}.", known_hosts.display()).into()),
        }
        if auth.1.is_empty() { session.userauth_agent(&auth.0)?; }
        else { session.userauth_password(&auth.0, &auth.1)?; }
        Ok(session.sftp()?)
    })();
    result.map_err(|error| format!("SFTP: {error}"))
}

pub fn browse(connection: &RemoteConnection, password: &str, address: &str) -> Result<Vec<RemoteEntry>, String> {
    if !same_server(&connection.url, address) { return Err("Directory must be on the configured SFTP server.".into()); }
    let (base, path) = target(address)?;
    let sftp = connect(&base, &(connection.username.clone(), password.into()))?;
    let mut entries = Vec::new();
    for (path, stat) in sftp.readdir(&path).map_err(|error| error.to_string())? {
        if !stat.is_dir() && !is_media(&path) { continue; }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue; };
        let mut url = base.clone();
        url.set_path(&path.to_string_lossy());
        if stat.is_dir() { url.set_path(&format!("{}/", path.to_string_lossy())); }
        entries.push(RemoteEntry { name: name.into(), url: url.into(), directory: stat.is_dir(), modified_at: stat.mtime.map(|time| time as i64) });
    }
    entries.sort_by(|left, right| right.directory.cmp(&left.directory).then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase())));
    Ok(entries)
}

type Cache = HashMap<(String, String, String), tempfile::NamedTempFile>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

pub fn resolve(address: &str, auth: Option<&(String, String)>) -> Result<String, String> {
    let auth = auth.ok_or("SFTP credentials are required. Open the connection in Remote first.")?;
    let (url, path) = target(address)?;
    let key = (address.to_owned(), auth.0.clone(), auth.1.clone());
    let mut cache = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().map_err(|_| "SFTP cache unavailable")?;
    if !cache.contains_key(&key) {
        let sftp = connect(&url, auth)?;
        let mut remote = sftp.open(&path).map_err(|error| error.to_string())?;
        let suffix = path.extension().and_then(|extension| extension.to_str()).map(|extension| format!(".{extension}")).unwrap_or_default();
        let mut local = tempfile::Builder::new().prefix("kaleido-sftp-").suffix(&suffix).tempfile().map_err(|error| error.to_string())?;
        std::io::copy(&mut remote, &mut local).map_err(|error| error.to_string())?;
        cache.insert(key.clone(), local);
    }
    Url::from_file_path(cache[&key].path()).map(String::from).map_err(|_| "Invalid SFTP cache path".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires KALEIDO_SFTP_TEST_HOST and a verified known_hosts entry"]
    fn live_host_key_verification() {
        let host = std::env::var("KALEIDO_SFTP_TEST_HOST").unwrap();
        let mut session = ssh2::Session::new().unwrap();
        let address = (host.as_str(), 22).to_socket_addrs().unwrap().next().unwrap();
        session.set_tcp_stream(TcpStream::connect_timeout(&address, Duration::from_secs(10)).unwrap());
        session.set_timeout(10_000);
        let known_hosts = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".ssh/known_hosts");
        prefer_known_host_keys(&session, &host, 22, &known_hosts).unwrap();
        session.handshake().unwrap();
        let mut hosts = session.known_hosts().unwrap();
        hosts.read_file(&PathBuf::from(std::env::var_os("HOME").unwrap()).join(".ssh/known_hosts"), ssh2::KnownHostFileKind::OpenSSH).unwrap();
        let (key, kind) = session.host_key().unwrap();
        eprintln!("Negotiated host key: {kind:?}");
        assert!(matches!(hosts.check_port(&host, 22, key), ssh2::CheckResult::Match));
    }

    #[test]
    fn validates_servers_and_decodes_paths() {
        assert!(valid_url("sftp://example.com:2222/Music/"));
        assert_eq!(target("sftp://example.com/Music/My%20Song.mp3").unwrap().1, PathBuf::from("/Music/My Song.mp3"));
        for address in ["https://example.com/", "sftp://user:secret@example.com/", "sftp://example.com/?query", "sftp://example.com/%00"] {
            assert!(!valid_url(address));
        }
        assert!(same_server("sftp://example.com/", "sftp://example.com:22/Music/"));
        assert!(!same_server("sftp://example.com/", "sftp://example.com:2222/"));
        assert!(!same_server("sftp://example.com/", "sftp://other.example.com/"));
    }

    #[test]
    fn rejects_browsing_a_different_server_before_connecting() {
        let connection = RemoteConnection { name: "Music".into(), url: "sftp://example.com/".into(), username: "user".into() };
        assert!(browse(&connection, "", "sftp://other.example.com/").unwrap_err().contains("configured SFTP server"));
    }

    #[test]
    fn widget_preferences_persist_independently() {
        let mut library: crate::model::Library = serde_json::from_str("{}").unwrap();
        assert!(library.show_meters && library.show_meters_compact);
        assert!(library.show_recent_tracks && library.show_recent_tracks_compact);
        library.show_meters_compact = false;
        library.show_recent_tracks = false;
        let restored: crate::model::Library = serde_json::from_str(&serde_json::to_string(&library).unwrap()).unwrap();
        assert!(restored.show_meters && !restored.show_meters_compact);
        assert!(!restored.show_recent_tracks && restored.show_recent_tracks_compact);
    }
}
