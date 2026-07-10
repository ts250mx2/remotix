# Compila el instalador de la APP interactiva de Remotix (RemotixSetup.exe).
#
# 1) Compila remotix.exe (la app con ventana) en release con el servidor de
#    produccion "baked" (REMOTIX_DEFAULT_SERVER).
# 2) (Opcional) Firma el exe para evitar el aviso de SmartScreen.
# 3) Invoca Inno Setup (ISCC) sobre infra\installer\remotix-app.iss para generar
#    infra\installer\Output\RemotixSetup.exe.
# 4) Publica en server\public: RemotixSetup.exe (instalador, descarga principal)
#    y remotix.exe (portable, para soporte ad-hoc).
#
# Uso:
#   infra\build-app-installer.ps1
#   infra\build-app-installer.ps1 -Server wss://soporte.midominio.com -Version 1.2.0 -Sign

param(
    [string]$Server = 'wss://remotix.hlsistemas.com',
    [string]$Version = '1.0.0',
    [switch]$Sign
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$agentDir = Join-Path $root 'agent'
$iss = Join-Path $PSScriptRoot 'installer\remotix-app.iss'
$exe = Join-Path $agentDir 'target\release\remotix.exe'
$publicDir = Join-Path $root 'server\public'

# --- Localizar cargo ---
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) {
    $candidate = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path $candidate) { $cargo = $candidate }
}
if (-not $cargo) {
    Write-Host "No se encontro 'cargo'. Instala Rust desde https://rustup.rs y reintenta." -ForegroundColor Red
    exit 1
}

# --- Compilar la app ---
Write-Host "Compilando remotix.exe (servidor baked: $Server)…" -ForegroundColor Cyan
$env:REMOTIX_DEFAULT_SERVER = $Server
Push-Location $agentDir
& $cargo build --release --bin remotix
Pop-Location
if (-not (Test-Path $exe)) { Write-Host "No se genero el exe: $exe" -ForegroundColor Red; exit 1 }

if ($Sign) { & (Join-Path $PSScriptRoot 'sign.ps1') -File $exe }

# --- Localizar ISCC ---
$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    foreach ($p in @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles}\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    )) { if (Test-Path $p) { $iscc = $p; break } }
}
if (-not $iscc) {
    Write-Host "No se encontro Inno Setup (ISCC.exe). Instalalo:  winget install JRSoftware.InnoSetup" -ForegroundColor Red
    exit 1
}

# --- Compilar el instalador ---
Write-Host "Compilando el instalador (app) con Inno Setup…" -ForegroundColor Cyan
& $iscc "/DAppVersion=$Version" $iss
if ($LASTEXITCODE -ne 0) { Write-Host "ISCC fallo." -ForegroundColor Red; exit 1 }

$setup = Join-Path $PSScriptRoot 'installer\Output\RemotixSetup.exe'
if ($Sign -and (Test-Path $setup)) { & (Join-Path $PSScriptRoot 'sign.ps1') -File $setup }

# --- Publicar en server\public ---
New-Item -ItemType Directory -Force -Path $publicDir | Out-Null
Copy-Item $setup (Join-Path $publicDir 'RemotixSetup.exe') -Force   # descarga principal
Copy-Item $exe (Join-Path $publicDir 'remotix.exe') -Force          # portable (soporte ad-hoc)

# Manifiesto de versión (auto-actualización de la APP): /api/update/latest lo lee
# y la app se actualiza a este RemotixSetup.exe. Sin BOM (Node no lo tolera).
$manifest = [ordered]@{ version = $Version; url = '/download/RemotixSetup.exe'; notes = ''; mandatory = $false }
[System.IO.File]::WriteAllText(
    (Join-Path $publicDir 'remotix-latest.json'),
    ($manifest | ConvertTo-Json),
    (New-Object System.Text.UTF8Encoding($false))
)
Write-Host "Publicado en server\public: RemotixSetup.exe + remotix.exe + remotix-latest.json (v$Version)" -ForegroundColor Green

Write-Host "`nListo: $setup" -ForegroundColor Green
if (-not $Sign) {
    Write-Host "AVISO: sin firmar. SmartScreen avisara al usuario. Usa -Sign con tu certificado para produccion." -ForegroundColor Yellow
}
