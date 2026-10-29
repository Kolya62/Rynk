#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Builds word models (app/src/main/assets/lm/<code>.rlm, format RLM1: words + bigrams) from
Leipzig corpora for languages that have none yet. Only words typeable on the language's layout
are kept.

Usage: python3 -I tools/lmgen.py <leipzig dir>
"""
import collections
import glob
import math
import pathlib
import re
import struct
import sys
import unicodedata
import zlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "app" / "src" / "main" / "assets" / "lm"
LANGS = ROOT / "rust-core" / "src" / "layout" / "languages.rs"

SOURCES = {
    "fo": ["fao_wikipedia_2021_30K"],
    "jv": ["jav_wikipedia_2021_100K", "jav_community_2017"],
    "su": ["sun_wikipedia_2021_30K", "sun_community_2017"],
    "ceb": ["ceb_wikipedia_2021_100K", "ceb_community_2017"],
    "xh": ["xho-za_web_2018_30K", "xho_community_2017"],
    "rw": ["kin_community_2017"],
    "ny": ["nya_community_2017"],
    "sn": ["sna-zw_web_2018_100K"],
    "st": ["sot-za_web_2018_10K"],
    "mg": ["mlg_wikipedia_2014_30K", "mlg_community_2017"],
    "mi": ["mri_web_2011_100K"],
    "ht": ["hat_wikipedia_2021_30K", "hat_community_2017"],
    "lb": ["ltz_wikipedia_2021_100K"],
    "fy": ["fry_wikipedia_2021_100K"],
    "tt": ["tat_web_2018_1M"],
    "ba": ["bak_wikipedia_2021_100K", "bak_community_2017"],
    "yi": ["yid_wikipedia_2021_30K"],
    "ckb": ["ckb_wikipedia_2021_100K", "ckb_community_2017"],
    "ps": ["pus_wikipedia_2021_30K"],
    "sa": ["san_wikipedia_2021_100K"],
    "as": ["asm_wikipedia_2021_100K", "asm_community_2017"],
    "or": ["ori_wikipedia_2021_100K", "ori_community_2017"],
    "ti": ["tir_community_2017"],
}
MAX_WORDS = 40000
MAX_NEXT = 8
INNER = set("'’-ʼ‌‍")


def layout_chars(code):
    """Every character on the layout (both layers and long-press), lowercase"""
    for line in LANGS.read_text().splitlines():
        if f'code: "{code}",' in line:
            seg = line.split("rows:", 1)[1].split("caseless:", 1)[0]
            chars = set("".join(re.findall(r'"((?:[^"\\]|\\.)*)"', seg)).lower())
            return chars
    raise SystemExit(f"no layout {code}")


def tokens(sentence):
    word = []
    for ch in sentence:
        cat = unicodedata.category(ch)
        if cat[0] in "LM" or (ch in INNER and word):
            word.append(ch)
        else:
            if word:
                yield "".join(word).strip("".join(INNER))
            word = []
            if ch in ".!?।॥።؟":
                yield None  # sentence boundary
    if word:
        yield "".join(word).strip("".join(INNER))


def varint(v):
    out = bytearray()
    while v >= 0x80:
        out.append((v & 0x7F) | 0x80)
        v >>= 7
    out.append(v)
    return bytes(out)


def build(code, dirs, data):
    allowed = layout_chars(code) | INNER
    words = collections.Counter()
    pairs = collections.Counter()
    starts = collections.Counter()
    for d in dirs:
        for f in glob.glob(str(data / d / "*" / "*-sentences.txt")) + glob.glob(str(data / d / "*-sentences.txt")):
            for line in open(f, encoding="utf-8"):
                text = line.rstrip("\n").split("\t", 1)[-1]
                prev = None  # None = sentence start
                for t in tokens(text):
                    if t is None:
                        prev = None
                        continue
                    ok = t and all(c in allowed for c in t.lower()) and len(t) <= 40
                    if not ok:
                        prev = False  # unknown token breaks the context
                        continue
                    words[t] += 1
                    if prev is None:
                        starts[t] += 1
                    elif prev:
                        pairs[(prev, t)] += 1
                    prev = t
    vocab = [w for w, c in words.most_common(MAX_WORDS) if c >= 2]
    if len(vocab) < 500:
        print(f"{code}: only {len(vocab)} words, skipped")
        return
    ids = {w: i for i, w in enumerate(vocab)}
    top, low = math.log10(words[vocab[0]]), math.log10(max(1, words[vocab[-1]]))
    payload = bytearray(varint(len(vocab)))
    for w in vocab:
        b = w.encode()
        freq = round(2500 * (math.log10(words[w]) - low) / max(1e-9, top - low))
        payload += varint(len(b)) + b + varint(words[w]) + struct.pack("<H", max(0, min(2500, freq)))
    # bigrams: context id = word id + 1, 0 = sentence start
    groups = collections.defaultdict(list)
    ctx_total = collections.Counter()
    for (a, b), c in pairs.items():
        if a in ids and b in ids:
            ctx_total[a] += c
    for (a, b), c in pairs.items():
        if c >= 2 and a in ids and b in ids:
            groups[ids[a] + 1].append((c / ctx_total[a], ids[b]))
    total_starts = sum(starts.values()) or 1
    groups[0] = [(c / total_starts, ids[w]) for w, c in starts.items() if c >= 2 and w in ids]
    keys = sorted(k for k, v in groups.items() if v)
    payload += varint(len(keys))
    prev = 0
    n_pairs = 0
    for k in keys:
        best = sorted(groups[k], key=lambda x: -x[0])[:MAX_NEXT]
        payload += varint(k - prev) + varint(len(best))
        prev = k
        for p, i in best:
            payload += varint(i) + bytes([max(0, min(255, round(-math.log10(p) * 32)))])
            n_pairs += 1
    payload += varint(0)  # no trigrams
    head = b"RLM1" + struct.pack("<HHI", 1, 0, zlib.adler32(bytes(payload)) & 0xFFFFFFFF)
    (OUT / f"{code}.rlm").write_bytes(head + payload)
    print(f"{code}: {len(vocab)} words, {n_pairs} pairs, {len(head) + len(payload)} bytes")


if __name__ == "__main__":
    data = pathlib.Path(sys.argv[1])
    for code, dirs in SOURCES.items():
        build(code, dirs, data)
