# Data licenses

The application code is licensed under GPL-3.0-or-later (see `LICENSE`). The language model files in
`app/src/main/assets/lm/*.rlm` are derived works of the corpora below and are distributed under
**CC BY-SA 4.0** (required by the share-alike terms of the OpenSubtitles frequency lists).

They were built by `tools/dictgen` (`fetch_corpora.py`, then `build_all.py`), which lives in the
repository history up to commit `d2d5b8f`; the exact corpus used for each language is listed in
`tools/dictgen/sources.json` there.

## Sources

- **Leipzig Corpora Collection**, Universität Leipzig — news, Wikipedia and web corpora
  (one per language, plus a 1M-sentence web corpus for the main languages), licensed under
  [CC BY](https://creativecommons.org/licenses/by/4.0/).
  D. Goldhahn, T. Eckart, U. Quasthoff: *Building Large Monolingual Dictionaries at the Leipzig
  Corpora Collection: From 100 to 200 Languages.* LREC 2012.
  <https://wortschatz.uni-leipzig.de/en/download>

- **FrequencyWords** by Hermit Dave, word frequency lists built from OpenSubtitles 2018
  (P. Lison, J. Tiedemann: *OpenSubtitles2016: Extracting Large Parallel Corpora from Movie and
  TV Subtitles.* LREC 2016), licensed under
  [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/).
  <https://github.com/hermitdave/FrequencyWords>

- **Tatoeba** (<https://tatoeba.org>) — short everyday sentences used as conversational
  context data, licensed under [CC BY 2.0 FR](https://creativecommons.org/licenses/by/2.0/fr/).

- **Rynk curated lists** (`tools/dictgen/curated/`) — slang, abbreviations and canonical
  spellings maintained in this repository's history.
