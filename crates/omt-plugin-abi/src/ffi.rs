//! The C ABI boundary. Nothing here is `unsafe` — these are just the
//! `#[repr(C)]` types and `extern "C"` signatures both sides agree on. The
//! actual pointer dereferences live in `omt-plugin` (SDK) and
//! `omt-nativehost` (loader), each in a tightly scoped `unsafe` block.
//!
//! **Memory model — the "sink" pattern.** No buffer is ever allocated by
//! one side and freed by the other (Rust has no shared allocator across a
//! dynamic boundary). Instead, whoever produces bytes lends them as a
//! `(ptr, len)` valid only for the duration of a **sink** call; the
//! receiver copies them. So:
//!
//! * the plugin's `register` / `dispatch` emit their JSON by calling a
//!   host-provided [`Sink`];
//! * when the plugin needs a host operation, it calls [`Host::call`] with
//!   its *own* sink to receive the reply.
//!
//! The exported symbols a plugin must provide (the `omt-plugin` macro emits
//! them): `omt_plugin_abi_version() -> u32`,
//! `omt_plugin_register(sink_ctx, sink)` and
//! `omt_plugin_dispatch(req_ptr, req_len, host, sink_ctx, sink)`.

use core::ffi::c_void;

/// Receives one lent byte buffer. `ctx` is opaque receiver state.
///
/// # Safety
/// `ptr`..`ptr+len` is a valid, initialized, immutable byte range for the
/// duration of the call only; the callee must copy what it needs.
pub type Sink = unsafe extern "C" fn(ctx: *mut c_void, ptr: *const u8, len: usize);

/// How a plugin calls back into the host during a dispatch.
#[repr(C)]
pub struct Host {
    /// Opaque host state.
    pub ctx: *mut c_void,
    /// Run one [`crate::HostRequest`] (JSON at `req_ptr`..`+req_len`) and
    /// deliver the [`crate::HostResponse`] JSON by calling `sink(sink_ctx,
    /// …)` exactly once before returning.
    ///
    /// # Safety
    /// `req_ptr`..`req_len` must be a valid byte range for the call; `sink`
    /// must be a valid function pointer.
    pub call: unsafe extern "C" fn(ctx: *mut c_void, req_ptr: *const u8, req_len: usize, sink_ctx: *mut c_void, sink: Sink),
}

/// Exported symbol names the loader looks up.
pub const SYM_ABI_VERSION: &[u8] = b"omt_plugin_abi_version";
pub const SYM_REGISTER: &[u8] = b"omt_plugin_register";
pub const SYM_DISPATCH: &[u8] = b"omt_plugin_dispatch";

/// Signature of `omt_plugin_abi_version`.
pub type FnAbiVersion = unsafe extern "C" fn() -> u32;
/// Signature of `omt_plugin_register`.
pub type FnRegister = unsafe extern "C" fn(sink_ctx: *mut c_void, sink: Sink);
/// Signature of `omt_plugin_dispatch`.
pub type FnDispatch = unsafe extern "C" fn(req_ptr: *const u8, req_len: usize, host: *const Host, sink_ctx: *mut c_void, sink: Sink);
