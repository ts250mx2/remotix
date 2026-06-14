@echo off
REM Remotix — arranque local (doble clic). Ejecuta start.ps1 saltando la política.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0start.ps1"
pause
