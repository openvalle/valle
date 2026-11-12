//! `valle update`, run on a copy of the executable against releases served from this machine:
//! what it installs, and that anything wrong with a release leaves the copy as it was.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

const VALLE: &str = env!("CARGO_BIN_EXE_valle");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const NAME: &str = if cfg!(windows) { "valle.exe" } else { "valle" };

/// The package's name for this machine, as the release job names it.
fn archive(tag: &str) -> String {
    let os = std::env::consts::OS;
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        arch => arch,
    };
    let kind = if cfg!(windows) { "zip" } else { "tar.gz" };
    format!("valle-{tag}-{os}-{arch}.{kind}")
}

/// `file` as the package's executable: a `.tar.gz`, or on Windows a `.zip` that stores it.
fn package(file: &[u8]) -> Vec<u8> {
    if cfg!(windows) {
        let mut fields = vec![0; 10];
        fields.extend([file.len() as u32; 2].map(u32::to_le_bytes).as_flattened());
        fields.extend((NAME.len() as u16).to_le_bytes());
        let mut out = b"PK\x03\x04\x14\0\0\0".to_vec();
        out.extend(&fields);
        out.extend([0; 2]);
        out.extend(NAME.as_bytes());
        out.extend(file);
        let at = out.len() as u32;
        out.extend(b"PK\x01\x02\x14\0\x14\0\0\0");
        out.extend(&fields);
        out.extend([0; 16]);
        out.extend(NAME.as_bytes());
        let size = out.len() as u32 - at;
        out.extend(b"PK\x05\x06\0\0\0\0\x01\0\x01\0");
        out.extend([size, at].map(u32::to_le_bytes).as_flattened());
        out.extend([0; 2]);
        return out;
    }
    let mut tar = vec![0u8; 512];
    tar[..NAME.len()].copy_from_slice(NAME.as_bytes());
    tar[124..136].copy_from_slice(format!("{:011o}\0", file.len()).as_bytes());
    tar[156] = b'0';
    tar.extend(file);
    tar.resize(tar.len().div_ceil(512) * 512 + 1024, 0);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&tar).unwrap();
    gz.finish().unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Releases served from this machine: `latest` redirects to the tag's page, as GitHub's does,
/// and `files` are the tag's.
struct Releases {
    base: String,
    /// How many files were asked for.
    downloads: Arc<AtomicUsize>,
}

impl Releases {
    fn serve(tag: &str, files: Vec<(String, Vec<u8>)>) -> Releases {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let downloads = Arc::new(AtomicUsize::new(0));
        let (tag, root, count) = (tag.to_string(), base.clone(), downloads.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                let _ = reader.read_line(&mut request);
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                    line.clear();
                }
                let path = request.split(' ').nth(1).unwrap_or("");
                let file = (path.strip_prefix(&format!("/download/{tag}/")))
                    .and_then(|name| files.iter().find(|(n, _)| n == name));
                let close = "Connection: close\r\n";
                let _ = match (path, file) {
                    ("/latest", _) => stream.write_all(
                        format!("HTTP/1.1 302 Found\r\nLocation: {root}/tag/{tag}\r\nContent-Length: 0\r\n{close}\r\n").as_bytes(),
                    ),
                    (_, Some((_, bytes))) => {
                        count.fetch_add(1, Ordering::Relaxed);
                        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{close}\r\n", bytes.len());
                        stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(bytes))
                    }
                    (path, None) if path.starts_with("/tag/") => {
                        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n{close}\r\n").as_bytes())
                    }
                    _ => stream.write_all(
                        format!("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n{close}\r\n").as_bytes(),
                    ),
                };
            }
        });
        Releases { base, downloads }
    }

    /// A release `tag` whose package holds `file`, with its sum, or `sum` in its place.
    fn of(tag: &str, file: &[u8], sum: Option<&str>) -> Releases {
        let (name, package) = (archive(tag), package(file));
        let sum = sum.map_or_else(|| sha256(&package), str::to_string);
        let sums = format!("{}  other.tgz\n{sum}  {name}\n", "0".repeat(64));
        let files = vec![
            ("SHA256SUMS".to_string(), sums.into_bytes()),
            (name, package),
        ];
        Releases::serve(tag, files)
    }
}

/// A folder with a copy of the executable, removed when dropped.
struct Installed(PathBuf);

impl Installed {
    fn new(name: &str) -> Installed {
        let folder =
            std::env::temp_dir().join(format!("valle-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::copy(VALLE, folder.join(NAME)).unwrap();
        Installed(folder)
    }

    fn exe(&self) -> PathBuf {
        self.0.join(NAME)
    }

    fn update(&self, releases: &Releases, version: Option<&str>) -> Output {
        let mut command = Command::new(self.exe());
        command.args(["update", "--json"]).args(version);
        command.env("VALLE_RELEASES", &releases.base);
        for proxy in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env_remove(proxy);
        }
        command.output().unwrap()
    }

    /// The folder's files other than the executable.
    fn others(&self) -> Vec<String> {
        let files = std::fs::read_dir(&self.0).unwrap().flatten();
        let names = files.map(|f| f.file_name().to_string_lossy().into_owned());
        names.filter(|n| n != NAME).collect()
    }
}

impl Drop for Installed {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn result(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn same(a: &Path, b: &Path) -> bool {
    std::fs::read(a).unwrap() == std::fs::read(b).unwrap()
}

/// A valle that only says its version, for systems where a script is an executable.
#[cfg(unix)]
fn script(version: &str) -> Vec<u8> {
    format!("#!/bin/sh\necho 'valle {version}'\n").into_bytes()
}

#[test]
fn the_latest_is_left_alone_when_this_is_it() {
    let installed = Installed::new("latest");
    let releases = Releases::serve(&format!("v{VERSION}"), Vec::new());
    let output = installed.update(&releases, None);
    let out = result(&output);
    assert!(output.status.success());
    assert_eq!(out["summary"], format!("valle {VERSION} is the latest"));
    assert_eq!(releases.downloads.load(Ordering::Relaxed), 0);
    assert!(same(&installed.exe(), Path::new(VALLE)));
}

/// On every system: the release named is installed over the executable while it runs, here
/// the same executable, which is one that says the version asked for.
#[test]
fn a_release_asked_for_by_name_takes_the_place_of_the_one_running() {
    let installed = Installed::new("named");
    let releases = Releases::of(&format!("v{VERSION}"), &std::fs::read(VALLE).unwrap(), None);
    let before = std::fs::metadata(installed.exe())
        .unwrap()
        .modified()
        .unwrap();
    for _ in 0..2 {
        let output = installed.update(&releases, Some(VERSION));
        let out = result(&output);
        assert!(output.status.success(), "{out}");
        assert_eq!(out["version"], VERSION);
        assert_eq!(
            out["file"].as_str().map(|f| Path::new(f).file_name()),
            Some(Some(NAME.as_ref()))
        );
    }
    assert!(same(&installed.exe(), Path::new(VALLE)));
    assert!(
        std::fs::metadata(installed.exe())
            .unwrap()
            .modified()
            .unwrap()
            > before
    );
    let said = Command::new(installed.exe())
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&said.stdout).trim(),
        format!("valle {VERSION}")
    );
    // Windows keeps the last one replaced, which was running; the one before it is swept.
    let others = installed.others();
    assert!(others.len() <= usize::from(cfg!(windows)), "{others:?}");
}

#[cfg(unix)]
#[test]
fn a_newer_latest_is_installed_and_says_what_else_to_update() {
    let installed = Installed::new("newer");
    let releases = Releases::of("v99.0.1", &script("99.0.1"), None);
    let output = installed.update(&releases, None);
    let out = result(&output);
    assert!(output.status.success(), "{out}");
    assert_eq!(
        (&out["from"], &out["version"]),
        (&VERSION.into(), &"99.0.1".into())
    );
    assert_eq!(
        out["notes"],
        "https://github.com/openvalle/valle/releases/tag/v99.0.1"
    );
    // Another minor version: the skill is written for one.
    assert_eq!(out["skill"], "npx skills add openvalle/valle");
    assert_eq!(std::fs::read(installed.exe()).unwrap(), script("99.0.1"));
    assert_eq!(installed.others(), Vec::<String>::new());
}

#[test]
fn a_package_that_does_not_match_its_sum_changes_nothing() {
    let installed = Installed::new("sum");
    let tag = format!("v{VERSION}");
    let releases = Releases::of(&tag, &std::fs::read(VALLE).unwrap(), Some(&"ab".repeat(32)));
    let output = installed.update(&releases, Some(VERSION));
    let out = result(&output);
    assert!(!output.status.success());
    assert_eq!(out["code"], "update");
    assert!(
        out["message"]
            .as_str()
            .unwrap()
            .contains("does not match its checksum"),
        "{out}"
    );
    assert!(same(&installed.exe(), Path::new(VALLE)));
    assert_eq!(installed.others(), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn a_valle_that_does_not_say_the_version_changes_nothing() {
    let installed = Installed::new("says");
    for (file, said) in [
        (script("99.0.0"), "valle 99.0.0"),
        (b"#!/bin/sh\nexit 3\n".to_vec(), "it stopped"),
    ] {
        let releases = Releases::of("v99.0.1", &file, None);
        let output = installed.update(&releases, None);
        let out = result(&output);
        assert!(!output.status.success());
        assert!(out["message"].as_str().unwrap().contains(said), "{out}");
        assert_eq!(out["hint"], format!("valle {VERSION} is still in place"));
        assert!(same(&installed.exe(), Path::new(VALLE)));
        assert_eq!(installed.others(), Vec::<String>::new());
    }
}

#[test]
fn a_release_that_is_not_there_says_so() {
    let installed = Installed::new("missing");
    let releases = Releases::serve("v99.0.1", Vec::new());
    let out = result(&installed.update(&releases, Some("99.0.2")));
    assert_eq!(out["code"], "update");
    assert!(
        out["message"].as_str().unwrap().contains("v99.0.2"),
        "{out}"
    );
    let out = result(&installed.update(&releases, Some("soon")));
    assert_eq!(out["code"], "usage");
    assert!(same(&installed.exe(), Path::new(VALLE)));
}
