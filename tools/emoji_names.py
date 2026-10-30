#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Builds the emoji search index from CLDR annotations (common/annotations/<lang>.xml).

    tools/emoji_names.py rust-core/src/emoji.rs en.xml ru.xml ... > rust-core/src/emoji_names.txt

Only emoji of the palette are kept. Each line: emoji, a tab, then lowercase keywords of all the
given languages separated by '|' (the name first).
"""
import re, sys, xml.etree.ElementTree as ET

palette = []
for m in re.finditer(r'\("[^"]*", "([^"]*)"\)', open(sys.argv[1], encoding="utf-8").read()):
    for e in m.group(1).split(" "):
        if e not in palette:
            palette.append(e)
words = {e: [] for e in palette}
for path in sys.argv[2:]:
    for a in ET.parse(path).getroot().iter("annotation"):
        cp = a.get("cp", "").replace("️", "")
        if cp not in words:
            continue
        kw = [k.strip().lower() for k in (a.text or "").split("|") if k.strip()]
        if a.get("type") == "tts":
            kw = kw[:1]
            words[cp][:0] = [k for k in kw if k not in words[cp]]
        else:
            words[cp] += [k for k in kw if k not in words[cp]]
for e in palette:
    print(e + "\t" + "|".join(words[e]))
