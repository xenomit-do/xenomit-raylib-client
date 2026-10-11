param(
    [ValidateSet('windows', 'linux', 'android')]
    [string]$Target = 'windows',
    [switch]$Release,
    [string]$AndroidSdkRoot = $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { $env:ANDROID_SDK_ROOT }),
    [string]$AndroidNdkHome = $env:ANDROID_NDK_HOME,
    [string]$JavaHome = $env:JAVA_HOME
)
$ErrorActionPreference = 'Stop'

if ($Target -eq 'windows') {
    $buildArgs = @('build', '--bin', 'xenomit-client')
    if ($Release) { $buildArgs += '--release' }
    cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Windows build failed.' }
    exit 0
}

if ($Target -eq 'linux') {
    & (Join-Path $PSScriptRoot 'build-linux.ps1')
    exit 0
}

function Invoke-Cargo {
    param([Parameter(ValueFromRemainingArguments)][string[]]$Args)
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & cargo @Args
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($code -ne 0) { throw "cargo $($Args -join ' ') failed (exit $code)." }
}

function Invoke-RustupTargetAdd {
    param([string]$Triple)
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    rustup target add $Triple
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($code -ne 0) { throw "Could not install the Rust target $Triple." }
}

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
if (-not $AndroidSdkRoot) { throw 'Android SDK not found. Set ANDROID_HOME or install Android Build Support in Unity Hub.' }
if (-not $AndroidNdkHome -or -not (Test-Path -LiteralPath (Join-Path $AndroidNdkHome 'build\cmake\android.toolchain.cmake'))) { throw 'Set ANDROID_NDK_HOME to a complete Android NDK.' }
if (-not $JavaHome -or -not (Test-Path -LiteralPath (Join-Path $JavaHome 'bin\java.exe'))) { throw 'Set JAVA_HOME to an installed Java runtime.' }
if (-not (Test-Path -LiteralPath '..\game_resources\main.swf')) { throw 'Android packaging embeds ../game_resources/main.swf; build from the full sibling checkout.' }
$env:ANDROID_HOME = $AndroidSdkRoot
$env:ANDROID_NDK_HOME = $AndroidNdkHome
$env:JAVA_HOME = $JavaHome
$env:CMAKE_GENERATOR = 'Ninja'
$env:CARGO_TARGET_DIR = Join-Path (Get-Location).Path 'target\android-build'
. (Join-Path $PSScriptRoot 'android-ndk-env.ps1') -AndroidNdkHome $AndroidNdkHome
Invoke-RustupTargetAdd 'aarch64-linux-android'
if (-not (Get-Command cargo-apk -ErrorAction SilentlyContinue)) {
    Invoke-Cargo install cargo-apk
}
if ($Release) {
    if (($env:CARGO_APK_RELEASE_KEYSTORE -and -not $env:CARGO_APK_RELEASE_KEYSTORE_PASSWORD) -or
        (-not $env:CARGO_APK_RELEASE_KEYSTORE -and $env:CARGO_APK_RELEASE_KEYSTORE_PASSWORD)) {
        throw 'Set both CARGO_APK_RELEASE_KEYSTORE and CARGO_APK_RELEASE_KEYSTORE_PASSWORD, or leave both unset for a local test key.'
    }
    if (-not $env:CARGO_APK_RELEASE_KEYSTORE) {
        $null = New-Item -ItemType Directory -Path $env:CARGO_TARGET_DIR -Force
        $testKeystore = Join-Path $env:CARGO_TARGET_DIR 'test-signing.keystore'
        if (-not (Test-Path -LiteralPath $testKeystore)) {
            & (Join-Path $JavaHome 'bin\keytool.exe') -genkeypair -noprompt -keystore $testKeystore -storepass android -keypass android -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 -dname 'CN=Xenomit Local Test,O=Xenomit,C=US'
            if ($LASTEXITCODE -ne 0) { throw 'Could not create the local Android test signing key.' }
        }
        $env:CARGO_APK_RELEASE_KEYSTORE = $testKeystore
        $env:CARGO_APK_RELEASE_KEYSTORE_PASSWORD = 'android'
        Write-Host 'Signing with a local test key. Set CARGO_APK_RELEASE_KEYSTORE and CARGO_APK_RELEASE_KEYSTORE_PASSWORD for a personal release key.'
    }
}
$buildArgs = @('apk', 'build', '--manifest-path', 'android/Cargo.toml', '--lib')
if ($Release) { $buildArgs += '--release' }
Invoke-Cargo @buildArgs
