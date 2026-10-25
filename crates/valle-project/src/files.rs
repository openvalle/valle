//! File naming, classification, and independent copies shared by projects and the asset library.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use unicode_normalization::UnicodeNormalization;

use crate::{
    ContentDigest,
    folder::{Project, absolute, digest_file},
};

/// The existing preview identity checks, also used by scans and source digest caches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStamp {
    pub len: u64,
    pub modified: String,
    pub identity: String,
    pub changed: Option<(i64, i64)>,
}

impl FileStamp {
    pub fn read(path: &Path) -> Result<Self> {
        Self::from_file(&fs::File::open(path)?)
    }

    pub fn from_file(file: &fs::File) -> Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            bail!("expected an ordinary file");
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            len: metadata.len(),
            modified: format!("{:?}", metadata.modified()?),
            #[cfg(unix)]
            identity: format!("{}:{}", metadata.dev(), metadata.ino()),
            #[cfg(windows)]
            identity: windows_file_identity(file)?,
            #[cfg(not(any(unix, windows)))]
            identity: String::new(),
            #[cfg(unix)]
            changed: Some((metadata.ctime(), metadata.ctime_nsec())),
            #[cfg(not(unix))]
            changed: None,
        })
    }
}

#[cfg(windows)]
fn windows_file_identity(file: &fs::File) -> std::io::Result<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO, FileIdInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx,
    };
    let mut extended = std::mem::MaybeUninit::<FILE_ID_INFO>::uninit();
    // The successful call initializes the complete FILE_ID_INFO for this open handle.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            extended.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    } != 0
    {
        let info = unsafe { extended.assume_init() };
        return Ok(format!(
            "{}:{}",
            info.VolumeSerialNumber,
            hex::encode(info.FileId.Identifier)
        ));
    }
    let mut legacy = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), legacy.as_mut_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let info = unsafe { legacy.assume_init() };
    Ok(format!(
        "{}:{}",
        info.dwVolumeSerialNumber,
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)
    ))
}

/// Rename within a volume with the OS's exclusive-destination primitive.
pub fn rename_new(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let from = CString::new(source.as_os_str().as_bytes())?;
        let to = CString::new(destination.as_os_str().as_bytes())?;
        #[cfg(target_os = "macos")]
        let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            bail!("exclusive renaming is unsupported on this platform");
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let from = source
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let to = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(), to.as_ptr(), 0)
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    crate::sync_directory(destination.parent().context("destination has no parent")?)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Video,
    Audio,
    Image,
    Font,
    Model3d,
    Environment,
    Shader,
    Component,
    Data,
    Other,
}

impl Kind {
    pub fn detect(path: &Path) -> Self {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if name.ends_with(".motion.tsx") {
            return Self::Component;
        }
        if name.ends_with(".shader.json") {
            return Self::Shader;
        }
        match path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str()
        {
            "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" | "mts" | "m2ts" | "mxf" | "mpg"
            | "mpeg" | "3gp" | "wmv" | "flv" => Self::Video,
            "mp3" | "wav" | "flac" | "ogg" | "opus" | "m4a" | "aac" | "aif" | "aiff" => Self::Audio,
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" | "avif" => {
                Self::Image
            }
            "ttf" | "otf" | "ttc" | "woff" | "woff2" => Self::Font,
            "glb" | "gltf" => Self::Model3d,
            "hdr" | "exr" => Self::Environment,
            "sksl" | "glsl" | "wgsl" | "frag" | "vert" => Self::Shader,
            "json" | "csv" | "tsv" => Self::Data,
            _ => {
                let mut bytes = [0; 16];
                let count = fs::File::open(path)
                    .and_then(|mut f| f.read(&mut bytes))
                    .unwrap_or(0);
                let bytes = &bytes[..count];
                if bytes.starts_with(b"\x89PNG\r\n")
                    || bytes.starts_with(b"\xff\xd8\xff")
                    || bytes.starts_with(b"GIF8")
                {
                    Self::Image
                } else if bytes.starts_with(b"glTF") {
                    Self::Model3d
                } else if bytes.starts_with(b"OTTO")
                    || bytes.starts_with(b"\0\x01\0\0")
                    || bytes.starts_with(b"ttcf")
                {
                    Self::Font
                } else if bytes.starts_with(b"fLaC")
                    || bytes.starts_with(b"ID3")
                    || bytes.starts_with(b"OggS")
                    || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE"))
                {
                    Self::Audio
                } else {
                    Self::Other
                }
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Image => "image",
            Self::Font => "font",
            Self::Model3d => "model3d",
            Self::Environment => "environment",
            Self::Shader => "shader",
            Self::Component => "component",
            Self::Data => "data",
            Self::Other => "other",
        }
    }
    pub fn directory(self) -> &'static str {
        match self {
            Self::Video => "videos",
            Self::Audio => "music",
            Self::Image => "images",
            Self::Font => "fonts",
            Self::Model3d | Self::Environment => "3d",
            Self::Shader => "shaders",
            Self::Component => "components",
            Self::Data => "data",
            Self::Other => "other",
        }
    }
    pub fn snapshot_source(self) -> bool {
        matches!(self, Self::Component | Self::Data | Self::Shader)
    }
}

pub fn folded(value: &str) -> String {
    value.nfc().flat_map(char::to_lowercase).collect()
}

pub fn files_in(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    let mut entries = fs::read_dir(root)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            files.extend(files_in(&entry.path())?);
        } else if kind.is_file() {
            files.push(entry.path());
        }
    }
    Ok(files)
}

pub fn occupied(directory: &Path, name: &str) -> Result<bool> {
    if !directory.exists() {
        return Ok(false);
    }
    let key = folded(name);
    for entry in fs::read_dir(directory)? {
        if folded(&entry?.file_name().to_string_lossy()) == key {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn suffixed(name: &str, number: usize) -> String {
    if number == 1 {
        return name.to_owned();
    }
    if name.to_ascii_lowercase().ends_with(".motion.tsx") {
        let split = name.len() - ".motion.tsx".len();
        return format!("{}-{number}{}", &name[..split], &name[split..]);
    }
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    match path.extension() {
        Some(extension) => format!("{stem}-{number}.{}", extension.to_string_lossy()),
        None => format!("{name}-{number}"),
    }
}

/// Make an independent clone or copy, then publish without replacing any existing file.
pub fn copy_new(
    source: &Path,
    destination: &Path,
    staging: &Path,
    expected: ContentDigest,
) -> Result<()> {
    fs::create_dir_all(destination.parent().context("destination has no parent")?)?;
    stage_copy(source, staging, expected)?
        .persist_noclobber(destination)
        .map_err(|e| e.error)?;
    crate::sync_directory(destination.parent().unwrap())?;
    Ok(())
}

pub(crate) fn stage_copy(
    source: &Path,
    staging: &Path,
    expected: ContentDigest,
) -> Result<tempfile::NamedTempFile> {
    fs::create_dir_all(staging)?;
    let (file, path) = tempfile::Builder::new()
        .prefix(".import-")
        .tempfile_in(staging)?
        .into_parts();
    drop(file);
    fs::remove_file(&path)?;
    reflink_copy::reflink_or_copy(source, &path)?;
    let mut permissions = fs::metadata(&path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o200);
    }
    #[cfg(windows)]
    #[expect(
        clippy::permissions_set_readonly_false,
        reason = "Windows only clears FILE_ATTRIBUTE_READONLY; Unix sets the owner-write bit above."
    )]
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions)?;
    let file = fs::OpenOptions::new().read(true).write(true).open(&path)?;
    file.sync_all()?;
    if digest_file(&path)? != expected {
        bail!("source changed during import: {}", source.display());
    }
    let temp = tempfile::NamedTempFile::from_parts(file, path);
    Ok(temp)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedFile {
    pub alias: String,
    pub path: String,
    pub kind: Kind,
    pub already_exists: bool,
    pub renamed: bool,
}

impl Project {
    /// The caller holds the project lock and validates component imports before invoking this.
    pub fn import_file(&self, source: &Path, link: bool) -> Result<ImportedFile> {
        let source = source
            .canonicalize()
            .with_context(|| format!("opening {}", source.display()))?;
        if !source.is_file() {
            bail!("not a file: {}", source.display());
        }
        let kind = Kind::detect(&source);
        let name = source
            .file_name()
            .context("source has no filename")?
            .to_string_lossy();
        let (path, already_exists, renamed) = if link {
            (source.clone(), true, false)
        } else {
            let digest = digest_file(&source)?;
            let length = fs::metadata(&source)?.len();
            let mut existing = None;
            for folder in ["components", "media"] {
                for path in files_in(&self.root.join(folder))? {
                    if fs::metadata(&path)?.len() == length && digest_file(&path)? == digest {
                        existing = Some(path);
                        break;
                    }
                }
                if existing.is_some() {
                    break;
                }
            }
            if let Some(path) = existing {
                (path, true, false)
            } else {
                let directory = self.root.join(if kind == Kind::Component {
                    "components"
                } else {
                    "media"
                });
                fs::create_dir_all(&directory)?;
                if directory.canonicalize()?.strip_prefix(&self.root).is_err() {
                    bail!("import directory is outside the project");
                }
                let mut number = 1;
                loop {
                    let filename = suffixed(&name, number);
                    if occupied(&directory, &filename)? {
                        number += 1;
                        continue;
                    }
                    let target = directory.join(&filename);
                    match copy_new(&source, &target, &self.root.join(".valle/tmp"), digest) {
                        Ok(()) => break (target, false, number > 1),
                        Err(error)
                            if error
                                .downcast_ref::<std::io::Error>()
                                .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists) =>
                        {
                            number += 1;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        };
        let document = fs::read(self.timeline_path())
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .unwrap_or_default();
        let resources = document["resources"].as_object();
        let timeline_base = self.timeline_path().parent().unwrap().to_path_buf();
        let reused = resources.and_then(|r| {
            r.iter().find_map(|(alias, value)| {
                let value = value.as_str()?;
                if value.contains("://") {
                    return None;
                }
                (absolute(&timeline_base.join(value)).ok().as_ref() == Some(&path))
                    .then(|| alias.clone())
            })
        });
        let alias = if let Some(alias) = reused {
            alias
        } else {
            let stem = source.file_stem().unwrap_or_default().to_string_lossy();
            let base = stem
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                .take(48)
                .collect::<String>();
            let base = if base.chars().any(|c| c.is_ascii_alphanumeric()) {
                base
            } else {
                kind.as_str().to_owned()
            };
            let mut alias = base.clone();
            let mut number = 2;
            while resources.is_some_and(|r| r.contains_key(&alias)) {
                alias = format!("{base}-{number}");
                number += 1;
            }
            alias
        };
        Ok(ImportedFile {
            alias,
            path: if link {
                path.to_string_lossy().into_owned()
            } else {
                self.path_key(&path)?
            },
            kind,
            already_exists,
            renamed,
        })
    }
}
