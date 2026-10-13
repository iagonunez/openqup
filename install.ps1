# openqup bootstrap for Windows (PowerShell 5.1 or later).
#
#   powershell -ExecutionPolicy Bypass -c "irm https://github.com/iagonunez/openqup/releases/latest/download/install.ps1 | iex"
#   & ([scriptblock]::Create((irm https://github.com/iagonunez/openqup/releases/latest/download/install.ps1))) --ref v0.2.0
#
# Downloads openqup.exe from the latest openqup release, checks it against the release's
# SHA256SUMS and its Authenticode signature, puts it in %LOCALAPPDATA%\openqup\bin (added to your
# user PATH) and runs `openqup install`, which builds and installs OpenQ.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest is much faster without the bar
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$repo = if ($env:OPENQUP_REPO) { $env:OPENQUP_REPO } else { 'iagonunez/openqup' }
$base = if ($env:OPENQUP_DOWNLOAD) { $env:OPENQUP_DOWNLOAD } else { "https://github.com/$repo/releases/latest/download" }

if (-not [Environment]::Is64BitOperatingSystem) { throw 'openqup needs 64-bit Windows.' }
if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64' -or $env:PROCESSOR_ARCHITEW6432 -eq 'ARM64') {
  throw 'Windows on Arm is not supported yet.'
}
$target = 'x86_64-pc-windows-msvc'
$name = "openqup-$target.exe"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("openqup-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  Write-Host "openqup: downloading $name"
  Invoke-WebRequest -UseBasicParsing "$base/$name" -OutFile "$tmp\openqup.exe"
  Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS" -OutFile "$tmp\SHA256SUMS"
  $want = (Get-Content "$tmp\SHA256SUMS" | Where-Object { ($_ -split '\s+')[1] -in @($name, "*$name") } | Select-Object -First 1)
  if (-not $want) { throw "$name isn't listed in SHA256SUMS" }
  $want = ($want -split '\s+')[0].ToLower()
  $got = (Get-FileHash -Algorithm SHA256 "$tmp\openqup.exe").Hash.ToLower()
  if ($want -ne $got) { throw "checksum mismatch for $name (expected $want, got $got)" }

  $sig = Get-AuthenticodeSignature -LiteralPath "$tmp\openqup.exe"
  if ($sig.Status -eq 'Valid') {
    Write-Host "openqup: signed by $($sig.SignerCertificate.GetNameInfo('SimpleName', $false))"
  } elseif ($sig.Status -eq 'NotSigned') {
    Write-Host 'openqup: this build is not code-signed (checksum verified)'
  } else {
    throw "openqup.exe has an invalid signature ($($sig.Status))"
  }

  $bin = Join-Path $env:LOCALAPPDATA 'openqup\bin'
  New-Item -ItemType Directory -Force -Path $bin | Out-Null
  $exe = Join-Path $bin 'openqup.exe'
  Move-Item -Force "$tmp\openqup.exe" $exe
  Write-Host "openqup: installed $exe"

  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  if (-not (($userPath -split ';') -contains $bin)) {
    [Environment]::SetEnvironmentVariable('Path', ((@($userPath, $bin) | Where-Object { $_ }) -join ';'), 'User')
    $env:Path = "$env:Path;$bin"
    Write-Host "openqup: added $bin to your PATH (new terminals)"
  }
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

& $exe install @args
exit $LASTEXITCODE
