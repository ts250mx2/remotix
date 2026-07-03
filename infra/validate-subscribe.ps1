# Valida de punta a punta el flujo "suscribir una PC por su clave" (multiusuario).
#
# Levanta el servidor en un puerto aparte (8090) contra tu MySQL, y comprueba:
#   1. Se registra un dispositivo -> clave fija.
#   2. Usuario A la reclama por clave -> queda como DUEÑO.
#   3. Usuario B la suscribe con la MISMA clave -> queda como COMPARTIDO.
#      (=> una PC en dos cuentas a la vez)
#   4. B se da de baja (DELETE /:id/subscription) -> desaparece solo para B.
#   5. A (dueño) la elimina (DELETE /:id) -> desaparece del todo.
# Al final PARA el servidor y BORRA los datos de prueba (device + 2 usuarios).
#
# Uso (PowerShell):
#   powershell -ExecutionPolicy Bypass -File infra\validate-subscribe.ps1 -MysqlPassword 'TU_PASSWORD'
#
# Nada persiste: emails/máquina llevan un sello único y se borran al terminar.
# En el primer arranque aplicará la migración idempotente 'agent_version' a tu BD.

param(
    [Parameter(Mandatory = $true)][string]$MysqlPassword,
    [int]$Port = 8090,
    [string]$MysqlUser = 'remotix',
    [string]$MysqlDatabase = 'remotix',
    [string]$MysqlExe = 'C:\Program Files\MySQL\MySQL Server 8.0\bin\mysql.exe'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$serverDir = Join-Path $root 'server'
$base = "http://localhost:$Port"

$stamp = Get-Date -Format 'yyyyMMddHHmmss'
$emailA = "test-a-$stamp@remotix.test"
$emailB = "test-b-$stamp@remotix.test"
$pcName = "PC-Test-$stamp"
$machineId = "test-machine-$stamp"

$script:pass = 0
$script:fail = 0
function Check($name, $cond) {
    if ($cond) { Write-Host "  [PASS] $name" -ForegroundColor Green; $script:pass++ }
    else { Write-Host "  [FAIL] $name" -ForegroundColor Red; $script:fail++ }
}

Write-Host "==> Arrancando el servidor en el puerto $Port (BD: $MysqlDatabase)…" -ForegroundColor Cyan
$job = Start-Job -ScriptBlock {
    param($dir, $port, $user, $pass, $db)
    Set-Location $dir
    $env:MYSQL_HOST = '127.0.0.1'; $env:MYSQL_PORT = '3306'
    $env:MYSQL_USER = $user; $env:MYSQL_PASSWORD = $pass; $env:MYSQL_DATABASE = $db
    $env:PORT = $port; $env:SESSION_SECRET = 'dev-only-change-me'; $env:NODE_ENV = 'development'
    npx tsx src/index.ts *>&1
} -ArgumentList $serverDir, "$Port", $MysqlUser, $MysqlPassword, $MysqlDatabase

try {
    $up = $false
    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 700
        try { if ((Invoke-RestMethod "$base/health" -TimeoutSec 2).ok) { $up = $true; break } } catch {}
    }
    if (-not $up) {
        Write-Host "El servidor no respondió. Últimas líneas del log:" -ForegroundColor Red
        Receive-Job $job | Select-Object -Last 25
        return
    }
    Write-Host "Servidor arriba." -ForegroundColor Green
    Write-Host "`n==> Prueba de suscripción por clave" -ForegroundColor Cyan

    # 1) Registrar dispositivo (endpoint público del agente).
    $dev = Invoke-RestMethod "$base/api/device/register" -Method Post -ContentType 'application/json' `
        -Body (@{ name = $pcName; machineId = $machineId } | ConvertTo-Json)
    $key = $dev.accessKey; $deviceId = $dev.deviceId
    Check "Dispositivo registrado (clave $key)" ($key -and $deviceId)

    # 2) Usuario A: registro (auto-login) + reclamar por clave => dueño.
    Invoke-RestMethod "$base/api/auth/register" -Method Post -ContentType 'application/json' `
        -Body (@{ email = $emailA; password = 'Passw0rd!'; name = 'Usuario A' } | ConvertTo-Json) `
        -SessionVariable sessA | Out-Null
    Invoke-RestMethod "$base/api/devices/claim" -Method Post -ContentType 'application/json' `
        -Body (@{ accessKey = $key } | ConvertTo-Json) -WebSession $sessA | Out-Null
    $devA = (Invoke-RestMethod "$base/api/devices" -WebSession $sessA).devices | Where-Object { $_.id -eq $deviceId }
    Check "A ve la PC como 'dueño'" ($devA -and $devA.role -eq 'owner')

    # 3) Usuario B: se suscribe con la MISMA clave => compartido.
    Invoke-RestMethod "$base/api/auth/register" -Method Post -ContentType 'application/json' `
        -Body (@{ email = $emailB; password = 'Passw0rd!'; name = 'Usuario B' } | ConvertTo-Json) `
        -SessionVariable sessB | Out-Null
    Invoke-RestMethod "$base/api/devices/claim" -Method Post -ContentType 'application/json' `
        -Body (@{ accessKey = $key } | ConvertTo-Json) -WebSession $sessB | Out-Null
    $devB = (Invoke-RestMethod "$base/api/devices" -WebSession $sessB).devices | Where-Object { $_.id -eq $deviceId }
    Check "B ve la MISMA PC como 'compartido'" ($devB -and $devB.role -eq 'granted')
    Check "La PC está en las dos cuentas a la vez" ($devA -and $devB)

    # 4) B se da de baja (no afecta al dueño).
    Invoke-RestMethod "$base/api/devices/$deviceId/subscription" -Method Delete -WebSession $sessB | Out-Null
    $goneB = (Invoke-RestMethod "$base/api/devices" -WebSession $sessB).devices | Where-Object { $_.id -eq $deviceId }
    Check "Tras darse de baja, B ya no la ve" (-not $goneB)
    $stillA = (Invoke-RestMethod "$base/api/devices" -WebSession $sessA).devices | Where-Object { $_.id -eq $deviceId }
    Check "A la sigue viendo (baja de B no le afecta)" ($stillA)

    # 5) A (dueño) elimina la PC.
    Invoke-RestMethod "$base/api/devices/$deviceId" -Method Delete -WebSession $sessA | Out-Null
    $goneA = (Invoke-RestMethod "$base/api/devices" -WebSession $sessA).devices | Where-Object { $_.id -eq $deviceId }
    Check "Tras eliminar (dueño), desaparece" (-not $goneA)

    $color = if ($script:fail -eq 0) { 'Green' } else { 'Red' }
    Write-Host "`nResultado: $($script:pass) OK / $($script:fail) fallos" -ForegroundColor $color
}
finally {
    Write-Host "`n==> Limpiando (parar servidor + borrar datos de prueba)…" -ForegroundColor Cyan
    Stop-Job $job -ErrorAction SilentlyContinue
    Remove-Job $job -Force -ErrorAction SilentlyContinue
    if (Test-Path $MysqlExe) {
        $sql = "DELETE FROM devices WHERE machine_id='$machineId'; DELETE FROM users WHERE email IN ('$emailA','$emailB');"
        & $MysqlExe "-u$MysqlUser" "-p$MysqlPassword" -h 127.0.0.1 $MysqlDatabase -e $sql 2>$null
        Write-Host "Datos de prueba borrados (device $machineId + usuarios test-*-$stamp)." -ForegroundColor Green
    }
    else {
        Write-Host "No encontré mysql.exe; borra a mano el device '$machineId' y los usuarios test-*-$stamp@remotix.test." -ForegroundColor Yellow
    }
}
