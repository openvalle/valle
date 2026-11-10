//! The adapter frames are drawn on natively: the system's GPU, or, where it has none or only a
//! CPU one, lavapipe, so every machine without a GPU draws the same pixels.

use std::task::{Context, Poll, Waker};

/// An adapter, and whether it is the bundled lavapipe's.
pub struct Chosen {
    pub adapter: wgpu::Adapter,
    pub bundled: bool,
}

impl Chosen {
    /// What draws, as a person reads it.
    pub fn name(&self) -> String {
        let name = self.adapter.get_info().name;
        match self.bundled {
            true => format!("{name} (bundled lavapipe)"),
            false => name,
        }
    }
}

/// The system's default adapter, which `WGPU_BACKEND` and `WGPU_ADAPTER_NAME` may choose; but
/// lavapipe where that is missing or a CPU's: `$VALLE_LAVAPIPE`, a driver valle-lavapipe
/// compressed, or else `embedded`, the one the executable carries.
pub fn adapter(embedded: Option<&[u8]>) -> Result<Chosen, String> {
    let system = system();
    let gpu = (system.as_ref()).is_ok_and(|a| a.get_info().device_type != wgpu::DeviceType::Cpu);
    if !gpu && let Some(bundled) = bundled(embedded) {
        match (bundled, &system) {
            (Ok(adapter), _) => {
                return Ok(Chosen {
                    adapter,
                    bundled: true,
                });
            }
            (Err(e), Err(none)) => return Err(format!("{none}; {e}")),
            // The system's CPU adapter, then.
            (Err(_), Ok(_)) => {}
        }
    }
    system.map(|adapter| Chosen {
        adapter,
        bundled: false,
    })
}

fn system() -> Result<wgpu::Adapter, String> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    now(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))?
    .map_err(|e| format!("no GPU adapter: {e}"))
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn bundled(embedded: Option<&[u8]>) -> Option<Result<wgpu::Adapter, String>> {
    match std::env::var_os("VALLE_LAVAPIPE") {
        Some(path) => Some(
            (std::fs::read(&path))
                .map_err(|e| format!("VALLE_LAVAPIPE {}: {e}", path.display()))
                .and_then(|driver| crate::lavapipe::adapter(&driver)),
        ),
        None => embedded.map(crate::lavapipe::adapter),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn bundled(_: Option<&[u8]>) -> Option<Result<wgpu::Adapter, String>> {
    None
}

/// What a wgpu request gives natively, where it completes at once.
pub(crate) fn now<T>(request: impl Future<Output = T>) -> Result<T, String> {
    match std::pin::pin!(request).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => Ok(v),
        Poll::Pending => Err("a wgpu request did not complete at once".into()),
    }
}
