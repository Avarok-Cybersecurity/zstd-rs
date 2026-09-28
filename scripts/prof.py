#!/usr/bin/env python3
"""Summarise a samply profile (recorded with --unstable-presymbolicate): self time
per function, and per source line (via atos, including inlined frames).
Usage: prof.py <profile.json> <binary> [top_n]"""
import json, subprocess, sys, collections, bisect

prof = json.load(open(sys.argv[1]))
syms = json.load(open(sys.argv[1].replace('.json', '.syms.json')))
binary = sys.argv[2]
top = int(sys.argv[3]) if len(sys.argv) > 3 else 25
libname = binary.split('/')[-1]
lib_index = [i for i, l in enumerate(prof['libs']) if l['name'] == libname][0]
table = [d for d in syms['data'] if d['debug_name'] == libname][0]['symbol_table']
starts = [e['rva'] for e in table]
names = syms['string_table']

def func(rva):
    i = bisect.bisect_right(starts, rva) - 1
    if i >= 0 and rva < starts[i] + table[i]['size']:
        return names[table[i]['symbol']]
    return '?'

by_func = collections.Counter()
by_addr = collections.Counter()
total = 0
for t in prof['threads']:
    ft, st, samples = t['frameTable'], t['stackTable'], t['samples']
    res = t['resourceTable']
    for s in samples['stack']:
        if s is None:
            continue
        total += 1
        f = st['frame'][s]
        addr = ft['address'][f]
        r = t['funcTable']['resource'][ft['func'][f]]
        if r is None or res['lib'][r] != lib_index:
            by_func['<other lib>'] += 1
            continue
        by_func[func(addr)] += 1
        by_addr[addr] += 1

print(f'{total} samples')
for name, n in by_func.most_common(top):
    print(f'{100*n/total:5.1f}%  {name}')
lines = collections.Counter()
addrs = [a for a, _ in by_addr.most_common(400)]
if addrs:
    out = subprocess.run(['atos', '-o', binary, '-l', '0x100000000', '-i'] + [hex(0x100000000 + a) for a in addrs], capture_output=True, text=True).stdout
    blocks = out.strip().split('\n\n')
    for a, blk in zip(addrs, blocks):
        frames = blk.strip().split('\n')
        def short(fr):
            fn = fr.split(' (in ')[0].split('::')[-1][:40]
            loc = fr[fr.rfind('(') + 1:fr.rfind(')')] if '(' in fr else '?'
            return f'{fn}@{loc}'
        ours = [f for f in frames if 'zstd_rs' in f or 'zstd_rs' in f]
        loc = short(frames[0]) + ('  <- ' + short(ours[0]) if ours and ours[0] != frames[0] else '')
        lines[loc] += by_addr[a]
print('--- hottest source lines (innermost inlined frame)')
for loc, n in lines.most_common(top):
    print(f'{100*n/total:5.1f}%  {loc}')
