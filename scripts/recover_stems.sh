#!/usr/bin/env bash
# Descarga pistas ya generadas en X-Minus. No sube audio ni consume una nueva separación.
# Uso: ./scripts/recover_stems.sh JOB_ID [CARPETA_DESTINO] [drums bass ...]
set -euo pipefail
if [[ $# -lt 1 ]]; then
  echo "Uso: $0 JOB_ID [CARPETA_DESTINO] [drums bass ...]" >&2
  exit 2
fi
job="$1"
if [[ ! "$job" =~ ^[a-fA-F0-9]{40,64}$ ]]; then
  echo "El job_id debe ser hexadecimal (40-64 caracteres)." >&2
  exit 2
fi
out="${2:-$HOME/Downloads}"
mkdir -p "$out"
if [[ $# -gt 2 ]]; then
  stems=("${@:3}")
else
  stems=(drums bass)
fi
for stem in "${stems[@]}"; do
  case "$stem" in
    vocal|inst|drums|bass|guitar|piano|toms|cymbals|snare|kick) ;;
    *) echo "Stem no admitido por este script: $stem" >&2; exit 2 ;;
  esac
  file="$out/xminus_${job:0:10}_${stem}.mp3"
  if [[ -e "$file" ]]; then
    echo "Ya existe $file; no se sobrescribe." >&2
    continue
  fi
  echo "Descargando $stem..."
  # -L recorre tanto /dl -> /separated como /dl -> /encode -> /separated.
  # No se envían cookies ni claves privadas.
  mime=$(curl --fail --location --max-redirs 6 --retry 2 --show-error --silent \
    --proto '=https' --proto-redir '=https' --write-out '%{content_type}' \
    --output "${file}.part" \
    "https://mmd.uvronline.vip/dl/vocalCutAi?job-id=${job}&stem=${stem}&fmt=mp3&cdn=0") || {
      rm -f "${file}.part"
      echo "No se pudo descargar $stem" >&2
      exit 1
    }
  if [[ "$mime" != audio/* && "$mime" != 'application/octet-stream' ]]; then
    rm -f "${file}.part"
    echo "El servidor devolvió un tipo de contenido inesperado: $mime" >&2
    exit 1
  fi
  if [[ $(wc -c < "${file}.part") -lt 128 ]]; then
    rm -f "${file}.part"
    echo "Descarga demasiado pequeña para $stem" >&2
    exit 1
  fi
  mv -- "${file}.part" "$file"
  echo "Guardado: $file"
done
