#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Adds words a model lacks from a Leipzig word-frequency list (in place).

    tools/rlm_extend.py <model.rlm> <...-words.txt> <alphabet> [min_count]

The model may be zstd-compressed RLM2 (as in the assets); the result is written the same way.
New words get ids after the existing ones, so n-grams stay valid. Their probability comes
from their count, shifted by the median offset between the two sources on shared words.
"""
import collections, statistics, struct, subprocess, sys, zlib

def venc(x):
    o = bytearray()
    while True:
        b = x & 127; x >>= 7
        if x: o.append(b | 128)
        else: o.append(b); return bytes(o)

class R:
    def __init__(s, d): s.d, s.p = d, 0
    def v(s):
        r = sh = 0
        while True:
            b = s.d[s.p]; s.p += 1
            r |= (b & 127) << sh; sh += 7
            if b < 128: return r

def zz(z): return z >> 1 if z & 1 == 0 else -((z + 1) >> 1)

def main():
    path, words_path, alphabet = sys.argv[1], sys.argv[2], set(sys.argv[3])
    min_count = int(sys.argv[4]) if len(sys.argv) > 4 else 3
    raw = open(path, "rb").read()
    packed = raw[:4] == b"\x28\xb5\x2f\xfd"
    if packed:
        raw = subprocess.run(["zstd", "-d", "-c"], input=raw, capture_output=True, check=True).stdout
    assert raw[:4] == b"RLM2", "convert with tools/rlm2.py first"
    r = R(raw[12:])
    alpha = [chr(r.v()) for _ in range(r.v())]
    n = r.v()
    words, qs = [], []
    q = 0
    for _ in range(n):
        ln = r.v(); t = []
        for _ in range(ln):
            c = r.d[r.p]; r.p += 1
            t.append(chr(r.v()) if c == 255 else alpha[c])
        q += zz(r.v())
        words.append("".join(t)); qs.append(q)
    ngrams = r.d[r.p:]
    known = {w.lower() for w in words}

    counts = collections.Counter()
    forms = collections.defaultdict(collections.Counter)
    for line in open(words_path, encoding="utf-8", errors="replace"):
        f = line.rstrip("\n").split("\t")
        if len(f) < 3:
            continue
        w, c = f[1], int(f[2])
        lw = w.lower()
        if not (2 <= len(lw) <= 24) or not all(ch in alphabet or ch == "-" for ch in lw):
            continue
        if lw.startswith("-") or lw.endswith("-") or "--" in lw:
            continue
        counts[lw] += c
        forms[lw][w] += c
    total = sum(counts.values())
    # offset between the sources, from shared words: q_model ≈ -32*log10(count/total) + shift
    import math
    by_lower = {w.lower(): qv for w, qv in zip(words, qs)}
    diffs = [by_lower[w] + 32 * math.log10(c / total) for w, c in counts.items() if w in by_lower and c >= 20]
    shift = statistics.median(diffs) if diffs else 0.0
    new = [(w, c) for w, c in counts.items() if w not in known and c >= min_count]
    new.sort(key=lambda x: -x[1])
    for w, c in new:
        disp = forms[w].most_common(1)[0][0]
        # a capitalized form only when it dominates (names); otherwise lowercase
        if disp != w and forms[w][disp] < 0.8 * c:
            disp = w
        words.append(disp)
        qs.append(max(0, min(9 * 32, round(-32 * math.log10(c / total) + shift))))

    counts_a = collections.Counter(ch for w in words for ch in w)
    alpha = [ch for ch, _ in counts_a.most_common(255)]
    code = {ch: i for i, ch in enumerate(alpha)}
    out = bytearray(venc(len(alpha)))
    for ch in alpha:
        out += venc(ord(ch))
    out += venc(len(words))
    prev = 0
    for w, qv in zip(words, qs):
        out += venc(len(w))
        for ch in w:
            if ch in code: out.append(code[ch])
            else: out.append(255); out += venc(ord(ch))
        d = qv - prev; prev = qv
        out += venc(d * 2 if d >= 0 else -d * 2 - 1)
    out += ngrams
    data = b"RLM2" + struct.pack("<HHI", 2, 0, zlib.adler32(bytes(out)) & 0xFFFFFFFF) + bytes(out)
    if packed:
        data = subprocess.run(["zstd", "-19", "-q", "-c"], input=data, capture_output=True, check=True).stdout
    open(path, "wb").write(data)
    print(f"{path}: {n} + {len(new)} words (shift {shift:.1f})")

main()
