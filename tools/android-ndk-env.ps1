# Shared Android NDK / bindgen environment for cargo and cargo-apk on Windows hosts.
param(
    [Parameter(Mandatory)]
    [string]$AndroidNdkHome,
    [int]$ApiLevel = 23
)
$ErrorActionPreference = 'Stop'

$prebuilt = Join-Path $AndroidNdkHome 'toolchains\llvm\prebuilt\windows-x86_64'
if (-not (Test-Path -LiteralPath $prebuilt)) {
    throw "NDK LLVM prebuilt not found under $prebuilt"
}
$bin = Join-Path $prebuilt 'bin'
$triple = "aarch64-linux-android$ApiLevel"

$env:ANDROID_NDK_HOME = $AndroidNdkHome
$env:CARGO_NDK_ANDROID_PLATFORM = "$ApiLevel"
$env:ANDROID_PLATFORM = "android-$ApiLevel"
$env:CC_aarch64_linux_android = Join-Path $bin "$triple-clang.cmd"
$env:CXX_aarch64_linux_android = Join-Path $bin "$triple-clang++.cmd"
$env:AR_aarch64_linux_android = Join-Path $bin 'llvm-ar.exe'
$env:RANLIB_aarch64_linux_android = Join-Path $bin 'llvm-ranlib.exe'

$androidSysroot = Join-Path $prebuilt 'sysroot'
$androidClangLib = Join-Path $prebuilt 'lib\clang'
$androidResourceDir = Get-ChildItem -LiteralPath $androidClangLib -Directory |
    Sort-Object Name -Descending |
    Select-Object -First 1 -ExpandProperty FullName
$env:BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android =
    "--target=$triple --sysroot=`"$androidSysroot`" -resource-dir=`"$androidResourceDir`""
