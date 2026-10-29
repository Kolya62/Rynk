#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Builds reading → word conversion dictionaries (app/src/main/assets/cjk/<code>.rcd).

  zh-hans, zh-hant: toneless pinyin (ü typed as v) → simplified / traditional words (CC-CEDICT)
  yue:              toneless jyutping → words (CC-Canto readings of CC-CEDICT)
  ja:               hiragana → kanji/kana words (JMdict), plus verb and adjective stems so that
                    inflected forms convert segment by segment

Word frequencies come from Leipzig corpora segmented by forward maximum matching.

Usage: python3 -I tools/cjkgen.py <data dir>
  <data dir>/cedict.txt, canto-readings.txt, JMdict_e.gz,
  <data dir>/leipzig/*/*-sentences.txt (zho_* and jpn_* corpora)

File format:
  "RCD1" | u16 version=1 | u16 0 | u32 adler32(payload)
  payload: varint n, n × [varint len, key utf8, varint len, word utf8, u8 q] sorted by key, best first
  q = round(-log10(p) * 16)
"""
import collections
import glob
import gzip
import math
import multiprocessing as mp
import pathlib
import re
import struct
import sys
import xml.etree.ElementTree as ET
import zlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "app" / "src" / "main" / "assets" / "cjk"
LM = ROOT / "app" / "src" / "main" / "assets" / "lm"
TRAD_MARK = set("們個這說會來時對國語學為發開關麼與後過還見長門問間現種點體實當應")
SIMP_MARK = set("们个这说会来时对国语学为发开关么与后过还见长门问间现种点体实当应")
MAXLEN = 8


def varint(v):
    out = bytearray()
    while v >= 0x80:
        out.append((v & 0x7F) | 0x80)
        v >>= 7
    out.append(v)
    return bytes(out)


def write_rcd(path, entries):
    """entries: dict (key, word) -> log10 p"""
    rows = sorted(entries.items(), key=lambda kv: (kv[0][0], -kv[1], kv[0][1]))
    payload = bytearray(varint(len(rows)))
    for (key, word), lp in rows:
        k, w = key.encode(), word.encode()
        payload += varint(len(k)) + k + varint(len(w)) + w + bytes([max(0, min(255, round(-lp * 16)))])
    head = b"RCD1" + struct.pack("<HHI", 1, 0, zlib.adler32(bytes(payload)) & 0xFFFFFFFF)
    path.write_bytes(head + payload)
    print(f"{path.name}: {len(rows)} entries, {len(head) + len(payload)} bytes")


def read_rlm_words(code):
    """Words of an old curated model, most common first (used as a frequency prior)"""
    p = LM / f"{code}.rlm"
    if not p.exists():
        return []
    d = p.read_bytes()[12:]
    i = 0

    def vi():
        nonlocal i
        v = s = 0
        while True:
            b = d[i]
            i += 1
            v |= (b & 0x7F) << s
            s += 7
            if b < 0x80:
                return v

    out = []
    for _ in range(vi()):
        n = vi()
        out.append(d[i:i + n].decode())
        i += n
        vi()
        i += 2
    return out


# ---------------------------------------------------------------- corpus counting

_WORDS = None


def _init(words):
    global _WORDS
    _WORDS = words


def _count_lines(args):
    lines, script = args
    words = _WORDS[script]
    c = collections.Counter()
    for line in lines:
        i, n = 0, len(line)
        while i < n:
            for L in range(min(MAXLEN, n - i), 0, -1):
                s = line[i:i + L]
                if s in words:
                    c[s] += 1
                    i += L
                    break
            else:
                i += 1
    return c


def corpus_lines(pattern):
    for f in glob.glob(pattern):
        with open(f, encoding="utf-8") as fh:
            for line in fh:
                parts = line.rstrip("\n").split("\t", 1)
                yield parts[-1]


def count(words_by_script, lines_by_script):
    jobs = []
    for script, lines in lines_by_script.items():
        for i in range(0, len(lines), 20000):
            jobs.append((lines[i:i + 20000], script))
    total = {s: collections.Counter() for s in words_by_script}
    with mp.Pool(initializer=_init, initargs=(words_by_script,)) as pool:
        for (lines, script), c in zip(jobs, pool.imap(_count_lines, jobs)):
            total[script].update(c)
    return total


def to_logp(counts, keys, prior=()):
    tot = sum(counts.values()) + 1
    prior_rank = {w: r for r, w in enumerate(prior)}
    out = {}
    for w in keys:
        c = counts.get(w, 0)
        if w in prior_rank:
            # curated conversational words: at least a solid frequency
            c = max(c, tot * 1e-4 / (1 + prior_rank[w] / 50))
        out[w] = math.log10((c + 0.3) / tot)
    return out


# ---------------------------------------------------------------- Chinese

def pinyin_key(py):
    sylls = py.lower().replace("u:", "v").split()
    key = []
    for s in sylls:
        s = re.sub(r"[1-5]$", "", s)
        if not re.fullmatch(r"[a-zv]+", s):
            return None
        key.append(s)
    return "".join(key)


def jyutping_key(jp):
    key = []
    for s in jp.lower().split():
        s = re.sub(r"[1-6]$", "", s)
        if not re.fullmatch(r"[a-z]+", s):
            return None
        key.append(s)
    return "".join(key)


def chinese(data):
    simp_entries, trad_entries, yue_entries = [], [], []
    proper = set()  # names (capitalized pinyin): rarely what a short reading means
    for line in open(data / "cedict.txt", encoding="utf-8"):
        if line.startswith("#"):
            continue
        m = re.match(r"(\S+) (\S+) \[([^\]]+)\]", line)
        if not m:
            continue
        trad, simp, py = m.groups()
        k = pinyin_key(py)
        if k and len(simp) <= MAXLEN:
            simp_entries.append((k, simp))
            trad_entries.append((k, trad))
            if py[:1].isupper():
                proper.add((k, simp))
                proper.add((k, trad))
    for line in open(data / "canto-readings.txt", encoding="utf-8"):
        if line.startswith("#"):
            continue
        m = re.match(r"(\S+) (\S+) \[[^\]]+\] \{([^}]+)\}", line)
        if m:
            k = jyutping_key(m.group(3))
            if k:
                yue_entries.append((k, m.group(1)))
    yue_entries += [(k, w) for w, k in YUE_COLLOQUIAL]
    simp_words = {w for _, w in simp_entries}
    trad_words = {w for _, w in trad_entries} | {w for _, w in yue_entries}
    lines = {"simp": [], "trad": []}
    for line in corpus_lines(str(data / "leipzig" / "zho*" / "*" / "*-sentences.txt")):
        t = sum(1 for ch in line if ch in TRAD_MARK)
        s = sum(1 for ch in line if ch in SIMP_MARK)
        lines["trad" if t > s else "simp"].append(line)
    print(f"corpus: {len(lines['simp'])} simplified, {len(lines['trad'])} traditional sentences")
    counts = count({"simp": simp_words, "trad": trad_words}, lines)
    lp_s = to_logp(counts["simp"], simp_words, read_rlm_words("zh-hans"))
    lp_t = to_logp(counts["trad"], trad_words, read_rlm_words("zh-hant"))
    yue_prior = [w for w, _ in YUE_COLLOQUIAL] + read_rlm_words("yue")
    lp_y = to_logp(counts["trad"], trad_words, yue_prior)
    for name, entries, lp in (("zh-hans", simp_entries, lp_s), ("zh-hant", trad_entries, lp_t), ("yue", yue_entries, lp_y)):
        d = {}
        colloquial = {w for w, _ in YUE_COLLOQUIAL} if name == "yue" else set()
        for k, w in entries:
            # written Cantonese prefers its own characters (係 over 系)
            bonus = 0.8 if w in colloquial else 0.0
            if (k, w) in proper:
                bonus -= 1.0
            d[(k, w)] = max(d.get((k, w), -99), lp[w] + bonus)
        write_rcd(OUT / f"{name}.rcd", d)


# Everyday written Cantonese missing from the CC-CEDICT readings (word, toneless jyutping),
# most frequent first
YUE_COLLOQUIAL = [(w, k) for w, k in (x.split(":") for x in """
係:hai 唔:m 佢:keoi 嘅:ge 咗:zo 啲:di 咁:gam 嘢:je 冇:mou 喺:hai 睇:tai 嚟:lai 乜:mat 邊:bin 點:dim
啱:ngaam 俾:bei 畀:bei 攞:lo 嗰:go 呢:ni 呢:ne 哋:dei 我哋:ngodei 你哋:neidei 佢哋:keoidei 唔該:mgoi
多謝:doze 冇問題:moumantai 點解:dimgaai 乜嘢:matje 邊度:bindou 而家:jigaa 咩:me 㗎:gaa 啦:laa 喇:laa
囉:lo 噃:bo 嘛:maa 吖:aa 啩:gwaa 咪:mai 唧:zek 搵:wan 瞓:fan 食:sik 飲:jam 行:haang 企:kei 返:faan
鍾意:zungji 靚:leng 好:hou 幾:gei 仲:zung 都:dou 就:zau 先:sin 未:mei 晒:saai 埋:maai 喎:wo 啫:ze
噉:gam 咩事:meisi 點樣:dimjoeng 幾多:geido 依家:jigaa 宜家:jigaa 琴日:kamjat 聽日:tingjat 今日:gamjat
尋日:camjat 頭先:tausin 等陣:dangzan 一陣:jatzan 啱啱:ngaamngaam 唔係:mhai 係咪:haimai 有冇:jaumou
得唔得:dakmdak 好唔好:houmhou 唔好:mhou 唔使:msai 唔駛:msai 識:sik 諗:nam 講:gong 話:waa 嘈:cou
攰:gui 癐:gui 嬲:nau 衰:seoi 抵:dai 平:peng 貴:gwai 錢:cin 蚊:man 屋企:ukkei 返工:faangung 放工:fonggung
食飯:sikfaan 飲茶:jamcaa 冇事:mousi 有啲:jaudi 少少:siusiu 多啲:dodi 快啲:faaidi 慢慢:maanmaan
""".split())]


# ---------------------------------------------------------------- Japanese

def hira(s):
    return "".join(chr(ord(c) - 0x60) if "ァ" <= c <= "ヶ" else c for c in s)


GODAN = {  # ending: (i-row for masu stem, a-row for negative, te/ta stem ending)
    "v5k": ("く", "き", "か", "い"), "v5g": ("ぐ", "ぎ", "が", "い"), "v5s": ("す", "し", "さ", "し"),
    "v5t": ("つ", "ち", "た", "っ"), "v5n": ("ぬ", "に", "な", "ん"), "v5b": ("ぶ", "び", "ば", "ん"),
    "v5m": ("む", "み", "ま", "ん"), "v5r": ("る", "り", "ら", "っ"), "v5u": ("う", "い", "わ", "っ"),
    "v5k-s": ("く", "き", "か", "っ"), "v5r-i": ("る", "り", "ら", "っ"),
}


def pos_code(desc):
    """JMdict part-of-speech entity text (expanded by the XML parser) → code"""
    if desc.startswith("Ichidan verb"):
        return "v1"
    if desc.startswith("adjective (keiyoushi)"):
        return "adj-i"
    if desc.startswith("Godan verb - Iku/Yuku"):
        return "v5k-s"
    m = re.match(r"Godan verb with '(\w+)' ending( \(irregular verb\))?$", desc)
    if m:
        code = {"ku": "v5k", "gu": "v5g", "su": "v5s", "tsu": "v5t", "nu": "v5n", "bu": "v5b", "mu": "v5m", "ru": "v5r", "u": "v5u"}.get(m.group(1))
        return code + "-i" if code == "v5r" and m.group(2) else code
    return desc


def stems(word, reading, pos):
    """Inflection stems (surface, reading) for verbs and i-adjectives"""
    out = []
    for p in map(pos_code, pos):
        if p == "v1" and word.endswith("る") and reading.endswith("る"):
            out.append((word[:-1], reading[:-1]))
        elif p in GODAN:
            end, i_row, a_row, te = GODAN[p]
            if word.endswith(end) and reading.endswith(end):
                for repl in {i_row, a_row, te}:
                    out.append((word[:-1] + repl, reading[:-1] + repl))
        elif p == "adj-i" and word.endswith("い") and reading.endswith("い"):
            out.append((word[:-1], reading[:-1]))
    return [(w, r) for w, r in out if len(r) >= 1 and w != r]


def japanese(data):
    pairs = {}  # (reading, word) -> (priority score, is_stem)
    words = set()
    with gzip.open(data / "JMdict_e.gz", "rb") as fh:
        for _, el in ET.iterparse(fh, events=("end",)):
            if el.tag != "entry":
                continue
            kanji = [(k.findtext("keb"), [p.text for p in k.findall("ke_pri")]) for k in el.findall("k_ele")]
            pos_codes = [p.text for s in el.findall("sense") for p in s.findall("pos")]
            for r in el.findall("r_ele"):
                reb = r.findtext("reb")
                r_pri = [p.text for p in r.findall("re_pri")]
                restr = {x.text for x in r.findall("re_restr")}
                key = hira(reb)
                if not re.fullmatch(r"[ぁ-ゖー]+", key):
                    continue
                forms = []
                if r.find("re_nokanji") is None and kanji:
                    forms += [(k, pri) for k, pri in kanji if not restr or k in restr]
                if not kanji or r.find("re_nokanji") is not None or reb != key:
                    forms.append((reb, r_pri))  # kana word, katakana loanword
                for word, pri in forms:
                    score = 0.0
                    for t in pri:
                        if t in ("news1", "ichi1", "spec1", "gai1"):
                            score += 1.0
                        elif t in ("news2", "ichi2", "spec2", "gai2"):
                            score += 0.4
                        elif t.startswith("nf"):
                            score += (49 - int(t[2:])) / 48
                    old = pairs.get((key, word))
                    if old is None or score > old[0]:
                        pairs[(key, word)] = (score, False)
                    words.add(word)
                    for sw, sr in stems(word, key, pos_codes):
                        if (sr, sw) not in pairs:
                            pairs[(sr, sw)] = (score - 0.3, True)
                            words.add(sw)
            el.clear()
    lines = {"ja": list(corpus_lines(str(data / "leipzig" / "jpn*" / "*" / "*-sentences.txt")))}
    counts = count({"ja": words}, lines)["ja"]
    lp = to_logp(counts, words, read_rlm_words("ja"))
    d = {}
    for (key, word), (score, is_stem) in pairs.items():
        c = counts.get(word, 0)
        # keep what is used: corpus evidence, a priority tag, or a short kana word
        if c == 0 and score <= 0 and not is_stem:
            continue
        if is_stem and score <= 0 and c == 0:
            continue
        if word == key:
            score -= 0.2  # plain hiragana is always offered separately
        d[(key, word)] = lp[word] + 0.6 * score
    write_rcd(OUT / "ja.rcd", d)


if __name__ == "__main__":
    data = pathlib.Path(sys.argv[1])
    OUT.mkdir(parents=True, exist_ok=True)
    chinese(data)
    japanese(data)
