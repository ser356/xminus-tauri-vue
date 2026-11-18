#!/usr/bin/env python3
"""Chequeo offline del nuevo HAR sin imprimir URLs con ids o secretos."""
import json
import sys
from collections import defaultdict
from pathlib import Path
from urllib.parse import urljoin, urlsplit, parse_qs

if len(sys.argv) != 2:
    raise SystemExit('Uso: python3 scripts/verify_redirects.py "captura.har"')
entries = json.loads(Path(sys.argv[1]).read_text())['log']['entries']
chains = {}
for entry in entries:
    req, resp = entry['request'], entry['response']
    url = urlsplit(req['url'])
    if url.path == '/dl/vocalCutAi':
        stem = parse_qs(url.query).get('stem', ['?'])[0]
        assert resp['status'] == 302, (stem, resp['status'])
        target = urljoin(req['url'], resp.get('redirectURL') or next(
            (v['value'] for v in resp.get('headers', []) if v['name'].lower() == 'location'), ''))
        chains[stem] = [url.path, urlsplit(target).path]
        if urlsplit(target).path == '/encode':
            match = next((next_entry for next_entry in entries
                         if next_entry['request']['url'] == target
                         and next_entry['response']['status'] == 302), None)
            assert match is not None, f'Falta segunda redirección de {stem}'
            second = urljoin(match['request']['url'], match['response'].get('redirectURL') or next(
                (h['value'] for h in match['response'].get('headers', []) if h['name'].lower() == 'location'), ''))
            chains[stem].append(urlsplit(second).path)
assert all(chains.get(stem) for stem in ('vocal', 'inst', 'drums', 'bass')), chains.keys()
assert chains['vocal'][1].startswith('/separated/')
assert chains['inst'][1].startswith('/separated/')
for stem in ('drums', 'bass'):
    assert len(chains[stem]) == 3 and chains[stem][1] == '/encode'
    assert chains[stem][2].startswith('/separated/')
for stem, chain in chains.items():
    print(f'{stem}: {len(chain)-1} redirección(es): ' + ' -> '.join(
        '/separated/[archivo].mp3' if path.startswith('/separated/') else path for path in chain))
print('OK: se han identificado las cuatro cadenas de descarga sin revelar los IDs.')
