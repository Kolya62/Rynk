#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Converts RLM1 language models to the denser RLM2 format (in place).

RLM2 payload:
  varint n_alpha, n_alpha x varint codepoint          (alphabet, most frequent first)
  varint n, n x [varint n_chars, n_chars x code, zigzag varint delta of q]
      code < 255 is an alphabet index, 255 is followed by a varint codepoint;
      q = round(-log10(p) * 32) of the unigram probability
  bigram groups, trigram groups:
  varint n_groups, per group: varint key delta, varint k,
      k x varint id delta (ids ascending), k x u8 q6 (q6 = round(q / 6), q as in RLM1)
"""
import collections, math, struct, sys, zlib

def adler32(b):
    return zlib.adler32(b) & 0xFFFFFFFF

def venc(x):
    o = bytearray()
    while True:
        b = x & 127
        x >>= 7
        if x:
            o.append(b | 128)
        else:
            o.append(b)
            return bytes(o)

class R:
    def __init__(s, d): s.d, s.p = d, 0
    def v(s):
        r = sh = 0
        while True:
            b = s.d[s.p]; s.p += 1
            r |= (b & 127) << sh; sh += 7
            if b < 128: return r

def convert(data):
    assert data[:4] == b"RLM1"
    r = R(data[12:])
    n = r.v()
    words = []
    for _ in range(n):
        l = r.v(); t = r.d[r.p:r.p + l].decode(); r.p += l
        c = r.v(); f = struct.unpack_from("<H", r.d, r.p)[0]; r.p += 2
        words.append((t, c, f))
    total = max(1, sum(c for _, c, _ in words))
    def logp(c, f):
        lp = math.log10(c / total) if c > 0 else -7.0 + 5.7 * (min(f, 2500) / 2500.0)
        return max(lp, -9.0)
    counts = collections.Counter(ch for t, _, _ in words for ch in t)
    alpha = [ch for ch, _ in counts.most_common(255)]
    code = {ch: i for i, ch in enumerate(alpha)}
    out = bytearray(venc(len(alpha)))
    for ch in alpha:
        out += venc(ord(ch))
    out += venc(n)
    prev = 0
    for t, c, f in words:
        out += venc(len(t))
        for ch in t:
            if ch in code:
                out.append(code[ch])
            else:
                out.append(255); out += venc(ord(ch))
        q = round(-logp(c, f) * 32)
        dlt = q - prev; prev = q
        out += venc(dlt * 2 if dlt >= 0 else -dlt * 2 - 1)
    for _ in range(2):
        g = r.v(); out += venc(g)
        for _ in range(g):
            out += venc(r.v())
            k = r.v(); out += venc(k)
            ents = []
            for _ in range(k):
                i = r.v(); q = r.d[r.p]; r.p += 1
                ents.append((i, q))
            ents.sort()
            last = 0
            for i, _ in ents:
                out += venc(i - last); last = i
            out += bytes(min(255, round(q / 6)) for _, q in ents)
    assert r.p == len(r.d)
    return b"RLM2" + struct.pack("<HHI", 2, 0, adler32(bytes(out))) + bytes(out)

for path in sys.argv[1:]:
    data = open(path, "rb").read()
    if data[:4] == b"RLM1":
        new = convert(data)
        open(path, "wb").write(new)
        print(path, len(data), "->", len(new))
