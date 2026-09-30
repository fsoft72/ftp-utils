//! Tests `SuppaFtpConnection` against a minimal in-process FTP server, so
//! the real protocol paths (listing, streaming transfers, hash probing,
//! directory creation) are exercised without an external server.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ftp_utils_core::exclude::ExcludeSet;
use ftp_utils_core::ftp_client::SuppaFtpConnection;
use ftp_utils_core::hash::remote_md5;
use ftp_utils_core::remote::walk_remote;
use ftp_utils_core::FtpConnection;

const TIMEOUT: Duration = Duration::from_secs(5);

/// Server-side state shared with the test.
#[derive(Default)]
struct State {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
    /// Hash command the server understands (`XMD5` or `MD5`), if any.
    hash_command: Option<&'static str>,
    /// Every hash command received, in order.
    hash_requests: Vec<String>,
}

type Shared = Arc<Mutex<State>>;

struct FakeFtpServer {
    port: u16,
    state: Shared,
}

impl FakeFtpServer {
    fn start(state: State) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state: Shared = Arc::new(Mutex::new(state));

        let shared = Arc::clone(&state);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let shared = Arc::clone(&shared);
                thread::spawn(move || {
                    let _ = serve(stream, shared);
                });
            }
        });

        FakeFtpServer { port, state }
    }

    fn connect(&self) -> SuppaFtpConnection {
        SuppaFtpConnection::connect("127.0.0.1", self.port, "user", "pass", false, false, TIMEOUT).unwrap()
    }
}

fn reply(control: &mut TcpStream, line: &str) -> std::io::Result<()> {
    control.write_all(format!("{line}\r\n").as_bytes())
}

fn parent_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((parent, _)) => parent,
        None => "/",
    }
}

fn name_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, name)| name).unwrap_or(path)
}

/// Handles one control connection until QUIT or disconnect.
fn serve(control: TcpStream, state: Shared) -> std::io::Result<()> {
    let mut writer = control.try_clone()?;
    let mut reader = BufReader::new(control);
    let mut passive: Option<TcpListener> = None;

    reply(&mut writer, "220 fake ftp ready")?;

    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let command = line.trim_end().to_string();
        let (verb, argument) = command.split_once(' ').unwrap_or((command.as_str(), ""));

        match verb.to_ascii_uppercase().as_str() {
            "USER" => reply(&mut writer, "331 password please")?,
            "PASS" => reply(&mut writer, "230 logged in")?,
            "TYPE" => reply(&mut writer, "200 type set")?,
            "PASV" => {
                let listener = TcpListener::bind("127.0.0.1:0")?;
                let port = listener.local_addr()?.port();
                reply(&mut writer, &format!("227 Entering Passive Mode (127,0,0,1,{},{})", port >> 8, port & 0xff))?;
                passive = Some(listener);
            }
            "LIST" => {
                let Some(listener) = passive.take() else {
                    reply(&mut writer, "425 no data connection")?;
                    continue;
                };
                reply(&mut writer, "150 here comes the listing")?;
                let (mut data, _) = listener.accept()?;
                let listing = {
                    let state = state.lock().unwrap();
                    let mut out = String::new();
                    for dir in state.dirs.iter().filter(|d| parent_of(d) == argument && d.as_str() != "/") {
                        out.push_str(&format!("drwxr-xr-x 2 user group 4096 Jan 10 12:00 {}\r\n", name_of(dir)));
                    }
                    for (file, bytes) in state.files.iter().filter(|(f, _)| parent_of(f) == argument) {
                        out.push_str(&format!(
                            "-rw-r--r-- 1 user group {} Jan 10 12:00 {}\r\n",
                            bytes.len(),
                            name_of(file)
                        ));
                    }
                    out
                };
                data.write_all(listing.as_bytes())?;
                drop(data);
                reply(&mut writer, "226 listing done")?;
            }
            "RETR" => {
                let Some(listener) = passive.take() else {
                    reply(&mut writer, "425 no data connection")?;
                    continue;
                };
                let content = state.lock().unwrap().files.get(argument).cloned();
                let Some(content) = content else {
                    reply(&mut writer, "550 no such file")?;
                    continue;
                };
                reply(&mut writer, "150 sending")?;
                let (mut data, _) = listener.accept()?;
                data.write_all(&content)?;
                drop(data);
                reply(&mut writer, "226 transfer complete")?;
            }
            "STOR" => {
                let Some(listener) = passive.take() else {
                    reply(&mut writer, "425 no data connection")?;
                    continue;
                };
                reply(&mut writer, "150 ready to receive")?;
                let (mut data, _) = listener.accept()?;
                let mut content = Vec::new();
                data.read_to_end(&mut content)?;
                state.lock().unwrap().files.insert(argument.to_string(), content);
                reply(&mut writer, "226 stored")?;
            }
            "DELE" => {
                let removed = state.lock().unwrap().files.remove(argument).is_some();
                reply(&mut writer, if removed { "250 deleted" } else { "550 no such file" })?;
            }
            "MKD" => {
                state.lock().unwrap().dirs.insert(argument.to_string());
                reply(&mut writer, &format!("257 \"{argument}\" created"))?;
            }
            "XMD5" | "MD5" => {
                let mut state = state.lock().unwrap();
                state.hash_requests.push(verb.to_ascii_uppercase());
                let supported =
                    state.hash_command == Some(if verb.eq_ignore_ascii_case("XMD5") { "XMD5" } else { "MD5" });
                match (supported, state.files.get(argument)) {
                    (true, Some(content)) => {
                        let hash = format!("{:x}", md5::compute(content));
                        drop(state);
                        reply(&mut writer, &format!("251 {hash}"))?;
                    }
                    (true, None) => reply(&mut writer, "550 no such file")?,
                    (false, _) => reply(&mut writer, "500 unknown command")?,
                }
            }
            "QUIT" => {
                reply(&mut writer, "221 bye")?;
                return Ok(());
            }
            _ => reply(&mut writer, "502 not implemented")?,
        }
    }
}

fn state_with(files: &[(&str, &[u8])], dirs: &[&str]) -> State {
    State {
        files: files.iter().map(|(p, c)| (p.to_string(), c.to_vec())).collect(),
        dirs: dirs.iter().map(|d| d.to_string()).collect(),
        ..State::default()
    }
}

/// Deterministic pseudo-random bytes, larger than several transfer chunks.
fn big_payload() -> Vec<u8> {
    (0..300_000u32).map(|i| (i.wrapping_mul(31) % 251) as u8).collect()
}

#[test]
fn lists_files_and_directories_with_sizes() {
    let server = FakeFtpServer::start(state_with(&[("/data/a.txt", b"hello")], &["/data", "/data/sub"]));
    let mut conn = server.connect();

    let mut entries = conn.list_dir("/data").unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    assert_eq!(entries.len(), 2);
    assert_eq!((entries[0].name.as_str(), entries[0].is_dir, entries[0].size), ("a.txt", false, 5));
    assert_eq!((entries[1].name.as_str(), entries[1].is_dir), ("sub", true));
    conn.close();
}

#[test]
fn walk_remote_recurses_over_the_wire() {
    let server = FakeFtpServer::start(state_with(
        &[("/data/a.txt", b"12345"), ("/data/sub/b.txt", b"123")],
        &["/data", "/data/sub"],
    ));
    let mut conn = server.connect();

    let mut entries = walk_remote(&mut conn, "/data", &ExcludeSet::default(), None).unwrap();
    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    let summary: Vec<_> = entries.iter().map(|e| (e.relative_path.as_str(), e.size)).collect();
    assert_eq!(summary, vec![("a.txt", 5), ("sub/b.txt", 3)]);
    conn.close();
}

#[test]
fn streams_a_large_download_to_a_writer() {
    let payload = big_payload();
    let server = FakeFtpServer::start(state_with(&[("/big.bin", &payload)], &[]));
    let mut conn = server.connect();

    let mut received = Vec::new();
    let written = conn.retr_to_writer("/big.bin", &mut received).unwrap();

    assert_eq!(written as usize, payload.len());
    assert_eq!(received, payload);
    conn.close();
}

#[test]
fn connection_is_reusable_after_a_download() {
    let server = FakeFtpServer::start(state_with(&[("/a.txt", b"one"), ("/b.txt", b"two")], &[]));
    let mut conn = server.connect();

    assert_eq!(conn.retr_to_buffer("/a.txt").unwrap(), b"one");
    assert_eq!(conn.retr_to_buffer("/b.txt").unwrap(), b"two");
    conn.close();
}

#[test]
fn downloading_a_missing_file_is_an_error() {
    let server = FakeFtpServer::start(State::default());
    let mut conn = server.connect();

    let mut sink = Vec::new();
    assert!(conn.retr_to_writer("/nope", &mut sink).is_err());
    assert!(conn.retr_to_buffer("/nope").is_err());
    conn.close();
}

#[test]
fn streams_a_large_upload_from_a_reader() {
    let payload = big_payload();
    let server = FakeFtpServer::start(State::default());
    let mut conn = server.connect();

    conn.store_from_reader("/up.bin", &mut std::io::Cursor::new(payload.clone())).unwrap();
    conn.store_from_buffer("/small.txt", b"hi").unwrap();

    let state = server.state.lock().unwrap();
    assert_eq!(state.files.get("/up.bin"), Some(&payload));
    assert_eq!(state.files.get("/small.txt"), Some(&b"hi".to_vec()));
    drop(state);
    conn.close();
}

#[test]
fn deletes_files_and_reports_missing_ones() {
    let server = FakeFtpServer::start(state_with(&[("/a.txt", b"x")], &[]));
    let mut conn = server.connect();

    conn.delete("/a.txt").unwrap();
    assert!(conn.delete("/a.txt").is_err());

    assert!(server.state.lock().unwrap().files.is_empty());
    conn.close();
}

#[test]
fn ensure_remote_dir_creates_missing_parents_once() {
    let server = FakeFtpServer::start(state_with(&[], &["/"]));
    let mut conn = server.connect();

    conn.ensure_remote_dir("/a/b/file.txt").unwrap();
    conn.ensure_remote_dir("/a/b/other.txt").unwrap();

    let dirs: Vec<String> = server.state.lock().unwrap().dirs.iter().cloned().collect();
    assert_eq!(dirs, vec!["/".to_string(), "/a".to_string(), "/a/b".to_string()]);
    conn.close();
}

#[test]
fn try_hash_uses_the_server_side_command_and_remembers_it() {
    let mut state = state_with(&[("/a.txt", b"hello")], &[]);
    state.hash_command = Some("MD5");
    let server = FakeFtpServer::start(state);
    let mut conn = server.connect();

    let first = conn.try_hash("/a.txt");
    let second = conn.try_hash("/a.txt");

    assert_eq!(first.as_deref(), Some("5d41402abc4b2a76b9719d911017c592"));
    assert_eq!(second, first);
    // XMD5 was tried (and rejected) only during the first probe; afterwards
    // the working command is used directly.
    assert_eq!(server.state.lock().unwrap().hash_requests, vec!["XMD5", "MD5", "MD5"]);
    conn.close();
}

#[test]
fn try_hash_gives_up_after_one_failed_probe() {
    let server = FakeFtpServer::start(state_with(&[("/a.txt", b"hello")], &[]));
    let mut conn = server.connect();

    assert_eq!(conn.try_hash("/a.txt"), None);
    assert_eq!(conn.try_hash("/a.txt"), None);
    assert_eq!(conn.try_hash("/a.txt"), None);

    assert_eq!(server.state.lock().unwrap().hash_requests, vec!["XMD5", "MD5"]);
    conn.close();
}

#[test]
fn remote_md5_falls_back_to_a_streamed_download() {
    let payload = big_payload();
    let server = FakeFtpServer::start(state_with(&[("/big.bin", &payload)], &[]));
    let mut conn = server.connect();

    let hash = remote_md5(&mut conn, "/big.bin").unwrap();

    assert_eq!(hash, format!("{:x}", md5::compute(&payload)));
    conn.close();
}

#[test]
fn remote_md5_prefers_the_server_hash() {
    let mut state = state_with(&[("/a.txt", b"hello")], &[]);
    state.hash_command = Some("XMD5");
    let server = FakeFtpServer::start(state);
    let mut conn = server.connect();

    assert_eq!(remote_md5(&mut conn, "/a.txt").unwrap(), "5d41402abc4b2a76b9719d911017c592");
    conn.close();
}
