# Data licenses

The application code is licensed under GPL-3.0-or-later (see `LICENSE`). The data files below are
derived works of the listed sources and are distributed under **CC BY-SA 4.0**, as their
share-alike terms require.

## Word models — `app/src/main/assets/lm/*.rlm`

- **Leipzig Corpora Collection**, Universität Leipzig — news, Wikipedia, web and community
  corpora, [CC BY](https://creativecommons.org/licenses/by/4.0/).
  D. Goldhahn, T. Eckart, U. Quasthoff: *Building Large Monolingual Dictionaries at the Leipzig
  Corpora Collection: From 100 to 200 Languages.* LREC 2012. <https://wortschatz.uni-leipzig.de/en/download>
- **FrequencyWords** by Hermit Dave, from OpenSubtitles 2018 (P. Lison, J. Tiedemann, LREC 2016),
  [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). <https://github.com/hermitdave/FrequencyWords>
- **Tatoeba** (<https://tatoeba.org>), [CC BY 2.0 FR](https://creativecommons.org/licenses/by/2.0/fr/).
- Curated word lists maintained in this repository's history.

The models of 85 languages were built by `tools/dictgen` (in the history up to commit `d2d5b8f`,
see `tools/dictgen/sources.json` there). The models of `as ba ckb ceb fo fy ht jv lb mg mi ny or
ps rw sa sn st su ti tt xh yi` are built from Leipzig corpora by `tools/lmgen.py`; the corpus of
each language is listed in that script.

## Conversion dictionaries — `app/src/main/assets/cjk/*.rcd`

Built by `tools/cjkgen.py`.

- **CC-CEDICT** (MDBG), Chinese readings and words, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/).
  <https://www.mdbg.net/chinese/dictionary?page=cc-cedict>
- **CC-Canto readings of CC-CEDICT**, © 2015 Pleco Software, [CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/)
  (adapted under 4.0 as 3.0 permits). <https://cantonese.org>
- **JMdict**, Electronic Dictionary Research and Development Group,
  [CC BY-SA 4.0](https://www.edrdg.org/edrdg/licence.html). <https://www.edrdg.org/jmdict/j_jmdict.html>
- Word frequencies from the Leipzig corpora `zho_news_2020_300K`, `zho-cn_web_2015_300K`,
  `zho_wikipedia_2018_300K` and `jpn_wikipedia_2018_300K` (CC BY).
