# Building the raylib client

The client is one Rust/raylib workspace for Windows, Linux, and Android. Game code lives in the root crate (`xenomit-raylib-client` / library `xenomit_client`); `android/` is only the NativeActivity `cdylib` entry that calls `xenomit_client::client::run_android()`. Platform differences are limited to raylib's window backend (Win32 / X11+GLX / Android EGL+GLES), startup sizing, private storage, and APK packaging.

| Platform | Rust target | Binary | Helper |
|----------|-------------|--------|--------|
| Windows desktop | host (`x86_64-pc-windows-msvc`) | `target\release\xenomit-client.exe` | `tools\build.ps1` or `cargo build --bin xenomit-client` |
| Linux desktop | `x86_64-unknown-linux-gnu` (native on Linux/WSL) | `target/linux/release/xenomit-client` | `tools/build-linux.sh` or `tools\build-linux.ps1` (WSL from Windows) |
| Android | `aarch64-linux-android` (APK via `cargo-apk`) | `target/android-build/release/apk/*.apk` | `tools\build.ps1 -Target android` |

Raylib is built from `vendor/raylib-sys` (patched for Android API level and bindgen). Desktop Linux needs X11/OpenGL dev packages; Android uses OpenGL ES via the NDK toolchain. Optional Cargo feature `wayland` on `raylib` is not enabled by default (X11 backend).

## Windows

From PowerShell in this directory:

```powershell
$env:LIBCLANG_PATH = 'C:\path\to\clang\bin'
cargo build --release --bin xenomit-client
```

The existing MSVC and libclang setup in `AGENTS.md` also applies. Run `target\release\xenomit-client.exe`; pass `--server tr3 --dosid <session-cookie>` to skip the sign-in form. The cookie is read only from the command line and is not stored by the build scripts.

## Linux

Install Rust, CMake, Ninja, pkg-config, Clang/libclang, and raylib's X11/OpenGL development libraries. On Debian/Ubuntu the native window dependencies are `libx11-dev libxrandr-dev libxinerama-dev libxcursor-dev libxi-dev libgl1-mesa-dev libasound2-dev`. Then run:

```sh
./tools/build-linux.sh
```

The Linux helper uses an isolated `target/linux` directory and finds `libclang` when installed; its binary is `target/linux/release/xenomit-client`. The same Rust client source builds on Linux and Windows; no separate UI implementation is maintained.

**From a Windows host:** cross-compiling `x86_64-unknown-linux-gnu` with MSVC as the only toolchain is not practical (raylib-sys uses CMake + bindgen against the target sysroot). Use WSL instead:

```powershell
.\tools\build-linux.ps1
```

That runs `./tools/build-linux.sh` inside the `Ubuntu` WSL distro and leaves the ELF at `target\linux\release\xenomit-client`. A plain `cargo build --target x86_64-unknown-linux-gnu` on Windows without a Linux linker/sysroot is expected to fail; do not treat it as a supported path.

## Android (ARM64 APK)

Install an Android SDK, Java 17+, and the Android NDK. Set `ANDROID_HOME`, `ANDROID_NDK_HOME`, and `JAVA_HOME`; the NDK must include `build/cmake/android.toolchain.cmake`. The build helper discovers the Unity Android toolchain when present and supplies the matching NDK settings needed by raylib and bindgen. Build from this directory with `tools/build.ps1 -Target android -Release` on Windows, or `tools/build-android.sh --release` on Linux. These helpers install the ARM64 Rust target and `cargo-apk` if needed. For a manual setup, install the target and `cargo-apk` once:

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk
cargo apk build --release --manifest-path android/Cargo.toml --lib
```

The small `android/` crate is only the APK's NativeActivity cdylib entrypoint; the game application and renderer stay in the root Rust library. Its manifest requests a landscape NativeActivity, internet access, min SDK 23, and an ARM64 package. Android uses an isolated `target/android-build` Cargo target directory so it does not hold the Windows desktop build lock; the APK is written under `target/android-build/release/apk/`. At startup, the stock `game_resources/main.swf` is included in the ARM64 binary and unpacked once to Android's app-private `internalDataPath/.xenomit-assets/main.swf`; this supplies the same embedded game XML and asset recipes as desktop. CDN files and settings are stored beside it. The Android build must therefore run from a checkout that also has `../game_resources/main.swf`.

For local release-mode testing, the helper creates and reuses a test signing key inside `target/android-build`. Set both `CARGO_APK_RELEASE_KEYSTORE` and `CARGO_APK_RELEASE_KEYSTORE_PASSWORD` to use your own release key. The generated key is only for local builds and must not be used to distribute the app; APKs signed with different keys cannot update one another.

The authenticated cookie is never compiled into the APK. Enter the server and cookie at the sign-in screen. Tapping either field requests the Android system soft keyboard; the native key path maps the ASCII letters and digits used by the server name and cookie into raylib's text queue. `PASTE DOSID` reads Android's clipboard only when pressed. Physical device/emulator IME and paste behavior still need hardware verification.

A custom NDK can be pointed to explicitly on Windows:

```powershell
$env:ANDROID_SDK_ROOT = 'C:\Android\Sdk'
$env:ANDROID_NDK_HOME = "$env:ANDROID_SDK_ROOT\ndk\<version>"
$env:JAVA_HOME = 'C:\Program Files\Java\jdk-17'
cargo apk build --release --manifest-path android/Cargo.toml --lib
```

## Desktop touch-layout preview and performance

`--mobile-preview` enables the mobile HUD in a landscape 20:9 desktop window (1280×576 by default). `--width` and `--height` override it. Desktop rendering is uncapped so the built-in log reports average FPS and last-frame time every five seconds. Android retains a 60 Hz ceiling for battery use.

Use `--exit-after N` for repeatable timed runs, for example:

```powershell
.\target\release\xenomit-client.exe --mobile-preview --exit-after 60
```

The timer closes the app through its normal shutdown path. Run performance tests with a separately supplied session; do not put session cookies in scripts, docs, or config files committed to the repository.
