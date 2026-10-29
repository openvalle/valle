//! Capture Ganesh's deferred pipeline compilation errors before delivering a frame.
use std::{
    ffi::{CStr, c_char, c_void},
    sync::Mutex,
};

use super::surface::SurfaceError;

unsafe extern "C" {
    fn valle_shader_errors_new(
        context: *mut c_void,
        callback: extern "C" fn(*mut c_void, *const c_char),
    ) -> *mut c_void;
    fn valle_shader_errors_delete(handler: *mut c_void);
}

pub(super) struct ShaderErrors {
    handler: *mut c_void,
    // Stable address across moves; the callback may run on a driver compilation thread.
    error: Box<Mutex<Option<String>>>,
}

impl ShaderErrors {
    pub fn new() -> Self {
        let mut error = Box::new(Mutex::new(None));
        let handler =
            unsafe { valle_shader_errors_new(std::ptr::from_mut(&mut *error).cast(), capture) };
        Self { handler, error }
    }

    /// The handler must outlive the context created with these options.
    pub fn options(&self) -> skia_safe::gpu::ContextOptions {
        let mut options = skia_safe::gpu::ContextOptions::default();
        const {
            assert!(
                std::mem::size_of::<skia_safe::gpu::ContextOptions>()
                    == std::mem::size_of::<skia_bindings::GrContextOptions>()
            );
            assert!(
                std::mem::align_of::<skia_safe::gpu::ContextOptions>()
                    == std::mem::align_of::<skia_bindings::GrContextOptions>()
            );
        }
        // Both types have the same repr(C) layout, checked by rust-skia and above. The
        // public Skia field is still private in its Rust wrapper at the pinned version.
        unsafe {
            let raw = std::ptr::from_mut(&mut options).cast::<skia_bindings::GrContextOptions>();
            (*raw).fShaderErrorHandler = self.handler.cast();
        }
        options
    }

    pub fn check(&self) -> Result<(), SurfaceError> {
        // Keep the failure sticky: a failed pipeline can remain in the context's cache.
        let error = self.error.lock().unwrap_or_else(|p| p.into_inner());
        match error.as_ref() {
            Some(message) => Err(SurfaceError::ShaderCompilation(message.clone())),
            None => Ok(()),
        }
    }
}

extern "C" fn capture(context: *mut c_void, message: *const c_char) {
    // The C++ handler and callback context stay alive until after DirectContext is destroyed.
    let error = unsafe { &*context.cast::<Mutex<Option<String>>>() };
    let mut error = error.lock().unwrap_or_else(|p| p.into_inner());
    if error.is_none() {
        let message = if message.is_null() {
            "unknown shader compilation failure".into()
        } else {
            unsafe { CStr::from_ptr(message) }.to_string_lossy()
        };
        *error = Some(message.chars().take(8192).collect());
    }
}

impl Drop for ShaderErrors {
    fn drop(&mut self) {
        unsafe { valle_shader_errors_delete(self.handler) };
    }
}
