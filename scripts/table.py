#!/usr/bin/env python3
"""Markdown tables from a bench CSV: one row per class, one column per codec,
cells "ratio / comp us / decomp us" (sum(compressed)/sum(raw); mean us per frame).
Usage: table.py <csv> <codec,codec,...> [classes,...]"""
import csv, sys, collections

rows = list(csv.DictReader(open(sys.argv[1])))
codecs = sys.argv[2].split(',')
classes = sys.argv[3].split(',') if len(sys.argv) > 3 else ['chat', 'ctrl', 'markdown', 'ws-json', 'yjs-inc', 'yjs-inc-legacy', 'yjs-snap', 'yjs-snap-legacy', 'file-chunk']
acc = collections.defaultdict(lambda: [0, 0, 0.0, 0.0, 0, True])
for r in rows:
    a = acc[(r['class'], r['codec'])]
    a[0] += int(r['raw']); a[1] += int(r['compressed']); a[2] += float(r['comp_us']); a[3] += float(r['decomp_us']); a[4] += 1
    a[5] &= r['roundtrip_ok'] == 'true'
print('| class (n) | ' + ' | '.join(codecs) + ' |')
print('|---|' + '---|' * len(codecs))
for c in classes:
    n = next((acc[(c, k)][4] for k in codecs if acc[(c, k)][4]), 0)
    if not n:
        continue
    cells = []
    for k in codecs:
        raw, comp, cus, dus, m, ok = acc[(c, k)]
        cells.append('—' if not m else f'{comp / raw:.3f} / {cus / m:.1f} / {dus / m:.2f}' + ('' if ok else ' (FAIL)'))
    print(f'| {c} ({n}) | ' + ' | '.join(cells) + ' |')
