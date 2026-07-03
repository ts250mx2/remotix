// Fuente ÚNICA de verdad de la descarga: el mismo instalador antes y después de
// iniciar sesión. Cambiar aquí lo cambia en toda la app.
export const REMOTIX_DOWNLOAD = '/download/RemotixSetup.exe';

export function DownloadButton({
  label = 'Descargar Remotix para Windows',
  className = 'download-btn',
}: {
  label?: string;
  className?: string;
}) {
  return (
    <a className={className} href={REMOTIX_DOWNLOAD} download>
      ⬇ {label}
    </a>
  );
}
