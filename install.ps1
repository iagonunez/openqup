# oqman bootstrap for Windows (PowerShell 5.1 or later).
#
#   powershell -ExecutionPolicy Bypass -c "irm https://github.com/iagonunez/oqman/releases/latest/download/install.ps1 | iex"
#   & ([scriptblock]::Create((irm https://github.com/iagonunez/oqman/releases/latest/download/install.ps1))) --ref v0.2.0
#
# Downloads oqman.exe from the latest oqman release, checks it against the release's
# SHA256SUMS and its Authenticode signature, puts it in %LOCALAPPDATA%\oqman\bin (added to your
# user PATH) and runs `oqman install`, which builds and installs OpenQ.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest is much faster without the bar
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$repo = if ($env:OQMAN_REPO) { $env:OQMAN_REPO } else { 'iagonunez/oqman' }
$base = if ($env:OQMAN_DOWNLOAD) { $env:OQMAN_DOWNLOAD } else { "https://github.com/$repo/releases/latest/download" }

if (-not [Environment]::Is64BitOperatingSystem) { throw 'oqman needs 64-bit Windows.' }
if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64' -or $env:PROCESSOR_ARCHITEW6432 -eq 'ARM64') {
  throw 'Windows on Arm is not supported yet.'
}
$target = 'x86_64-pc-windows-msvc'
$name = "oqman-$target.exe"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("oqman-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  Write-Host "oqman: downloading $name"
  Invoke-WebRequest -UseBasicParsing "$base/$name" -OutFile "$tmp\oqman.exe"
  Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS" -OutFile "$tmp\SHA256SUMS"
  $want = (Get-Content "$tmp\SHA256SUMS" | Where-Object { ($_ -split '\s+')[1] -in @($name, "*$name") } | Select-Object -First 1)
  if (-not $want) { throw "$name isn't listed in SHA256SUMS" }
  $want = ($want -split '\s+')[0].ToLower()
  $got = (Get-FileHash -Algorithm SHA256 "$tmp\oqman.exe").Hash.ToLower()
  if ($want -ne $got) { throw "checksum mismatch for $name (expected $want, got $got)" }

  $sig = Get-AuthenticodeSignature -LiteralPath "$tmp\oqman.exe"
  if ($sig.Status -eq 'Valid') {
    Write-Host "oqman: signed by $($sig.SignerCertificate.GetNameInfo('SimpleName', $false))"
  } elseif ($sig.Status -eq 'NotSigned') {
    Write-Host 'oqman: this build is not code-signed (checksum verified)'
  } else {
    throw "oqman.exe has an invalid signature ($($sig.Status))"
  }

  $bin = Join-Path $env:LOCALAPPDATA 'oqman\bin'
  New-Item -ItemType Directory -Force -Path $bin | Out-Null
  $exe = Join-Path $bin 'oqman.exe'
  Move-Item -Force "$tmp\oqman.exe" $exe
  Write-Host "oqman: installed $exe"

  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  if (-not (($userPath -split ';') -contains $bin)) {
    [Environment]::SetEnvironmentVariable('Path', ((@($userPath, $bin) | Where-Object { $_ }) -join ';'), 'User')
    $env:Path = "$env:Path;$bin"
    Write-Host "oqman: added $bin to your PATH (new terminals)"
  }
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

& $exe install @args
exit $LASTEXITCODE
