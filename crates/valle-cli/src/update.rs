//! `valle update`: replaces this executable with a release's, as the install scripts do. The
//! package is checked against the release's SHA256SUMS, and the executable in it must run and
//! say its version before it takes this one's place, so a download that is broken, or that
//! this machine cannot run, leaves the one in use as it was.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use ureq::ResponseExt;

use crate::Failure;

/// Where the releases are, unless `VALLE_RELEASES` names a mirror of them.
const RELEASES: &str = "https://github.com/openvalle/valle/releases";

/// The most a package or a list of sums may be: several times the largest.
const LIMIT: u64 = 512 << 20;

/// A version as its three numbers, from `0.1.0` or `v0.1.0`.
fn numbers(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.strip_prefix('v').unwrap_or(version).split('.');
    let numbers = [parts.next()?, parts.next()?, parts.next()?].map(|n| n.parse().ok());
    match (numbers, parts.next()) {
        ([Some(major), Some(minor), Some(patch)], None) => Some([major, minor, patch]),
        _ => None,
    }
}

/// This machine as the packages name it.
fn platform() -> Result<String, Failure> {
    let os = match std::env::consts::OS {
        os @ ("macos" | "linux" | "windows") => os,
        os => return Err(failed(format!("Valle has no package for {os}"))),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "arm64",
        arch => return Err(failed(format!("Valle has no package for {arch}"))),
    };
    Ok(format!("{os}-{arch}"))
}

fn failed(message: impl Into<String>) -> Failure {
    Failure::new("update", message)
}

/// The executable `name` of a release's package: of a `.tar.gz`, or of a `.zip`.
fn unpack(archive: &[u8], zip: bool, name: &str) -> Result<Vec<u8>, String> {
    match zip {
        true => unzip(archive, name),
        false => {
            let mut tar = Vec::new();
            (flate2::read::GzDecoder::new(archive).read_to_end(&mut tar))
                .map_err(|e| format!("the package is not gzip: {e}"))?;
            untar(&tar, name)
        }
    }
}

/// The file `name` of a tar archive.
fn untar(tar: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let mut at = 0;
    while let Some(header) = tar.get(at..at + 512) {
        let text = |field: &[u8]| {
            let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
            String::from_utf8_lossy(&field[..end]).trim().to_string()
        };
        let entry = text(&header[..100]);
        if entry.is_empty() {
            break;
        }
        let size = usize::from_str_radix(&text(&header[124..136]), 8)
            .map_err(|_| "the package's tar is not one")?;
        let regular = matches!(header[156], b'0' | 0);
        if regular && entry.trim_start_matches("./") == name {
            let file = tar.get(at + 512..at + 512 + size);
            return Ok(file.ok_or("the package's tar ends early")?.to_vec());
        }
        at += 512 + size.div_ceil(512) * 512;
    }
    Err(format!("the package has no {name}"))
}

/// The file `name` of a zip archive, stored or deflated, found in its central directory.
fn unzip(zip: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let short = || "the package's zip ends early".to_string();
    let u16_at = |at: usize| {
        let bytes = zip.get(at..at + 2).ok_or_else(short)?;
        Ok::<usize, String>(u16::from_le_bytes([bytes[0], bytes[1]]) as usize)
    };
    let u32_at = |at: usize| {
        let bytes = zip.get(at..at + 4).ok_or_else(short)?;
        Ok::<usize, String>(u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
    };
    // The end record is last, before a comment of at most 65535 bytes.
    let end = (0..=zip.len().saturating_sub(22))
        .rev()
        .take(65536)
        .find(|&at| zip.get(at..at + 4) == Some(b"PK\x05\x06"))
        .ok_or("the package is not a zip")?;
    let (entries, mut at) = (u16_at(end + 10)?, u32_at(end + 16)?);
    for _ in 0..entries {
        if zip.get(at..at + 4) != Some(b"PK\x01\x02") {
            return Err("the package's zip is not one".into());
        }
        let (method, packed, size) = (u16_at(at + 10)?, u32_at(at + 20)?, u32_at(at + 24)?);
        let lengths = (u16_at(at + 28)?, u16_at(at + 30)?, u16_at(at + 32)?);
        let local = u32_at(at + 42)?;
        let entry = zip.get(at + 46..at + 46 + lengths.0).ok_or_else(short)?;
        at += 46 + lengths.0 + lengths.1 + lengths.2;
        if entry != name.as_bytes() {
            continue;
        }
        // The local header has its own name and extra field, then the data.
        let data = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
        let data = zip.get(data..data + packed).ok_or_else(short)?;
        return match method {
            0 => Ok(data.to_vec()),
            8 => {
                let mut file = Vec::with_capacity(size);
                (flate2::read::DeflateDecoder::new(data).read_to_end(&mut file))
                    .map_err(|e| format!("the package's zip does not unpack: {e}"))?;
                Ok(file)
            }
            other => Err(format!("the package's zip is packed by method {other}")),
        };
    }
    Err(format!("the package has no {name}"))
}

/// The SHA-256 that `sums`, a SHA256SUMS file, lists for `name`.
fn listed<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let (sum, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then_some(sum)
    })
}

fn sha256(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// The releases, as the install scripts read them: the latest by where its page redirects to,
/// which is no API and has no rate limit, and a release's files by their names.
struct Releases {
    base: String,
    agent: ureq::Agent,
}

impl Releases {
    fn new() -> Releases {
        let base = std::env::var("VALLE_RELEASES").unwrap_or_else(|_| RELEASES.into());
        let config = ureq::Agent::config_builder()
            .proxy(ureq::Proxy::try_from_env())
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .build();
        Releases {
            base: base.trim_end_matches('/').to_string(),
            agent: config.into(),
        }
    }

    /// The latest release's tag.
    fn latest(&self) -> Result<String, Failure> {
        let url = format!("{}/latest", self.base);
        let response = (self.agent.head(&url).call())
            .map_err(|e| failed(format!("could not find the latest release: {url}: {e}")))?;
        let tag = response.get_uri().path().rsplit('/').next().unwrap_or("");
        match numbers(tag) {
            Some(_) => Ok(tag.to_string()),
            None => Err(failed(format!(
                "could not tell the latest release from {}",
                response.get_uri()
            ))
            .hint("name one, as in `valle update 0.1.0`")),
        }
    }

    /// The file `name` of the release `tag`.
    fn file(&self, tag: &str, name: &str) -> Result<Vec<u8>, Failure> {
        let url = format!("{}/download/{tag}/{name}", self.base);
        (self.agent.get(&url).call())
            .and_then(|mut r| r.body_mut().with_config().limit(LIMIT).read_to_vec())
            .map_err(|e| match e {
                ureq::Error::StatusCode(404) => {
                    failed(format!("there is no release {tag} with a {name}: {url}"))
                }
                e => failed(format!("could not download {url}: {e}")),
            })
    }
}

/// What a replaced executable is renamed to while it runs, and a new one is written as:
/// names beside `exe` that start with its own, which `sweep` removes.
fn beside(exe: &Path, kind: &str) -> PathBuf {
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let suffix = std::env::consts::EXE_SUFFIX;
    exe.with_file_name(format!(
        "{name}.{kind}.{}-{unique}{suffix}",
        std::process::id()
    ))
}

/// Removes what earlier updates and installs left beside `exe`: on Windows an executable in
/// use cannot be removed, only renamed, so the one replaced stays until a later run.
fn sweep(exe: &Path) {
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    let (old, new) = (format!("{name}.old"), format!("{name}.new."));
    let entries = exe
        .parent()
        .and_then(|folder| std::fs::read_dir(folder).ok());
    for entry in entries.into_iter().flatten().flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        if file.starts_with(&old) || file.starts_with(&new) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Puts `new` in the place of `exe`, which is running.
fn replace(exe: &Path, new: &Path) -> std::io::Result<()> {
    if cfg!(windows) {
        // Windows does not write over an executable in use, but renames it.
        let old = beside(exe, "old");
        std::fs::rename(exe, &old)?;
        if let Err(e) = std::fs::rename(new, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(e);
        }
        let _ = std::fs::remove_file(old);
        Ok(())
    } else {
        // A rename replaces the file without touching the one in use.
        std::fs::rename(new, exe)
    }
}

/// How to install again, where this executable cannot be replaced.
fn reinstall() -> &'static str {
    match cfg!(windows) {
        true => "to install again: irm https://openvalle.ai/install.ps1 | iex",
        false => "to install again: curl -fsSL https://openvalle.ai/install.sh | sh",
    }
}

pub fn run(version: Option<String>, quiet: bool) -> Result<Value, Failure> {
    let current = env!("CARGO_PKG_VERSION");
    let releases = Releases::new();
    let tag = match &version {
        Some(version) => match numbers(version) {
            Some([major, minor, patch]) => format!("v{major}.{minor}.{patch}"),
            None => {
                return Err(Failure::new("usage", format!("{version} is not a version"))
                    .hint("a release is three numbers, as in `valle update 0.1.0`"));
            }
        },
        None => releases.latest()?,
    };
    let to = tag.trim_start_matches('v');
    // The latest is installed only where it is newer; a version asked for by name, always.
    if version.is_none() && numbers(to) <= numbers(current) {
        return Ok(json!({
            "status": "ok",
            "version": current,
            "latest": to,
            "summary": match to == current {
                true => format!("valle {current} is the latest"),
                false => format!("valle {current} is newer than the latest release, {to}"),
            },
        }));
    }

    let exe = std::env::current_exe()
        .map_err(|e| failed(format!("could not tell where this valle is: {e}")))?;
    // A link to it stays a link: the file it names is the one replaced.
    let exe = match cfg!(windows) {
        true => exe,
        false => exe.canonicalize().unwrap_or(exe),
    };
    sweep(&exe);

    let platform = platform()?;
    let zip = cfg!(windows);
    let archive = format!(
        "valle-{tag}-{platform}.{}",
        if zip { "zip" } else { "tar.gz" }
    );
    if !quiet {
        eprintln!("Downloading Valle {to} for {platform}");
    }
    let sums = releases.file(&tag, "SHA256SUMS")?;
    let sums = String::from_utf8_lossy(&sums);
    let expected = listed(&sums, &archive)
        .ok_or_else(|| failed(format!("SHA256SUMS of {tag} does not list {archive}")))?;
    let package = releases.file(&tag, &archive)?;
    let actual = sha256(&package);
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(failed(format!(
            "{archive} does not match its checksum: expected {expected}, got {actual}"
        )));
    }
    let name = format!("valle{}", std::env::consts::EXE_SUFFIX);
    let file = unpack(&package, zip, &name).map_err(failed)?;

    // Beside the one in use, so that taking its place is a rename.
    let new = beside(&exe, "new");
    let written = std::fs::write(&new, &file).and_then(|()| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    });
    if let Err(e) = written {
        let _ = std::fs::remove_file(&new);
        let folder = exe.parent().unwrap_or(&exe).display();
        return Err(failed(format!("could not write in {folder}: {e}")).hint(reinstall()));
    }
    let said = std::process::Command::new(&new).arg("--version").output();
    let said = match said {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Ok(out) => format!("it stopped ({})", out.status),
        Err(e) => format!("it did not start: {e}"),
    };
    if said != format!("valle {to}") {
        let _ = std::fs::remove_file(&new);
        return Err(failed(format!(
            "the valle of {tag} does not run here as {to}: {said}"
        ))
        .hint(format!("valle {current} is still in place")));
    }
    if let Err(e) = replace(&exe, &new) {
        let _ = std::fs::remove_file(&new);
        return Err(failed(format!("could not replace {}: {e}", exe.display())).hint(reinstall()));
    }

    let notes = format!("{RELEASES}/tag/{tag}");
    let done = match to == current {
        true => format!("Installed valle {to} again"),
        false => format!("Updated valle {current} to {to}"),
    };
    let mut summary = format!(
        "{done}: {}\n\
         What changed: {notes}\n\
         A Studio that is open runs the valle it started with until it is started again.",
        exe.display()
    );
    // The skill is written for a minor version, and is installed apart from the executable.
    let skill = numbers(to).map(|n| n[..2].to_vec()) != numbers(current).map(|n| n[..2].to_vec());
    if skill {
        summary.push_str("\nThe skill changes with it: npx skills add openvalle/valle");
    }
    Ok(json!({
        "status": "ok",
        "from": current,
        "version": to,
        "file": exe.display().to_string(),
        "notes": notes,
        "skill": skill.then_some("npx skills add openvalle/valle"),
        "summary": summary,
    }))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn versions_are_three_numbers_and_compare_as_numbers() {
        assert_eq!(numbers("v0.1.10"), Some([0, 1, 10]));
        assert!(numbers("0.1.10") > numbers("0.1.9"));
        for not in ["", "v1", "1.2", "1.2.3.4", "1.2.x", "1.2.3-rc1", "latest"] {
            assert_eq!(numbers(not), None, "{not}");
        }
    }

    #[test]
    fn a_sum_is_found_by_its_file_s_name() {
        let sums = "aa  valle-v0.1.1-linux-arm64.tar.gz\nbb *valle-v0.1.1-macos-arm64.tar.gz\n";
        assert_eq!(listed(sums, "valle-v0.1.1-macos-arm64.tar.gz"), Some("bb"));
        assert_eq!(listed(sums, "valle-v0.1.1-linux-arm64.tar.gz"), Some("aa"));
        assert_eq!(listed(sums, "arm64.tar.gz"), None);
    }

    /// A tar of `files`, as `tar` writes them.
    fn tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, bytes) in files {
            let mut header = [0u8; 512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            let size = format!("{:011o}\0", bytes.len());
            header[124..136].copy_from_slice(size.as_bytes());
            header[156] = b'0';
            out.extend(header);
            out.extend(*bytes);
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
        out.resize(out.len() + 1024, 0);
        out
    }

    #[test]
    fn the_executable_comes_out_of_a_tar_gz() {
        let files: [(&str, &[u8]); 2] = [("README", &[7; 700]), ("./valle", b"the executable")];
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&tar(&files)).unwrap();
        let gz = gz.finish().unwrap();
        assert_eq!(unpack(&gz, false, "valle").unwrap(), b"the executable");
        assert_eq!(
            unpack(&gz, false, "valle.exe").unwrap_err(),
            "the package has no valle.exe"
        );
        assert!(unpack(b"not gzip", false, "valle").is_err());
    }

    /// A zip of `files`, deflated or stored.
    fn zip(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
        let (mut out, mut directory) = (Vec::new(), Vec::new());
        for (name, bytes) in files {
            let packed = match deflate {
                true => {
                    let level = flate2::Compression::best();
                    let mut packer = flate2::write::DeflateEncoder::new(Vec::new(), level);
                    packer.write_all(bytes).unwrap();
                    packer.finish().unwrap()
                }
                false => bytes.to_vec(),
            };
            let mut fields = Vec::new();
            fields.extend(
                [if deflate { 8u16 } else { 0 }, 0, 0]
                    .map(u16::to_le_bytes)
                    .as_flattened(),
            );
            fields.extend(
                [0, packed.len() as u32, bytes.len() as u32]
                    .map(u32::to_le_bytes)
                    .as_flattened(),
            );
            fields.extend((name.len() as u16).to_le_bytes());
            let local = out.len() as u32;
            out.extend(b"PK\x03\x04\x14\0\0\0");
            out.extend(&fields);
            // An extra field in the local header only, as zip writes.
            out.extend(4u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend([0; 4]);
            out.extend(&packed);
            directory.extend(b"PK\x01\x02\x14\0\x14\0\0\0");
            directory.extend(&fields);
            directory.extend([0; 12]);
            directory.extend(local.to_le_bytes());
            directory.extend(name.as_bytes());
        }
        let at = out.len() as u32;
        out.extend(&directory);
        out.extend(b"PK\x05\x06\0\0\0\0");
        out.extend([files.len() as u16; 2].map(u16::to_le_bytes).as_flattened());
        out.extend(
            [directory.len() as u32, at]
                .map(u32::to_le_bytes)
                .as_flattened(),
        );
        out.extend(7u16.to_le_bytes());
        out.extend(b"comment");
        out
    }

    #[test]
    fn the_executable_comes_out_of_a_zip() {
        let big = vec![3u8; 100_000];
        let files: [(&str, &[u8]); 2] = [("notes.txt", b"notes"), ("valle.exe", &big)];
        for deflate in [true, false] {
            let zip = zip(&files, deflate);
            assert_eq!(unpack(&zip, true, "valle.exe").unwrap(), big);
            assert_eq!(unpack(&zip, true, "notes.txt").unwrap(), b"notes");
            assert_eq!(
                unpack(&zip, true, "valle").unwrap_err(),
                "the package has no valle"
            );
            // Cut anywhere, it is an error, not a panic.
            for cut in [0, 10, zip.len() / 2, zip.len() - 30] {
                assert!(unpack(&zip[..cut], true, "valle.exe").is_err(), "{cut}");
            }
        }
    }
}
