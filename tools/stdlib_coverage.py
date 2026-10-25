#!/usr/bin/env python3
"""Regenerate the generated parts of docs/stdlib.md: the summary count, the
coverage-by-extension table and the per-extension function lists, and drop
functions that now exist from the "Notable gaps" table.

    tools/stdlib_coverage.py EXTS_TSV HAVE_TXT

EXTS_TSV: "extension<TAB>function" lines from real PHP; HAVE_TXT: the
functions phpx implements (see "Regenerating the coverage data" in the doc).
"""
import re, sys, os

exts_tsv, have_txt = sys.argv[1], sys.argv[2]
doc_path = os.path.join(os.path.dirname(__file__), '../docs/stdlib.md')
SKIP = {'xdebug', 'pcov', 'spx', 'Zend OPcache', 'igbinary', 'mongodb'}

by_ext = {}
for line in open(exts_tsv):
    ext, fn = line.rstrip('\n').split('\t')
    if ext not in SKIP:
        by_ext.setdefault(ext, []).append(fn)
have = {l.strip() for l in open(have_txt) if l.strip()}
total = sum(len(v) for v in by_ext.values())
impl = sum(1 for v in by_ext.values() for f in v if f in have)
core = by_ext.get('Core', []) + by_ext.get('standard', [])
core_impl = sum(1 for f in core if f in have)

rows = sorted(by_ext.items(), key=lambda kv: (-sum(f in have for f in kv[1]), -len(kv[1]), kv[0]))
table = ['| Extension | Implemented | PHP 8.5 total |', '|---|---:|---:|']
table += [f'| {e} | {sum(f in have for f in fs)} | {len(fs)} |' for e, fs in rows]

sections, none = [], []
for e, fs in rows:
    got = sorted(f for f in fs if f in have)
    miss = sorted(f for f in fs if f not in have)
    if not got:
        none.append(e)
        continue
    sections.append(f'### {e}\n')
    sections.append('Implemented: ' + ', '.join(f'`{f}`' for f in got) + '\n')
    sections.append(('Missing: ' + ', '.join(f'`{f}`' for f in miss)) if miss else 'Missing: none')
    sections.append('')
sections.append('Every other extension in the table above (' + ', '.join(none) + ') has\n**no** functions implemented yet.\n')

doc = open(doc_path).read()
doc = re.sub(r'- \*\*\d[\d,]*\*\* PHP functions implemented, out of the [\d,]+ functions exposed by\n  the reference PHP 8.5 build \(which includes many optional extensions\).\n  Counting only `Core` \+ `standard`, phpx has \d+ of \d+.',
             f'- **{impl}** PHP functions implemented, out of the {total:,} functions exposed by\n  the reference PHP 8.5 build (which includes many optional extensions).\n  Counting only `Core` + `standard`, phpx has {core_impl} of {len(core)}.', doc)
doc = re.sub(r'\| Extension \| Implemented \| PHP 8.5 total \|\n(\|.*\|\n)+', '\n'.join(table) + '\n', doc, count=1)
start = doc.index('## Implemented and missing functions, by extension')
end = doc.index('## Notable gaps')
doc = doc[:start] + '## Implemented and missing functions, by extension\n\n' + '\n'.join(sections) + '\n' + doc[end:]

# Notable gaps: keep only functions still missing.
def prune(m):
    cells = m.group(0).split(' | ')
    names = re.findall(r'`([a-z0-9_]+)`', cells[1])
    # Only cells that are plain lists of function names are pruned.
    if not names or re.sub(r'`[a-z0-9_]+`|[,\s]', '', cells[1]):
        return m.group(0)
    left = [n for n in names if n not in have]
    if not left:
        return ''
    cells[1] = ', '.join(f'`{n}`' for n in left)
    return ' | '.join(cells)
g0 = doc.index('## Notable gaps')
g1 = doc.index('## Built-in classes and interfaces')
gaps = re.sub(r'^\| (?!Area|---)[^\n]*\|\n', lambda m: prune(m), doc[g0:g1], flags=re.M)
doc = doc[:g0] + gaps + doc[g1:]
open(doc_path, 'w').write(doc)
print(f'{impl} of {total} functions; Core+standard {core_impl} of {len(core)}')
