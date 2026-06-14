# Firma un ejecutable/instalador con Authenticode (signtool).
#
# Eliminar las advertencias de SmartScreen para usuarios NO técnicos requiere un
# certificado de firma de código (idealmente EV). Este script automatiza la firma
# una vez que tengas el certificado; no puede generarlo por ti.
#
# Uso (certificado en el almacén de Windows, por huella):
#   infra\sign.ps1 -File agent\target\release\remotix-agent.exe -Thumbprint AA11BB22...
#
# Uso (archivo .pfx):
#   infra\sign.ps1 -File ... -Pfx C:\ruta\cert.pfx -Password (Read-Host -AsSecureString)

param(
    [Parameter(Mandatory = $true)][string]$File,
    [string]$Thumbprint = $env:REMOTIX_CERT_THUMBPRINT,
    [string]$Pfx = $env:REMOTIX_CERT_PFX,
    [System.Security.SecureString]$Password,
    [string]$TimestampUrl = 'http://timestamp.digicert.com'
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $File)) {
    Write-Host "No existe el archivo: $File" -ForegroundColor Red; exit 1
}

# Localizar signtool.exe del Windows SDK.
$signtool = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match 'x64' } | Select-Object -Last 1
if (-not $signtool) {
    Write-Host "No se encontró signtool.exe. Instala el Windows SDK." -ForegroundColor Red; exit 1
}
$signtool = $signtool.FullName

if (-not $Thumbprint -and -not $Pfx) {
    Write-Host @"
No se indicó certificado. Pasos para firmar (necesitas un certificado de firma de código):
  - Por almacén:  infra\sign.ps1 -File "$File" -Thumbprint <HUELLA>
  - Por .pfx:     infra\sign.ps1 -File "$File" -Pfx <ruta.pfx> -Password (Read-Host -AsSecureString)
Recomendado: certificado EV para que SmartScreen no advierta de inmediato.
"@ -ForegroundColor Yellow
    exit 2
}

$common = @('sign', '/fd', 'sha256', '/tr', $TimestampUrl, '/td', 'sha256', '/v')

if ($Pfx) {
    if (-not $Password) { $Password = Read-Host -AsSecureString "Contraseña del .pfx" }
    $plain = [Runtime.InteropServices.Marshal]::PtrToStringAuto(
        [Runtime.InteropServices.Marshal]::SecureStringToBSTR($Password))
    & $signtool @common '/f' $Pfx '/p' $plain $File
} else {
    & $signtool @common '/sha1' $Thumbprint $File
}

if ($LASTEXITCODE -ne 0) { Write-Host "Fallo al firmar." -ForegroundColor Red; exit $LASTEXITCODE }
& $signtool 'verify' '/pa' '/v' $File
Write-Host "Firmado correctamente: $File" -ForegroundColor Green
