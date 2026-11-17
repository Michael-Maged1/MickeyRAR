from pathlib import Path
import re
ROOT = Path(__file__).resolve().parents[1]
engine=(ROOT/'src/engine.rs').read_text()
main=(ROOT/'src/main.rs').read_text()
ui=(ROOT/'ui/main.slint').read_text()
checks={
 'Product title': 'title: "Mickey RAR";' in ui,
 'Ultra default': 'level: 9, solid: true, threads: None' in engine,
 'LZMA2 switch': '"-m0=LZMA2"' in engine,
 'Solid switch': '"-ms=on"' in engine and '"-ms=off"' in engine,
 'English preference': 'unwrap_or_else(|_| Language::English)' in main,
 'Arabic toggle': 'ui.on_toggle_language' in main,
 'Unsafe member path guard': 'member_path_looks_safe(name)' in main,
 'Input must exist': 'if !input.exists()' in engine,
 'Cannot overwrite input': 'if input == output' in engine,
 'No shell invocation': 'Command::new(executable)' in engine and 'sh -c' not in engine,
 'Muted charcoal UI': '#191b1c' in ui,
 'No large banner image': 'Image {' not in ui,
 'User design reference': (ROOT/'assets/approved-ui-reference.png').exists(),
 'Source unit tests': len(re.findall(r'#\[test\]', engine)) >= 12,
}
for name, okay in checks.items():
 print(f'{"PASS" if okay else "FAIL"}: {name}')
if not all(checks.values()): raise SystemExit(1)
print(f'PASS: {len(checks)} / {len(checks)} structural checks (NOT runtime tests)')
