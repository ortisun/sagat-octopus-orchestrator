import collections
import hashlib
import json
import pathlib
import re
import urllib.parse

ROOT = pathlib.Path(__file__).resolve().parents[3]
TEMP = pathlib.Path(__file__).parent
ITEMS = json.loads((TEMP / 'inventory.json').read_text())
CACHE = {}


def read(path):
    path = path.resolve()
    if path not in CACHE:
        CACHE[path] = path.read_text()
    return CACHE[path]


def outside_fences(text):
    fenced = False
    for number, line in enumerate(text.splitlines(), 1):
        if re.match(r'\s*(```|~~~)', line):
            fenced = not fenced
        elif not fenced:
            yield number, line


def anchors(text):
    counts = collections.Counter()
    found = set()
    for _, line in outside_fences(text):
        h = re.match(r'^#{1,6}\s+(.+?)\s*#*$', line)
        if h:
            name = re.sub(r'[^\w\- ]', '', h[1].lower()).replace(' ', '-')
            i = counts[name]
            counts[name] += 1
            found.add(name + ('-' + str(i) if i else ''))
        found.update(re.findall(r'(?:id|name)=[\"\']([^\"\']+)', line))
    return found


audit = []
missing_refs = []
missing_links = []
missing_anchors = []
missing_tokens = []
rx = re.compile(r'\[([^\]\n]+?):([^\]\n]+)\](?!\()')
for item in ITEMS:
    doc = ROOT / item['path']
    text = read(doc)
    row = {**item, 'citations': [], 'local_links': [], 'source_files': []}
    for number, line in outside_fences(text):
        for reference, symbol in rx.findall(line):
            if reference.startswith(('http', 'plugin')):
                continue
            candidates = [doc.parent / reference, ROOT / reference]
            if item['path'].startswith('vendor/contracts/rskafka/'):
                rel = doc.parent.relative_to(ROOT / 'vendor/contracts/rskafka')
                candidates.append(ROOT / 'vendor/rskafka' / rel / reference)
            source = next((x.resolve() for x in candidates if x.is_file() and x.resolve().is_relative_to(ROOT)), None)
            rec = {'line': number, 'reference': reference, 'symbol': symbol}
            if source is None:
                missing_refs.append({'doc': item['path'], **rec})
            else:
                body = read(source)
                rec['source'] = str(source.relative_to(ROOT))
                # Lexical existence is a screening check, not proof of behavior.
                tokens = re.findall(r'[A-Za-z_][A-Za-z_0-9]*', symbol)
                missing = [t for t in tokens if not re.search(r'\b' + re.escape(t) + r'\b', body)]
                if missing and symbol not in body:
                    missing_tokens.append({'doc': item['path'], **rec, 'missing': missing})
                row['source_files'].append(rec['source'])
            row['citations'].append(rec)
        for target in re.findall(r'\[[^\]]*\]\(([^\s)]+)(?:\s+[^)]*)?\)', line):
            if re.match(r'[A-Za-z]+:', target) or target.startswith('//'):
                continue
            path, _, fragment = urllib.parse.unquote(target.strip('<>')).partition('#')
            resolved = ((ROOT / path[1:]) if path.startswith('/') else doc.parent / path).resolve() if path else doc
            rec = {'line': number, 'target': target}
            row['local_links'].append(rec)
            if not resolved.exists():
                missing_links.append({'doc': item['path'], **rec})
            elif fragment and resolved.is_file() and resolved.suffix == '.md':
                if not re.fullmatch(r'L\d+(?:-L?\d+)?', fragment) and fragment not in anchors(read(resolved)):
                    missing_anchors.append({'doc': item['path'], **rec})
    row['source_files'] = sorted(set(row['source_files']))
    row['status'] = 'static screening complete; semantic comparison pending'
    audit.append(row)

result = {'scope': 'Frozen 203 bootstrap READMEs; static lexical and link screening only.',
          'documents': [{'path': x['path'], 'sha256': x['sha256'], 'lines': x['lines'],
                         'citations_checked': len(x['citations']),
                         'local_links_checked': len(x['local_links']),
                         'cited_sources': x['source_files']} for x in audit],
          'screening': {'unresolved_references': missing_refs,
                        'missing_links': missing_links, 'missing_anchors': missing_anchors,
                        'lexical_candidates': missing_tokens},
          'source_hashes': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                            for p in CACHE if p.is_relative_to(ROOT)}}
for item in ITEMS:
    if hashlib.sha256((ROOT / item['path']).read_bytes()).hexdigest() != item['sha256']:
        raise SystemExit('Frozen README changed: ' + item['path'])
(TEMP / 'static_checks.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'documents': len(audit), 'citations': sum(len(x['citations']) for x in audit),
                  'local_links': sum(len(x['local_links']) for x in audit),
                  'files_read_full_by_checker': len(CACHE),
                  'unresolved_references': len(missing_refs), 'missing_links': len(missing_links),
                  'missing_anchors': len(missing_anchors), 'lexical_candidates': len(missing_tokens)}, indent=2))
if missing_refs or missing_links or missing_anchors or missing_tokens:
    raise SystemExit(1)
