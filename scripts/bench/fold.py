#!/usr/bin/env python3
"""fold.py <perf-script.txt> <main-pid> : folded stacks for flamegraph.pl.
The script is `perf script -F comm,pid,tid,ip,sym,dso` output; the main thread is the thread
whose id is the main process's.
Chromium is a black box: every helper process is one frame, and runs of libcef.so frames inside
the main process collapse to one. LBR stacks hold 32 frames at most: a truncated stack is
completed with the most common chain of callers seen above its outermost frame."""
import re, sys, collections
if len(sys.argv) != 3:
    sys.exit(__doc__)
txt, mainpid = sys.argv[1], int(sys.argv[2])
stacks, cur = [], None
for line in open(txt, errors='replace'):
    if not line.strip():
        if cur: stacks.append(cur); cur = None
        continue
    if not line.startswith(('\t', ' ')):
        m = re.match(r'(.+?)\s+(\d+)/(\d+)', line)
        pid, tid = int(m.group(2)), int(m.group(3))
        # the main thread by its id: its name is the binary's, whatever the binary is called
        cur = ['wubble' if tid == pid else m.group(1).strip(), pid, []]
    else:
        m = re.match(r'\s+[0-9a-f]+ (.*) \((.*)\)$', line.rstrip())
        if m: cur[2].append((m.group(1), m.group(2).split('/')[-1]))
if cur: stacks.append(cur)
kinds = collections.defaultdict(set)
for c, p, _ in stacks: kinds[p].add(c)
def kind(p):
    k = kinds[p]
    if any('Gpu' in c or 'Viz' in c for c in k): return 'chromium GPU process (black box)'
    if any('Renderer' in c or 'Codec' in c or 'Media' in c for c in k): return 'chromium page process (black box)'
    return 'chromium other process (black box)'
def clean(sym):
    sym = re.sub(r'\+0x[0-9a-f]+$', '', sym)
    sym = re.sub(r'::\{closure#\d+\}', '::{closure}', sym)
    sym = re.sub(r'::<.*>$', '', sym)
    sym = re.sub(r' as [^>]*>', '>', sym)
    return sym.replace(';', ':')[:120]
def frames_of(frames):
    f = []
    for sym, dso in reversed(frames):   # root first
        if dso == 'libcef.so': name = '[chromium in-process]'
        elif dso.startswith('libnvidia'): name = '[nvidia driver]'
        elif sym == '[unknown]': name = '[unknown in %s]' % dso.split('.so')[0]
        elif dso.startswith('wubble'): name = clean(sym)
        else: name = clean(sym) + ' [%s]' % dso.split('.so')[0]
        if not f or f[-1] != name or not name.startswith('['): f.append(name)
    return f
ROOTS = ('run_event_loop', 'main', '_start', '__libc_start', 'start_thread', 'clone', 'lang_start')
def rooted(f): return bool(f) and any(k in f[0] for k in ROOTS)
main = [(c, frames_of(fr)) for c, p, fr in stacks if p == mainpid]
# callers[(thread, frame)] -> Counter of the chain above it, from stacks where it is not outermost
callers = collections.defaultdict(collections.Counter)
for c, f in main:
    for i in range(1, len(f)):
        callers[(c, f[i])][tuple(f[:i])] += 1
out = collections.Counter(); stats = collections.Counter()
for c, p, fr in stacks:
    if p != mainpid: out[kind(p)] += 1
for c, f in main:
    hops = 0
    while f and not rooted(f) and callers.get((c, f[0])) and hops < 8:
        # prefer a chain that is itself rooted
        best = max(callers[(c, f[0])].items(), key=lambda kv: (rooted(list(kv[0])), kv[1]))[0]
        f = list(best) + f; hops += 1
    stats['rooted' if rooted(f) else ('stitched' if hops else ('empty' if not f else 'unrooted'))] += 1
    thread = 'main thread' if c == 'wubble' else 'other threads'
    if c != 'wubble': f = [c] + f
    elif not rooted(f): f = ['(callers unknown)'] + f
    out[';'.join(['wubble main process', thread] + f)] += 1
for k, v in out.items(): print(k, v)
sys.stderr.write(str(dict(stats)) + ' main samples %d of %d\n' % (len(main), len(stacks)))
