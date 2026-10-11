//! NativeActivity cdylib shim. All gameplay and window code stays in the
//! shared xenomit-raylib-client library; the shim exports the C `main`
//! callback that raylib's Android glue invokes.

use std::ffi::c_void;

// raylib-sys renames the NDK glue entry at C compile time so rustc's cdylib
// export list can expose this Rust forwarding symbol to Android's loader.
unsafe extern "C" {
    fn XenomitNativeActivity_onCreate(
        activity: *mut c_void,
        saved_state: *mut c_void,
        saved_state_size: usize,
    );
}

#[no_mangle]
pub unsafe extern "C" fn ANativeActivity_onCreate(
    activity: *mut c_void,
    saved_state: *mut c_void,
    saved_state_size: usize,
) {
    XenomitNativeActivity_onCreate(activity, saved_state, saved_state_size);
}

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *mut *mut std::ffi::c_char) -> i32 {
    xenomit_client::client::run_android()
}
