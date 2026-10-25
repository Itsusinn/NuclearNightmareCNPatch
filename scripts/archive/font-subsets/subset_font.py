import csv
import hashlib
import json
import unicodedata
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw, ImageFont

project = Path(__file__).resolve().parents[3]
root = project / 'target' / 'font-subsets'
root.mkdir(parents=True, exist_ok=True)
source = project / 'fonts' / 'SarasaGothicSC-Regular.ttf'
translations = project / 'translations.csv'
source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
original = TTFont(source)
original_cmap = original.getBestCmap()
with translations.open(encoding='utf-8-sig', newline='') as handle:
    rows = list(csv.DictReader(handle))
text = ''.join(row['text'] + row['trans_text'] for row in rows)
required = {ord(c) for c in text if not unicodedata.category(c).startswith('C')}
missing = sorted(required - original_cmap.keys())
if missing:
    raise RuntimeError(f'Source font cannot render corpus: {missing}')
ui = required | set(range(0x20, 0x100))
common = set(ui)
for start, end in [(0x100, 0x530), (0x2000, 0x2070), (0x2190, 0x2300),
                   (0x25A0, 0x2600), (0x3000, 0x3040), (0xFF00, 0xFFF0)]:
    common.update(range(start, end))
gb2312 = set()
for lead in range(0xA1, 0xF8):
    for trail in range(0xA1, 0xFF):
        try:
            gb2312.update(map(ord, bytes([lead, trail]).decode('gb2312')))
        except UnicodeDecodeError:
            pass
common.update(gb2312)
report = {'source': str(source), 'source_bytes': source.stat().st_size,
          'source_sha256': source_hash, 'original_glyphs': original['maxp'].numGlyphs,
          'original_unicode': len(original_cmap), 'corpus_rows': len(rows),
          'corpus_characters': len(required), 'variants': []}
for key, desired, family in [('UIText', ui, 'NN Chinese UI Subset'),
                              ('GB2312', common, 'NN Chinese Common Subset')]:
    desired &= original_cmap.keys()
    font = TTFont(source, recalcTimestamp=False)
    options = subset.Options()
    options.layout_features = ['*']
    options.name_IDs = ['*']
    options.name_languages = ['*']
    options.name_legacy = True
    options.notdef_glyph = True
    options.notdef_outline = True
    options.recommended_glyphs = True
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(unicodes=desired)
    subsetter.subset(font)
    # Identify these modified fonts separately; preserve original copyright.
    rename_ids = {1, 2, 3, 4, 6, 16, 17, 18, 21, 22, 25}
    font['name'].names = [n for n in font['name'].names if n.nameID not in rename_ids]
    names = {1: family, 2: 'Regular', 3: f'{family}:1.0.42:Regular',
             4: family + ' Regular', 6: f'NNChinese-{key}-Regular',
             16: family, 17: 'Regular',
             13: 'Licensed under the SIL Open Font License, Version 1.1. See OFL.txt.',
             14: 'https://openfontlicense.org/open-font-license-official-text/'}
    for name_id, value in names.items():
        for platform, encoding, language in [(3, 1, 0x409), (1, 0, 0)]:
            font['name'].setName(value, name_id, platform, encoding, language)
    output = root / f'NNChinese-{key}-Regular.ttf'
    font.save(output)
    font.close()
    verified = TTFont(output, checkChecksums=2)
    for tag in verified.reader.keys():
        verified[tag]
    cmap = verified.getBestCmap()
    assert desired <= cmap.keys()
    assert required <= cmap.keys()
    assert verified['head'].unitsPerEm == original['head'].unitsPerEm
    for cp in desired:
        assert verified['hmtx'].metrics[cmap[cp]] == original['hmtx'].metrics[original_cmap[cp]]
    report['variants'].append({'file': output.name, 'bytes': output.stat().st_size,
                              'glyphs': verified['maxp'].numGlyphs, 'unicode': len(cmap),
                              'reduction_percent': round(100 * (1-output.stat().st_size/source.stat().st_size), 2),
                              'missing_corpus_characters': [],
                              'sha256': hashlib.sha256(output.read_bytes()).hexdigest()})
    verified.close()

# Compare the original and subset raster output on actual corpus characters.
sample = ''.join(dict.fromkeys(c for c in text if ord(c) >= 0x4E00 and ord(c) <= 0x9FFF))[:48]
sample_lines = [sample[:24], sample[24:48], 'ABC xyz 0123456789 + - / % {M} {State}']
specimen = Image.new('RGB', (1400, 630), '#f7f7f7')
draw = ImageDraw.Draw(specimen)
reference = None
for index, (label, font_path) in enumerate([('Original Sarasa Gothic SC', source)] +
        [(v['file'], root / v['file']) for v in report['variants']]):
    face = ImageFont.truetype(str(font_path), 40)
    block = Image.new('L', (1380, 150), 255)
    block_draw = ImageDraw.Draw(block)
    for line, value in enumerate(sample_lines):
        block_draw.text((10, line * 48), value, font=face, fill=0)
    if reference is None:
        reference = block.tobytes()
    else:
        assert block.tobytes() == reference, 'Raster output differs from original'
    draw.text((20, index * 210 + 12), label, fill='#333333')
    specimen.paste(block.convert('RGB'), (10, index * 210 + 42))
specimen.save(root / 'comparison.png')
report['sample_raster_matches_original'] = True
assert hashlib.sha256(source.read_bytes()).hexdigest() == source_hash
(root / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
print(json.dumps(report, ensure_ascii=True, indent=2))
