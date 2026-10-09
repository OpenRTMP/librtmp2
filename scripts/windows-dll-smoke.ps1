# Smoke-test a built librtmp2.dll on Windows through its C ABI.
#
#   scripts/windows-dll-smoke.ps1 -Dll target\x86_64-pc-windows-msvc\release\librtmp2.dll -Arch x86_64
#
# Checks that the DLL's PE machine type matches -Arch (x86_64 or arm64),
# that it loads with only the DLLs Windows itself provides (OpenSSL must be
# linked statically), that lrtmp2_version_string() matches Cargo.toml, that
# lrtmp2_tls_supported() reports TLS (unless -NoTls), and that a server
# created through the FFI accepts a real TCP connection.

param(
    [Parameter(Mandatory = $true)][string]$Dll,
    [Parameter(Mandatory = $true)][ValidateSet("x86_64", "arm64")][string]$Arch,
    [switch]$NoTls,
    [int]$Port = 19799
)

$ErrorActionPreference = "Stop"
$Dll = (Resolve-Path $Dll).Path

# PE header: e_lfanew at 0x3C, then "PE\0\0" and the 16-bit machine field.
$bytes = [System.IO.File]::ReadAllBytes($Dll)
$peOffset = [BitConverter]::ToInt32($bytes, 0x3C)
$machine = [BitConverter]::ToUInt16($bytes, $peOffset + 4)
$expected = @{ "x86_64" = 0x8664; "arm64" = 0xAA64 }[$Arch]
if ($machine -ne $expected) {
    throw ("{0} has PE machine 0x{1:X4}, expected 0x{2:X4} ({3})" -f $Dll, $machine, $expected, $Arch)
}
Write-Host ("PE machine 0x{0:X4} matches {1}" -f $machine, $Arch)

$processArch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
$nativeArch = @{ "x86_64" = "X64"; "arm64" = "Arm64" }[$Arch]
if ($processArch -ne $nativeArch) {
    throw "PowerShell runs as $processArch and cannot load a $Arch DLL; run this on a native $Arch host."
}

# Load from a directory holding nothing but the DLL, with PATH reduced to
# the Windows system directories, so a dependency on a non-system DLL
# (e.g. a dynamically linked libssl) fails here instead of in the field.
$isolated = Join-Path ([System.IO.Path]::GetTempPath()) ("librtmp2-smoke-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $isolated | Out-Null
Copy-Item $Dll $isolated
$env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"

$source = @"
using System;
using System.Runtime.InteropServices;

public static class Lrtmp2Smoke {
    [StructLayout(LayoutKind.Sequential)]
    public struct ServerConfig {
        public int max_connections;
        public int chunk_size;
        public int tls_enabled;
        public IntPtr tls_cert_file;
        public IntPtr tls_key_file;
        public IntPtr tls_ca_file;
        public int tls_insecure;
        public int max_pending_tls_per_addr;
        public int max_connections_per_addr;
    }

    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern IntPtr LoadLibraryW(string path);

    [DllImport("librtmp2.dll", CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr lrtmp2_version_string();
    [DllImport("librtmp2.dll", CallingConvention = CallingConvention.Cdecl)]
    public static extern int lrtmp2_tls_supported();
    [DllImport("librtmp2.dll", CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr lrtmp2_server_create(ref ServerConfig config);
    [DllImport("librtmp2.dll", CallingConvention = CallingConvention.Cdecl)]
    public static extern int lrtmp2_server_listen(IntPtr server, [MarshalAs(UnmanagedType.LPStr)] string bindAddr);
    [DllImport("librtmp2.dll", CallingConvention = CallingConvention.Cdecl)]
    public static extern int lrtmp2_server_poll(IntPtr server, int timeoutMs);
    [DllImport("librtmp2.dll", CallingConvention = CallingConvention.Cdecl)]
    public static extern void lrtmp2_server_destroy(IntPtr server);
}
"@
Add-Type -TypeDefinition $source

# Pre-load by absolute path so the DllImports below bind to this copy.
$handle = [Lrtmp2Smoke]::LoadLibraryW((Join-Path $isolated "librtmp2.dll"))
if ($handle -eq [IntPtr]::Zero) {
    $err = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
    throw "LoadLibrary failed with Win32 error $err (a missing dependency DLL is error 126)."
}
Write-Host "Loaded $Dll with only system DLLs on PATH"

$version = [System.Runtime.InteropServices.Marshal]::PtrToStringAnsi([Lrtmp2Smoke]::lrtmp2_version_string())
$cargoToml = Join-Path $PSScriptRoot "..\Cargo.toml"
if (Test-Path $cargoToml) {
    $crateVersion = (Select-String -Path $cargoToml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    if ($version -ne $crateVersion) {
        throw "lrtmp2_version_string() returned '$version', Cargo.toml says '$crateVersion'"
    }
}
Write-Host "lrtmp2_version_string() = $version"

$tls = [Lrtmp2Smoke]::lrtmp2_tls_supported()
$wantTls = if ($NoTls) { 0 } else { 1 }
if ($tls -ne $wantTls) {
    throw "lrtmp2_tls_supported() returned $tls, expected $wantTls"
}
Write-Host "lrtmp2_tls_supported() = $tls"

$config = New-Object Lrtmp2Smoke+ServerConfig
$config.max_connections = 4
$config.chunk_size = 4096
$server = [Lrtmp2Smoke]::lrtmp2_server_create([ref]$config)
if ($server -eq [IntPtr]::Zero) { throw "lrtmp2_server_create returned NULL" }
try {
    $rc = [Lrtmp2Smoke]::lrtmp2_server_listen($server, "127.0.0.1:$Port")
    if ($rc -ne 0) { throw "lrtmp2_server_listen failed with $rc" }
    $client = New-Object System.Net.Sockets.TcpClient
    $client.Connect("127.0.0.1", $Port)
    # Full C0 + C1: the server must accept the client and answer with S0
    # (version 3) followed by S1 and S2.
    $stream = $client.GetStream()
    $c0c1 = New-Object byte[] 1537
    $c0c1[0] = 3
    $stream.Write($c0c1, 0, $c0c1.Length)
    $want = 1 + 1536 + 1536
    $reply = New-Object byte[] $want
    $got = 0
    for ($i = 0; $i -lt 200 -and $got -lt $want; $i++) {
        $rc = [Lrtmp2Smoke]::lrtmp2_server_poll($server, 10)
        if ($rc -ne 0) { throw "lrtmp2_server_poll failed with $rc" }
        while ($client.Available -gt 0 -and $got -lt $want) {
            $got += $stream.Read($reply, $got, $want - $got)
        }
    }
    if ($got -lt $want) { throw "server sent $got of $want handshake bytes (S0+S1+S2)" }
    if ($reply[0] -ne 3) { throw "server answered with RTMP version $($reply[0]), expected 3" }
    $client.Close()
    for ($i = 0; $i -lt 5; $i++) { [void][Lrtmp2Smoke]::lrtmp2_server_poll($server, 10) }
    Write-Host "Server created, listened on 127.0.0.1:$Port, accepted a TCP client and answered its handshake"
} finally {
    [Lrtmp2Smoke]::lrtmp2_server_destroy($server)
}
Write-Host "librtmp2.dll smoke test passed ($Arch)"
