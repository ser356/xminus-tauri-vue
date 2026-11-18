#!/usr/bin/env python3
"""Verifica que un HAR tenga los hitos del protocolo sin imprimir secretos."""
import json
import sys
from urllib.parse import urlparse, parse_qs
from pathlib import Path


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("Uso: python3 scripts/verify_har.py archivo.har")
    data = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    entries = data["log"]["entries"]
    rows = []
    for e in entries:
        req, resp = e["request"], e["response"]
        url = urlparse(req["url"])
        if url.path in ("/upload/vocalCutAi", "/dl/vocalCutAi"):
            text = resp.get("content", {}).get("text", "")
            try:
                obj = json.loads(text)
                st = obj.get("status", "")
            except (ValueError, AttributeError):
                st = ""
            rows.append((req["method"], url.hostname, url.path,
                         ",".join(parse_qs(url.query)), resp["status"], st))
    print("method,host,ruta,claves_query,http,estado")
    for row in rows:
        print(",".join(map(str, row)))
    assert any(r[0] == "POST" and r[2] == "/upload/vocalCutAi" and r[5] == "accepted" for r in rows), "Falta subida aceptada"
    assert any(r[0] == "POST" and r[5] == "done" for r in rows), "Falta estado done"
    assert any(r[0] == "GET" and r[2] == "/dl/vocalCutAi" and r[4] == 302 for r in rows), "Falta redirección de descarga"
    print("Contrato mínimo encontrado; no implica que la API siga disponible.")


if __name__ == "__main__":
    main()
