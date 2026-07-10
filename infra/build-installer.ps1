# Compila el instalador del HOST desatendido de Remotix (RemotixHostSetup.exe).
# Es el servicio de Windows que arranca en el boot; es tambien el destino de la
# auto-actualizacion. Para la app interactiva usa build-app-installer.ps1.
#
# 1) Compila el agente (remotix-lite) en release con el servidor de produccion
#    "baked" dentro del exe (REMOTIX_DEFAULT_SERVER).
# 2) (Opcional) Firma el exe para evitar el aviso de SmartScreen.
# 3) Invoca el compilador de Inno Setup (ISCC.exe) sobre infra\installer\remotix.iss
#    para generar infra\installer\Output\RemotixHostSetup.exe.
#
# Uso:
#   infra\build-installer.ps1                                          # servidor por defecto (produccion)
#   infra\build-installer.ps1 -Server wss://soporte.midominio.com      # otro servidor baked
#   infra\build-installer.ps1 -Version 1.2.0 -Sign                     # version + firma (exe e instalador)
#
# Requisitos:
#   - Rust (https://rustup.rs) con target MSVC.
#   - Inno Setup 6 (https://jrsoftware.org/isdl.php  o  winget install JRSoftware.InnoSetup).

param(
    [string]$Server = 'wss://remotix.hlsistemas.com',
    [string]$Version = '1.0.0',
    [switch]$Sign
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$agentDir = Join-Path $root 'agent'
$iss = Join-Path $PSScriptRoot 'installer\remotix.iss'
$exe = Join-Path $agentDir 'target\release\remotix-lite.exe'

# --- 1) Localizar cargo ---
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) {
    $candidate = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path $candidate) { $cargo = $candidate }
}
if (-not $cargo) {
    Write-Host "No se encontro 'cargo'. Instala Rust desde https://rustup.rs y reintenta." -ForegroundColor Red
    exit 1
}

# --- 2) Compilar el exe con el servidor baked ---
Write-Host "Compilando remotix-lite (servidor baked: $Server)…" -ForegroundColor Cyan
$env:REMOTIX_DEFAULT_SERVER = $Server
Push-Location $agentDir
& $cargo build --release --bin remotix-lite
Pop-Location
if (-not (Test-Path $exe)) {
    Write-Host "No se genero el exe: $exe" -ForegroundColor Red
    exit 1
}

# --- 3) Firmar el exe (opcional) ---
if ($Sign) {
    & (Join-Path $PSScriptRoot 'sign.ps1') -File $exe
}

# --- 4) Localizar ISCC (compilador de Inno Setup) ---
$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    foreach ($p in @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles}\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    )) {
        if (Test-Path $p) { $iscc = $p; break }
    }
}
if (-not $iscc) {
    Write-Host "No se encontro Inno Setup (ISCC.exe)." -ForegroundColor Red
    Write-Host "Instalalo con:  winget install JRSoftware.InnoSetup" -ForegroundColor Yellow
    Write-Host "  o descargalo de  https://jrsoftware.org/isdl.php" -ForegroundColor Yellow
    exit 1
}

# --- 5) Compilar el instalador ---
Write-Host "Compilando el instalador con Inno Setup…" -ForegroundColor Cyan
& $iscc "/DAppVersion=$Version" $iss
if ($LASTEXITCODE -ne 0) { Write-Host "ISCC fallo." -ForegroundColor Red; exit 1 }

$setup = Join-Path $PSScriptRoot 'installer\Output\RemotixHostSetup.exe'
if ($Sign -and (Test-Path $setup)) {
    & (Join-Path $PSScriptRoot 'sign.ps1') -File $setup
}

# --- 6) Publicar el instalador del host + su manifiesto (canal host) ---
# El canal de auto-actualización del HOST es remotix-host-latest.json (lo lee el
# servicio vía /api/update/latest?channel=host). Es un manifiesto SEPARADO del
# de la app (remotix-latest.json, que escribe build-app-installer.ps1) para que
# un host jamás se "actualice" con el instalador de la app ni al revés.
$publicDir = Join-Path $root 'server\public'
New-Item -ItemType Directory -Force -Path $publicDir | Out-Null
Copy-Item $setup (Join-Path $publicDir 'RemotixHostSetup.exe') -Force
$manifest = [ordered]@{ version = $Version; url = '/download/RemotixHostSetup.exe'; notes = ''; mandatory = $false }
# UTF-8 SIN BOM (Out-File -Encoding utf8 de PS 5.1 mete BOM y rompe JSON.parse en Node).
[System.IO.File]::WriteAllText(
    (Join-Path $publicDir 'remotix-host-latest.json'),
    ($manifest | ConvertTo-Json),
    (New-Object System.Text.UTF8Encoding($false))
)
Write-Host "Publicado en server\public: RemotixHostSetup.exe + remotix-host-latest.json (v$Version)" -ForegroundColor Green

Write-Host "`nListo: $setup" -ForegroundColor Green
Write-Host "Instalacion desatendida en el equipo remoto:  RemotixHostSetup.exe /VERYSILENT /SUPPRESSMSGBOXES" -ForegroundColor Green
if (-not $Sign) {
    Write-Host "AVISO: sin firmar. SmartScreen avisara al usuario. Usa -Sign con tu certificado para produccion." -ForegroundColor Yellow
}
