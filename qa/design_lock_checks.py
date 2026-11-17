from pathlib import Path
import hashlib,json
root=Path(__file__).resolve().parents[1]
manifest=json.loads((root/'docs/UI_LOCK.json').read_text(encoding='utf-8'))
asset=root/manifest['visual_reference']
assert asset.exists(), 'Approved image missing'
assert hashlib.sha256(asset.read_bytes()).hexdigest()==manifest['visual_reference_sha256'], 'Approved image changed!'
assert manifest['default_language']=='en'
assert manifest['rtl_required'] is True
assert manifest['compression_default']=={'format':'7z','level':9,'method':'LZMA2','solid':True}
assert not manifest['design_constraints']['boxed_toolbar_buttons']
assert not manifest['design_constraints']['large_banner']
features=manifest['features']
assert len(features)==18 and len({f['id'] for f in features})==18
assert all(f['status'] in ('partial','missing','verified') for f in features)
assert all(f['callback'] and f['en'] and f['ar'] for f in features)
assert all(f['status']!='verified' for f in features), 'Runtime verification must not be assumed'
assert all(f['status']=='missing' for f in features if f['id'] in ('view','delete','find','wizard','info','repair'))
print('PASS: 12 design-contract assertions; runtime compilation not verified')
