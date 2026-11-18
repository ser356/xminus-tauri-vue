#!/usr/bin/env python3
"""Comprueba el catálogo integrado sin acceder a red ni leer HAR privados."""
import json
from pathlib import Path

catalog = json.loads((Path(__file__).parents[1] / 'src' / 'catalog.json').read_text())
presets = catalog['modes']
assert len(presets) == len({p['id'] for p in presets})
models = [m for p in presets for m in p['models']]
assert len(models) == 69, f"Entradas no previstas: {len(models)}"
assert sum(not m['is_test'] for m in models) == 45
for preset in presets:
    seen = set()
    for model in preset['models']:
        assert model['id'] not in seen, (preset['id'], model['id'])
        seen.add(model['id'])
        assert model['backend'] and model['stems']
        assert model['access_level'] in (0, 1, 2, 3)
        if preset['group'] in ('2', '4', '6'):
            assert len(model['stems']) == int(preset['group']), preset['id']

def model(preset_id: str, model_id: str) -> dict:
    p = next(x for x in presets if x['id'] == preset_id)
    return next(x for x in p['models'] if x['id'] == model_id)

assert model('vocals_music', 'mel_roformer_deux_becruily')['stems'] == ['vocal', 'inst']
assert model('splitter_4', 'htdemucs')['stems'] == ['vocal', 'inst', 'drums', 'bass']
assert model('splitter_4', 'htdemucs_drumsep')['stems'] == ['toms', 'cymbals', 'snare', 'kick']
assert model('splitter_6', 'bs_roformer_sw_bdovgp')['stems'] == ['vocal', 'inst', 'drums', 'bass', 'guitar', 'piano']
assert model('bv_other', 'mel_roformer_lv_bv_chain1')['stems'] == ['inst', 'lv', 'bv']
print(f"OK: {len(presets)} tipos, {len(models)} entradas ({45} publicadas, {24} de prueba)")
