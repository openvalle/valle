//! Lavapipe, Mesa's CPU Vulkan driver, carried in the executable Brotli-compressed and called
//! directly, without the Vulkan loader: no environment, manifest or registry is read, so a
//! machine without a GPU draws as every other does, in an elevated Windows process too.
//!
//! The driver is decompressed once into the global cache, as `lavapipe/<SHA-256 of the
//! compressed driver>/` and its name, and loaded from there: only the pages of it in use take
//! memory, which the system may reclaim, and every process shares them. A copy written whole is
//! moved into place, so a broken one leaves nothing behind. Where the cache cannot be written,
//! Linux loads the driver from memory instead.

use std::ffi::CStr;
use std::fs::{self, File};
use std::io::{BufWriter, Write};

use ash::vk;

#[cfg(target_os = "linux")]
const NAME: &str = "libvulkan_lvp.so";
#[cfg(target_os = "windows")]
const NAME: &str = "vulkan_lvp.dll";

/// The interface version the driver is asked for: Valle needs none of the loader's later ones.
const INTERFACE: u32 = 5;

/// The adapter of the driver `compressed` holds.
pub fn adapter(compressed: &[u8]) -> Result<wgpu::Adapter, String> {
    let failed = |e: String| format!("the bundled lavapipe: {e}");
    let library = load(compressed).map_err(failed)?;
    // SAFETY: the library is a Vulkan driver, which `entry` checks, and stays loaded for the
    // process, as the instance calls into it until the process ends.
    let instance = unsafe {
        let library = Box::leak(Box::new(library));
        instance(entry(library).map_err(failed)?).map_err(failed)?
    };
    let adapters = crate::gpu::now(instance.enumerate_adapters(wgpu::Backends::VULKAN));
    (adapters.map_err(failed)?.into_iter().next()).ok_or_else(|| failed("it has no device".into()))
}

/// The driver, from the global cache, or on Linux from memory where the cache fails.
fn load(compressed: &[u8]) -> Result<libloading::Library, String> {
    #[cfg(target_os = "linux")]
    {
        from_file(compressed).or_else(|cached| {
            from_memory(compressed).map_err(|memory| format!("{cached}; from memory: {memory}"))
        })
    }
    #[cfg(target_os = "windows")]
    from_file(compressed)
}

/// The driver `compressed` holds, decompressed into `file` as it is read: never whole in memory.
fn decompress(compressed: &[u8], file: &mut File) -> Result<(), String> {
    let mut out = BufWriter::with_capacity(1 << 20, file);
    brotli_decompressor::BrotliDecompress(&mut &compressed[..], &mut out)
        .and_then(|()| out.flush())
        .map_err(|e| format!("cannot decompress it: {e}"))
}

/// The driver from the global cache, written there first if it is not.
fn from_file(compressed: &[u8]) -> Result<libloading::Library, String> {
    let digest = ring::digest::digest(&ring::digest::SHA256, compressed);
    let hex: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    let dir = crate::urls::global_dir()?.join("lavapipe").join(hex);
    let path = dir.join(NAME);
    if !path.is_file() {
        let shown = |e: std::io::Error| format!("{}: {e}", dir.display());
        fs::create_dir_all(&dir).map_err(shown)?;
        let part = dir.join(format!("{NAME}.{}.part", std::process::id()));
        let written = (File::create(&part).map_err(shown)).and_then(|mut f| {
            decompress(compressed, &mut f)?;
            f.sync_all().map_err(shown)
        });
        if let Err(e) = written {
            let _ = fs::remove_file(&part);
            return Err(e);
        }
        // Another process may have moved its copy into place first; on Windows, one loaded
        // cannot be replaced, so that copy is kept.
        if let Err(e) = fs::rename(&part, &path) {
            let _ = fs::remove_file(&part);
            if !path.is_file() {
                return Err(shown(e));
            }
        }
    }
    // SAFETY: the driver's initializers are Mesa's, which only set up the driver.
    unsafe { libloading::Library::new(&path) }.map_err(|e| format!("{}: {e}", path.display()))
}

/// The driver from memory, which it then takes whole for as long as the process runs.
#[cfg(target_os = "linux")]
fn from_memory(compressed: &[u8]) -> Result<libloading::Library, String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    // SAFETY: the name is a C string; the descriptor returned is new and owned by `file`.
    let mut file = unsafe {
        let fd = libc::memfd_create(c"lavapipe".as_ptr(), libc::MFD_CLOEXEC);
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        File::from_raw_fd(fd)
    };
    decompress(compressed, &mut file)?;
    // The library stays mapped once loaded, after the descriptor is closed. A kernel may forbid
    // running code from memory, as `vm.memfd_noexec` does; loading fails then.
    let path = format!("/proc/self/fd/{}", file.as_raw_fd());
    // SAFETY: as `from_file`.
    unsafe { libloading::Library::new(path) }.map_err(|e| e.to_string())
}

/// The driver's entry point, after the interface negotiation the loader would do.
///
/// # Safety
/// `library` must stay loaded while the entry is used.
unsafe fn entry(library: &libloading::Library) -> Result<ash::Entry, String> {
    type Negotiate = unsafe extern "system" fn(*mut u32) -> vk::Result;
    let symbol = |name: &CStr| format!("it does not export {}", name.to_string_lossy());
    // SAFETY: the types are those the Vulkan loader interface gives these functions.
    unsafe {
        let name = c"vk_icdNegotiateLoaderICDInterfaceVersion";
        let negotiate = library.get::<Negotiate>(name.to_bytes_with_nul());
        let negotiate = negotiate.map_err(|_| symbol(name))?;
        let mut version = INTERFACE;
        let result = negotiate(&mut version);
        if result != vk::Result::SUCCESS {
            return Err(format!(
                "it refused interface version {INTERFACE}: {result}"
            ));
        }
        let name = c"vk_icdGetInstanceProcAddr";
        let get = library.get::<vk::PFN_vkGetInstanceProcAddr>(name.to_bytes_with_nul());
        let get_instance_proc_addr = *get.map_err(|_| symbol(name))?;
        Ok(ash::Entry::from_static_fn(ash::StaticFn {
            get_instance_proc_addr,
        }))
    }
}

/// An instance of `entry`'s driver, as wgpu makes one of the loader's.
///
/// # Safety
/// `entry` must stay valid for the instance's lifetime.
unsafe fn instance(entry: ash::Entry) -> Result<wgpu::Instance, String> {
    use wgpu::hal::vulkan;
    let flags = wgpu::InstanceFlags::from_build_config().with_env();
    // SAFETY: `entry` is a driver's, whose global functions these are.
    unsafe {
        let version = (entry.try_enumerate_instance_version())
            .map_err(|e| format!("vkEnumerateInstanceVersion: {e}"))?
            .unwrap_or(vk::API_VERSION_1_0);
        let extensions = vulkan::Instance::desired_extensions(&entry, version, flags)
            .map_err(|e| e.to_string())?;
        let names: Vec<_> = extensions.iter().map(|e| e.as_ptr()).collect();
        // The most wgpu-hal uses, as it asks the loader for.
        let app = vk::ApplicationInfo::default()
            .application_name(c"valle")
            .engine_name(c"wgpu-hal")
            .api_version(match version {
                v if v < vk::API_VERSION_1_1 => vk::API_VERSION_1_0,
                _ => vk::API_VERSION_1_3,
            });
        let info = vk::InstanceCreateInfo::default()
            .application_info(&app)
            .enabled_extension_names(&names);
        let raw =
            (entry.create_instance(&info, None)).map_err(|e| format!("vkCreateInstance: {e}"))?;
        let hal = vulkan::Instance::from_raw(
            entry,
            raw,
            version,
            0,
            None,
            extensions,
            flags,
            Default::default(),
            false,
            None,
        )
        .map_err(|e| e.to_string())?;
        Ok(wgpu::Instance::from_hal::<wgpu::hal::api::Vulkan>(hal))
    }
}
