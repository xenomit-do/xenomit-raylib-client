This vendor copy is raylib-sys 5.5.1 from raylib-rs, distributed under its
original Zlib license (see `raylib/LICENSE`). Local functional patches:

- `build.rs` parses Android API level from `CARGO_NDK_ANDROID_PLATFORM` or
  `ANDROID_PLATFORM`, with API 23 as the default. The published build script
  used `android` (the canonical Rust target suffix) as a CMake API level.
- Android bindgen receives the selected NDK sysroot and API-qualified target
  so it can find the standard C headers while cross-compiling on Windows.
- `raylib.h` and `platforms/rcore_android.c` expose the NativeActivity
  private storage path and soft keyboard show/hide calls to the Rust shell.
- The Android key path fills raylib's character queue for the ASCII server
  and cookie text fields; `GetClipboardText` bridges to Android
  `ClipboardManager` only when the application explicitly requests paste.
  UI/device IME behavior still needs on-device verification.
