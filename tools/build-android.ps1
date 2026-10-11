param(
    [string]$AndroidSdkRoot = $env:ANDROID_SDK_ROOT,
    [string]$AndroidNdkHome = $env:ANDROID_NDK_HOME,
    [string]$JavaHome = $env:JAVA_HOME,
    [switch]$Release
)
$ErrorActionPreference = 'Stop'

if (-not $AndroidSdkRoot) {
    $editorRoot = Join-Path $env:ProgramFiles 'Unity\Hub\Editor'
    $androidPlayers = Get-ChildItem -LiteralPath $editorRoot -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending |
        ForEach-Object { Join-Path $_.FullName 'Editor\Data\PlaybackEngines\AndroidPlayer' } |
        Where-Object { Test-Path -LiteralPath (Join-Path $_ 'SDK\platforms') }
    $androidPlayer = $androidPlayers | Select-Object -First 1
    if ($androidPlayer) {
        $AndroidSdkRoot = Join-Path $androidPlayer 'SDK'
        if (-not $AndroidNdkHome) { $AndroidNdkHome = Join-Path $androidPlayer 'NDK' }
        if (-not $JavaHome) { $JavaHome = Join-Path $androidPlayer 'OpenJDK' }
    }
}
if (-not $AndroidSdkRoot) { throw 'Set ANDROID_SDK_ROOT (or ANDROID_HOME) to the Android SDK directory, or install Unity Android Build Support.' }
if (-not $AndroidNdkHome) {
    $ndkRoot = Join-Path $AndroidSdkRoot 'ndk'
    if (Test-Path -LiteralPath $ndkRoot) {
        $AndroidNdkHome = Get-ChildItem -LiteralPath $ndkRoot -Directory | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty FullName
    }
}
if (-not $AndroidNdkHome -or -not (Test-Path -LiteralPath (Join-Path $AndroidNdkHome 'build\cmake\android.toolchain.cmake'))) {
    throw 'Could not find the Android NDK CMake toolchain. Set ANDROID_NDK_HOME to an installed NDK.'
}
if (-not $JavaHome -or -not (Test-Path -LiteralPath (Join-Path $JavaHome 'bin\java.exe'))) {
    throw 'Set JAVA_HOME to an installed Java 17+ runtime.'
}
if (-not (Test-Path -LiteralPath '..\game_resources\main.swf')) {
    throw 'Android packaging embeds ../game_resources/main.swf; build from the full sibling checkout.'
}
$env:ANDROID_HOME = $AndroidSdkRoot
Remove-Item Env:ANDROID_SDK_ROOT -ErrorAction SilentlyContinue
$env:ANDROID_NDK_HOME = $AndroidNdkHome
$env:JAVA_HOME = $JavaHome
$env:CMAKE_GENERATOR = 'Ninja'
$env:CARGO_TARGET_DIR = Join-Path (Get-Location).Path 'target\android-build'
. (Join-Path $PSScriptRoot 'android-ndk-env.ps1') -AndroidNdkHome $AndroidNdkHome
$prevEa = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
rustup target add aarch64-linux-android
$rustupCode = $LASTEXITCODE
$ErrorActionPreference = $prevEa
if ($rustupCode -ne 0) { throw 'Could not install the Rust Android target.' }
if (-not (Get-Command cargo-apk -ErrorAction SilentlyContinue)) {
    $ErrorActionPreference = 'Continue'
    cargo install cargo-apk
    $ErrorActionPreference = $prevEa
    if ($LASTEXITCODE -ne 0) { throw 'Could not install cargo-apk.' }
}
$cargoArgs = @('apk', 'build', '--manifest-path', 'android/Cargo.toml', '--lib')
if ($Release) { $cargoArgs += '--release' }
$ErrorActionPreference = 'Continue'
cargo @cargoArgs
$apkCode = $LASTEXITCODE
$ErrorActionPreference = $prevEa
if ($apkCode -ne 0) { throw 'Android APK build failed.' }
