# Remotix — arranque local "todo automático".
# Instala dependencias (si faltan), compila la consola web y arranca el servidor
# (API + señalización + consola) en un solo puerto. Luego abre el navegador.
#
# Uso:   powershell -ExecutionPolicy Bypass -File start.ps1
#   o doble clic en start.bat

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$port = if ($env:PORT) { $env:PORT } else { '8080' }

function Step($msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }

# 1) Comprobar Node.
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Host "Node.js no está instalado. Instálalo desde https://nodejs.org (LTS) y reintenta." -ForegroundColor Red
    exit 1
}
Write-Host "Node $(node -v) detectado."

# 2) Dependencias.
Step "Instalando dependencias (si faltan)"
if (-not (Test-Path "$root\server\node_modules")) { Push-Location "$root\server"; npm install; Pop-Location }
if (-not (Test-Path "$root\web\node_modules"))    { Push-Location "$root\web";    npm install; Pop-Location }

# 3) Compilar la consola web (la sirve el servidor).
Step "Compilando la consola web"
Push-Location "$root\web"; npm run build; Pop-Location

# 4) Compilar el servidor.
Step "Compilando el servidor"
Push-Location "$root\server"; npm run build; Pop-Location

# 5) Arrancar.
$dataDir = "$root\server"
$env:REMOTIX_DB = "$dataDir\remotix.db"
$urlOperador = "http://localhost:$port/operador"
$urlAyuda    = "http://localhost:$port/ayuda"

Step "Arrancando Remotix en el puerto $port"
Write-Host ""
Write-Host "  Consola del técnico:   $urlOperador" -ForegroundColor Green
Write-Host "  Página del usuario:    $urlAyuda" -ForegroundColor Green
Write-Host "  Portal (cuentas):      http://localhost:$port/" -ForegroundColor Green
Write-Host ""
Write-Host "  (Ctrl+C para detener)" -ForegroundColor DarkGray
Write-Host ""

# Abrir el navegador en la consola del técnico tras un breve arranque.
Start-Job -ScriptBlock { Start-Sleep -Seconds 2; Start-Process $using:urlOperador } | Out-Null

Push-Location "$root\server"
node dist/index.js
Pop-Location
