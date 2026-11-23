#!/usr/bin/env bash
# docker-purge-data.sh
# Elimina TODOS los datos de Docker (contenedores, imágenes, volúmenes,
# redes y caché de build) pero deja Docker instalado.
#
# Uso:
#   chmod +x docker-purge-data.sh
#   ./docker-purge-data.sh        # pide confirmación
#   ./docker-purge-data.sh -y     # sin confirmación

set -uo pipefail

if [[ $EUID -eq 0 ]]; then SUDO=""; else SUDO="sudo"; fi

if ! command -v docker >/dev/null 2>&1; then
  echo "Docker no está instalado." >&2
  exit 1
fi

if [[ "${1:-}" != "-y" ]]; then
  echo "Esto borrará TODOS los contenedores, imágenes, volúmenes, redes y caché."
  echo "Docker seguirá instalado."
  read -rp "Escribe 'si' para continuar: " resp
  [[ "$resp" == "si" ]] || { echo "Cancelado."; exit 0; }
fi

echo "[+] Eliminando contenedores..."
ids=$($SUDO docker ps -aq)
[[ -n "$ids" ]] && $SUDO docker rm -f $ids

echo "[+] Eliminando imágenes, redes, volúmenes y caché..."
$SUDO docker system prune -a --volumes -f
$SUDO docker builder prune -a -f

vols=$($SUDO docker volume ls -q)
[[ -n "$vols" ]] && $SUDO docker volume rm -f $vols

echo
echo "[+] Listo. Estado actual:"
$SUDO docker system df

# chmod +x purgar-docker.sh
# ./purgar-docker.sh        # pide confirmación
# ./purgar-docker.sh -y     # sin confirmación