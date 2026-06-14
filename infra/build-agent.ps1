# Compila el agente nativo de Windows (release) y lo deja listo para descargar.
#
# Uso:
#   infra\build-agent.ps1                                  # default -> ws://localhost:8080
#   infra\build-agent.ps1 -Server https://soporte.midominio.com   # baked para produccion
#   infra\build-agent.ps1 -Server https://... -Sign        # compila y firma (necesita certificado)
#
# Requiere el toolchain de Rust (https://rustup.rs) con el target MSVC.

param(
    [string]$Server = '',
    [switch]$Sign
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$agentDir = Join-Path $root 'agent'
$publicDir = Join-Path $root 'server\public'

# Localizar cargo (puede no estar en PATH si Rust se instalo en esta sesion).
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) {
    $candidate = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path $candidate) { $cargo = $candidate }
}
if (-not $cargo) {
    Write-Host "No se encontro 'cargo'. Instala Rust desde https://rustup.rs y reintenta." -ForegroundColor Red
    exit 1
}

if ($Server) {
    Write-Host "Compilando con servidor baked: $Server" -ForegroundColor Cyan
    $env:REMOTIX_DEFAULT_SERVER = $Server
} else {
    Write-Host "Compilando con servidor por defecto (ws://localhost:8080)." -ForegroundColor Yellow
    Remove-Item Env:\REMOTIX_DEFAULT_SERVER -ErrorAction SilentlyContinue
}

Push-Location $agentDir
# Exe unico cliente/servidor estilo TeamViewer (host siempre + login + visor nativo).
& $cargo build --release --bin remotix
Pop-Location

$bins = @(
    @{ exe = 'remotix.exe'; desc = 'Remotix (cliente/servidor unificado: aceptar y conectar)' }
)
New-Item -ItemType Directory -Force -Path $publicDir | Out-Null
foreach ($b in $bins) {
    $exe = Join-Path $agentDir "target\release\$($b.exe)"
    if (-not (Test-Path $exe)) { Write-Host "No se genero: $exe" -ForegroundColor Red; exit 1 }
    if ($Sign) { & (Join-Path $PSScriptRoot 'sign.ps1') -File $exe }
    Copy-Item $exe (Join-Path $publicDir $b.exe) -Force
    Write-Host "Listo: $publicDir\$($b.exe)  - $($b.desc)" -ForegroundColor Green
}
Write-Host "`nDescarga: /download/remotix.exe (un solo programa para todos)." -ForegroundColor Green
if (-not $Sign) {
    Write-Host "AVISO: sin firmar. Windows SmartScreen advertira a los usuarios. Usa -Sign con tu certificado antes de produccion." -ForegroundColor Yellow
}
