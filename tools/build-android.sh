#!/usr/bin/env sh
set -eu
: "${ANDROID_SDK_ROOT:?Set ANDROID_SDK_ROOT to the Android SDK directory}"
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME to the Android NDK directory}"
: "${JAVA_HOME:?Set JAVA_HOME to Java 17 or newer}"
test -f "$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake" || {
  echo "ANDROID_NDK_HOME does not contain the Android CMake toolchain" >&2
  exit 1
}
case "$(uname -s)" in
  Linux*) ndk_host="linux-x86_64" ;;
  Darwin*) ndk_host="darwin-x86_64" ;;
  *) echo "Unsupported Android build host: $(uname -s)" >&2; exit 1 ;;
esac
export CMAKE_GENERATOR=Ninja
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/android-build}"
api_level=23
ndk_toolchain="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$ndk_host"
ndk_clang_resource="$(find "$ndk_toolchain/lib/clang" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -n 1)"
triple="aarch64-linux-android${api_level}"
export CARGO_NDK_ANDROID_PLATFORM="$api_level"
export ANDROID_PLATFORM="android-${api_level}"
export CC_aarch64_linux_android="$ndk_toolchain/bin/${triple}-clang"
export CXX_aarch64_linux_android="$ndk_toolchain/bin/${triple}-clang++"
export AR_aarch64_linux_android="$ndk_toolchain/bin/llvm-ar"
export RANLIB_aarch64_linux_android="$ndk_toolchain/bin/llvm-ranlib"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--target=$triple --sysroot=$ndk_toolchain/sysroot -resource-dir=$ndk_clang_resource"
test -f ../game_resources/main.swf || {
  echo "Android packaging embeds ../game_resources/main.swf" >&2
  exit 1
}
rustup target add aarch64-linux-android
if ! command -v cargo-apk >/dev/null 2>&1; then cargo install cargo-apk; fi
if [ -n "${CARGO_APK_RELEASE_KEYSTORE:-}" ] || [ -n "${CARGO_APK_RELEASE_KEYSTORE_PASSWORD:-}" ]; then
  : "${CARGO_APK_RELEASE_KEYSTORE:?Set both CARGO_APK_RELEASE_KEYSTORE and CARGO_APK_RELEASE_KEYSTORE_PASSWORD}"
  : "${CARGO_APK_RELEASE_KEYSTORE_PASSWORD:?Set both CARGO_APK_RELEASE_KEYSTORE and CARGO_APK_RELEASE_KEYSTORE_PASSWORD}"
else
  test_keystore="$CARGO_TARGET_DIR/test-signing.keystore"
  if [ ! -f "$test_keystore" ]; then
    mkdir -p "$CARGO_TARGET_DIR"
    "$JAVA_HOME/bin/keytool" -genkeypair -noprompt -keystore "$test_keystore" -storepass android -keypass android -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 -dname 'CN=Xenomit Local Test,O=Xenomit,C=US'
  fi
  export CARGO_APK_RELEASE_KEYSTORE="$test_keystore"
  export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
  echo 'Signing with a local test key. Set CARGO_APK_RELEASE_KEYSTORE and CARGO_APK_RELEASE_KEYSTORE_PASSWORD for a personal release key.'
fi
cargo apk build --release --manifest-path android/Cargo.toml --lib
