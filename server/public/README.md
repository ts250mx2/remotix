# Binarios descargables

Coloca aquí el instalador/ejecutable del agente **firmado** para que los
usuarios lo descarguen desde la consola (`/ayuda` → "Descargar para control total").

Nombre esperado por la web:

- `remotix-agent.exe`  → servido en `/download/remotix-agent.exe`

Para generarlo:

```powershell
# Compilar (en una máquina con Rust):
infra\build-agent.ps1

# Firmar (con tu certificado de firma de código):
infra\sign.ps1 -File agent\target\release\remotix-agent.exe

# Copiar el .exe firmado a esta carpeta:
copy agent\target\release\remotix-agent.exe server\public\remotix-agent.exe
```

> Sin firmar, Windows SmartScreen mostrará una advertencia a los usuarios.
> Este archivo y los `.exe` están ignorados por git (ver .gitignore).
