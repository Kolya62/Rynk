#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generates rust-core/src/layout/languages.rs: keyboard layouts for every supported language.

Run: python3 -I tools/gen_layouts.py
Brahmic InScript layouts are derived from Devanagari by Unicode block offset; letters a script
does not have are dropped. Latin accents get their base letter from Unicode decomposition.
"""
import pathlib
import unicodedata as ud

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "rust-core" / "src" / "layout" / "languages.rs"
LM = {p.stem for p in (ROOT / "app" / "src" / "main" / "assets" / "lm").glob("*.rlm")}

QW = ["qwertyuiop", "asdfghjkl", "zxcvbnm"]
QZ = ["qwertzuiop", "asdfghjkl", "yxcvbnm"]

# Latin letters reachable by long-press on every Latin layout (after the language's own)
GLOBAL_ACCENTS = ("àáâäæãåāąă" "çćčċ" "ďđð" "èéêëēėęěẹ" "ğġģ" "ħ" "ìíîïīįıị" "ķ" "łľļĺ"
                  "ñńňņṅ" "òóôöõøœōőọ" "řŕ" "śšşșßṣ" "ťţțþ" "ùúûüūůűųụư" "ýÿ" "żźž")
SPECIAL_BASE = {"ł": "l", "ø": "o", "æ": "a", "œ": "o", "ß": "s", "đ": "d", "ð": "d", "þ": "t",
                "ı": "i", "ħ": "h", "ŋ": "n", "ə": "e", "ɓ": "b", "ɗ": "d", "ƙ": "k", "ƴ": "y",
                "ĳ": "i", "ŉ": "n"}

def base(c):
    if c in SPECIAL_BASE:
        return SPECIAL_BASE[c]
    d = ud.normalize("NFD", c)
    return d[0] if d else c

L = []  # language entries

def lang(code, name, rows, *, shift=None, popups=None, accents="", caseless=False, script="Latin",
         composer="None", turkic=False, currency="$", conv=""):
    rows = [r.split() if " " in r else list(r) for r in rows]
    shift = [r.split() if " " in r else list(r) for r in shift] if shift else []
    pop = {}
    if script == "Latin":
        on_layout = {k for r in rows for k in r}
        for c in accents + GLOBAL_ACCENTS:
            b = base(c)
            if c in on_layout or b not in on_layout:
                continue
            lst = pop.setdefault(b, [])
            if c not in lst:
                lst.append(c)
        # the language's own letters first, then at most a handful of foreign ones
        for b, lst in pop.items():
            own = [c for c in lst if c in accents]
            pop[b] = own + [c for c in lst if c not in accents][: max(0, 8 - len(own))]
    for k, v in (popups or {}).items():
        v = v.split() if " " in v else list(v)
        pop[k] = v + [c for c in pop.get(k, []) if c not in v]
    L.append(dict(code=code, name=name, rows=rows, shift=shift, pop=pop, caseless=caseless,
                  script=script, composer=composer, turkic=turkic, currency=currency, conv=conv))

# ---------------------------------------------------------------- Latin
lang("en", "English", QW)
lang("en-GB", "English (UK)", QW, currency="£")
lang("de", "Deutsch", ["qwertzuiopü", "asdfghjklöä", "yxcvbnm"], accents="ß", popups={"s": "ß"}, currency="€")
lang("de-CH", "Deutsch (Schweiz)", ["qwertzuiopü", "asdfghjklöä", "yxcvbnm"], currency="CHF")
lang("fr", "Français", ["azertyuiop", "qsdfghjklm", "wxcvbn'"], accents="àâæçéèêëîïôœùûüÿ", currency="€")
lang("fr-CA", "Français (Canada)", QW, accents="àâæçéèêëîïôœùûüÿ")
lang("es", "Español", ["qwertyuiop", "asdfghjklñ", "zxcvbnm"], accents="áéíóúüñ", currency="€")
lang("es-419", "Español (Latinoamérica)", ["qwertyuiop", "asdfghjklñ", "zxcvbnm"], accents="áéíóúüñ")
lang("pt", "Português", QW, accents="ãáâàçéêíóôõú", currency="€")
lang("pt-BR", "Português (Brasil)", QW, accents="ãáâàçéêíóôõú", currency="R$")
lang("it", "Italiano", QW, accents="àèéìòù", currency="€")
lang("nl", "Nederlands", QW, accents="éëïóöüèê", currency="€")
lang("ca", "Català", QW, accents="àçéèíïóòúü", currency="€")
lang("gl", "Galego", ["qwertyuiop", "asdfghjklñ", "zxcvbnm"], accents="áéíóúü", currency="€")
lang("eu", "Euskara", ["qwertyuiop", "asdfghjklñ", "zxcvbnm"], accents="ç", currency="€")
lang("sv", "Svenska", ["qwertyuiopå", "asdfghjklöä", "zxcvbnm"], accents="é", currency="kr")
lang("fi", "Suomi", ["qwertyuiopå", "asdfghjklöä", "zxcvbnm"], accents="šž", currency="€")
lang("no", "Norsk bokmål", ["qwertyuiopå", "asdfghjkløæ", "zxcvbnm"], accents="éèêóòô", currency="kr")
lang("da", "Dansk", ["qwertyuiopå", "asdfghjklæø", "zxcvbnm"], accents="é", currency="kr")
lang("is", "Íslenska", ["qwertyuiopð", "asdfghjklæö", "zxcvbnmþ"], accents="áéíóúý", currency="kr")
lang("fo", "Føroyskt", ["qwertyuiopå", "asdfghjklæø", "zxcvbnmð"], accents="áíóúý", currency="kr")
lang("et", "Eesti", ["qwertyuiopüõ", "asdfghjklöä", "zxcvbnm"], accents="šž", currency="€")
lang("lv", "Latviešu", QW, accents="āčēģīķļņšūž", currency="€")
lang("lt", "Lietuvių", QW, accents="ąčęėįšųūž", currency="€")
lang("pl", "Polski", QW, accents="ąćęłńóśźż", currency="zł")
lang("cs", "Čeština", ["qwertzuiopú", "asdfghjklů", "yxcvbnm"], accents="áčďéěíňóřšťúůýž", currency="Kč")
lang("sk", "Slovenčina", ["qwertzuiopú", "asdfghjklôä", "yxcvbnm"], accents="áäčďéíĺľňóôŕšťúýž", currency="€")
lang("hu", "Magyar", ["qwertzuiopőú", "asdfghjkléáű", "yxcvbnmöüó"], accents="í", currency="Ft")
lang("sl", "Slovenščina", ["qwertzuiopš", "asdfghjklčž", "yxcvbnm"], currency="€")
lang("hr", "Hrvatski", ["qwertzuiopš", "asdfghjklčćž", "yxcvbnmđ"], currency="€")
lang("bs", "Bosanski", ["qwertzuiopš", "asdfghjklčćž", "yxcvbnmđ"], currency="KM")
lang("sr-Latn", "Srpski (latinica)", ["qwertzuiopš", "asdfghjklčćž", "yxcvbnmđ"], currency="RSD")
lang("ro", "Română", ["qwertyuiopă", "asdfghjklșț", "zxcvbnmîâ"], currency="lei")
lang("sq", "Shqip", ["qwertzuiopç", "asdfghjklë", "yxcvbnm"], currency="L")
lang("mt", "Malti", ["qwertyuiopġħ", "asdfghjklż", "zxcvbnmċ"], accents="àèìòù", currency="€")
lang("tr", "Türkçe", ["qwertyuıopğü", "asdfghjklşi", "zxcvbnmöç"], accents="âîû", turkic=True, currency="₺")
lang("az", "Azərbaycan", ["qüertyuiopöğ", "asdfghjklıə", "zxcvbnmçş"], turkic=True, currency="₼")
lang("tk", "Türkmençe", ["äwertyuiopňö", "asdfghjklşü", "zýcvbnmçž"], currency="m")
lang("uz", "Oʻzbekcha", ["qwertyuiopʻ", "asdfghjkl", "zxcvbnm"], currency="soʻm")
lang("ku", "Kurdî (Kurmancî)", ["qwertyuiopûê", "asdfghjklşî", "zxcvbnmç"])
lang("vi", "Tiếng Việt", QW, accents="àáảãạăằắẳẵặâầấẩẫậđèéẻẽẹêềếểễệìíỉĩịòóỏõọôồốổỗộơờớởỡợùúủũụưừứửữựỳýỷỹỵ", currency="₫")
lang("id", "Bahasa Indonesia", QW, currency="Rp")
lang("ms", "Bahasa Melayu", QW, currency="RM")
lang("tl", "Filipino", QW, accents="ñ", currency="₱")
lang("jv", "Basa Jawa", QW, accents="éè", currency="Rp")
lang("su", "Basa Sunda", QW, accents="é", currency="Rp")
lang("ceb", "Cebuano", QW, accents="ñ", currency="₱")
lang("sw", "Kiswahili", QW, currency="TSh")
lang("ha", "Hausa", QW, accents="ɓɗƙƴ", popups={"b": "ɓ", "d": "ɗ", "k": "ƙ", "y": "ƴ"}, currency="₦")
lang("yo", "Yorùbá", QW, accents="ẹọṣàáèéìíòóùúńǹ", currency="₦")
lang("ig", "Igbo", QW, accents="ịọụṅ", currency="₦")
lang("zu", "isiZulu", QW, currency="R")
lang("xh", "isiXhosa", QW, currency="R")
lang("af", "Afrikaans", QW, accents="êëéèîïôöûü", currency="R")
lang("so", "Soomaali", QW)
lang("rw", "Kinyarwanda", QW)
lang("ny", "Chichewa", QW)
lang("sn", "chiShona", QW)
lang("st", "Sesotho", QW)
lang("mg", "Malagasy", QW, accents="àâèéêëìîïñôòöùû")
lang("mi", "Māori", QW, accents="āēīōū")
lang("ht", "Kreyòl ayisyen", QW, accents="èò")
lang("lb", "Lëtzebuergesch", ["qwertzuiopü", "asdfghjkléä", "yxcvbnmë"], currency="€")
lang("fy", "Frysk", QW, accents="âêéôûúäëïöü", currency="€")
lang("ga", "Gaeilge", QW, accents="áéíóú", currency="€")
lang("gd", "Gàidhlig", QW, accents="àèìòù", currency="£")
lang("cy", "Cymraeg", QW, accents="âêîôûŵŷáéíóúàèìòùäëïöü", currency="£")
lang("eo", "Esperanto", QW, accents="ĉĝĥĵŝŭ", popups={"c": "ĉ", "g": "ĝ", "h": "ĥ", "j": "ĵ", "s": "ŝ", "u": "ŭ"}, currency="€")
lang("la", "Latina", QW, accents="āēīōūȳ", currency="€")

# ---------------------------------------------------------------- Cyrillic
RU = ["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"]
lang("ru", "Русский", RU, popups={"е": "ё", "ь": "ъ"}, script="Cyrillic", currency="₽")
lang("uk", "Українська", ["йцукенгшщзхї", "фівапролджє", "ячсмитьбю"], popups={"г": "ґ", "ь": "ʼ"}, script="Cyrillic", currency="₴")
lang("be", "Беларуская", ["йцукенгшўзх", "фывапролджэ", "ячсмітьбю"], popups={"е": "ё", "ь": "ʼ"}, script="Cyrillic", currency="Br")
lang("bg", "Български", ["явертъуиопчш", "асдфгхйклщ", "зьцжбнмю"], script="Cyrillic", currency="лв")
lang("sr", "Српски", ["љњертзуиопшђ", "асдфгхјклчћж", "џцвбнм"], script="Cyrillic", currency="дин")
lang("mk", "Македонски", ["љњертѕуиопшѓ", "асдфгхјклчќж", "зџцвбнм"], script="Cyrillic", currency="ден")
lang("kk", "Қазақ", ["әіңғүұқөһ", "йцукенгшщзх", "фывапролджэ", "ячсмитьбю"], popups={"е": "ё", "ь": "ъ"}, script="Cyrillic", currency="₸")
lang("ky", "Кыргызча", RU, popups={"н": "ң", "о": "ө", "у": "ү", "е": "ё", "ь": "ъ"}, script="Cyrillic", currency="сом")
lang("tg", "Тоҷикӣ", ["йқукенгшҳзхъ", "фҳвапролджэ", "ячсмитӣбюғӯ"], popups={"ч": "ҷ", "е": "ё"}, script="Cyrillic", currency="SM")
lang("mn", "Монгол", ["фцужэнгшүзкъ", "йыбөахролдпе", "ячёсмитьвю"], script="Cyrillic", currency="₮")
lang("tt", "Татарча", RU, popups={"а": "ә", "о": "ө", "у": "ү", "ж": "җ", "н": "ң", "х": "һ", "е": "ё", "ь": "ъ"}, script="Cyrillic", currency="₽")
lang("ba", "Башҡортса", RU, popups={"а": "ә", "о": "ө", "у": "ү", "г": "ғ", "к": "ҡ", "н": "ң", "з": "ҙ", "с": "ҫ", "х": "һ", "е": "ё", "ь": "ъ"}, script="Cyrillic", currency="₽")

# ---------------------------------------------------------------- other alphabets
lang("el", "Ελληνικά", ["ςερτυθιοπ", "ασδφγηξκλ", "ζχψωβνμ"], popups={"α": "ά", "ε": "έ", "η": "ή", "ι": "ίϊΐ", "ο": "ό", "υ": "ύϋΰ", "ω": "ώ"}, script="Greek", currency="€")
lang("hy", "Հայերեն", ["է թ փ ձ ջ և ռ չ ճ ժ ծ", "ք ո ե ր տ ը ւ ի օ պ խ", "ա ս դ ֆ գ հ յ կ լ շ", "զ ղ ց վ բ ն մ"], script="Armenian", currency="֏")
lang("ka", "ქართული", ["ქწერტყუიოპ", "ასდფგჰჯკლ", "ზხცვბნმ"], popups={"წ": "ჭ", "რ": "ღ", "ტ": "თ", "ს": "შ", "ჯ": "ჟ", "ზ": "ძ", "ც": "ჩ"}, caseless=True, script="Georgian", currency="₾")
lang("he", "עברית", ["קראטוןםפ", "שדגכעיחלךף", "זסבהנמצתץ"], script="Hebrew", currency="₪")
lang("yi", "ייִדיש", ["קראטוןםפ", "שדגכעיחלךף", "זסבהנמצתץ"], popups={"א": "אַ אָ", "ו": "וּ װ", "י": "יִ ײ ײַ", "פ": "פּ פֿ", "ב": "בֿ", "כ": "כּ", "ש": "שׂ", "ת": "תּ"}, script="Hebrew", currency="₪")
lang("ar", "العربية", ["ض ص ث ق ف غ ع ه خ ح ج", "ش س ي ب ل ا ت ن م ك ط", "ذ ء ؤ ر ى ة و ز ظ د"],
     popups={"ا": "أ إ آ ٱ", "ي": "ئ", "ه": "ة", "و": "ؤ", "ل": "لا"}, script="Arabic", currency="ر.س")
lang("fa", "فارسی", ["ض ص ث ق ف غ ع ه خ ح ج چ", "ش س ی ب ل ا ت ن م ک گ", "ظ ط ز ر ذ د پ و"],
     popups={"ا": "آ أ ء", "ز": "ژ", "ی": "ئ ي", "ه": "ۀ ة", "و": "ؤ", "ک": "ك"}, script="Arabic", currency="﷼")
lang("ur", "اردو", ["ق و ع ر ت ے ء ی ہ پ", "ا س د ف گ ح ج ک ل", "ز ش چ ط ب ن م"],
     popups={"ت": "ٹ ث", "د": "ڈ ذ", "ر": "ڑ", "ن": "ں", "ہ": "ھ ۃ", "ی": "ئ", "ز": "ژ ظ ض", "س": "ص", "ع": "غ", "ک": "خ", "ا": "آ أ", "ے": "ۓ", "ط": "ظ", "ح": "خ"},
     script="Arabic", currency="₨")
lang("ckb", "کوردی", ["ق و ە ر ت ی ئ ح ۆ پ", "ا س د ف گ ه ژ ک ل", "ز خ ج ڤ ب ن م ش چ"],
     popups={"ر": "ڕ", "ل": "ڵ", "ی": "ێ", "ه": "ھ", "ع": "غ", "ح": "ع غ", "ز": "ض ظ", "س": "ص", "ت": "ط"}, script="Arabic")
lang("ps", "پښتو", ["ض ص ث ق ف غ ع ه خ ح ج چ", "ش س ی ب ل ا ت ن م ک ګ", "ظ ط ز ر ذ د پ و ښ ړ"],
     popups={"ا": "آ أ", "ی": "ي ې ۍ ئ", "ت": "ټ", "د": "ډ", "ن": "ڼ", "ز": "ژ ږ", "ج": "ځ", "چ": "څ", "ک": "ګ"}, script="Arabic", currency="؋")

# ---------------------------------------------------------------- Brahmic (InScript)
DEVA_N = ["ौ ै ा ी ू ब ह ग द ज ड ़", "ो े ् ि ु प र क त च ट ृ", "ॉ ं म न व ल स य"]
DEVA_S = ["औ ऐ आ ई ऊ भ ङ घ ध झ ढ ञ", "ओ ए अ इ उ फ ऱ ख थ छ ठ ऋ", "ऑ ँ ण ऩ ऴ श ष ः"]

def inscript(code, name, block, script, currency="₹", remap=None, popups=None):
    def conv(rows):
        out = []
        for r in rows:
            keys = []
            for k in r.split():
                c = chr(ord(k) - 0x900 + block) if 0x900 <= ord(k) < 0x980 else k
                c = (remap or {}).get(c, c)
                if ud.name(c, None):
                    keys.append(c)
            out.append(" ".join(keys))
        return out
    # short e/o (South Indian scripts), retroflex la: long-press, mapped by the same offset
    common = {"े": "ॆ", "ो": "ॊ", "ए": "ऎ", "ओ": "ऒ", "ल": "ळ"}
    pops = {}
    for k, v in common.items():
        ck, cv = conv([k])[0], conv([v])[0]
        if ck and cv:
            pops[ck] = cv
    pops.update(popups or {})
    lang(code, name, conv(DEVA_N), shift=conv(DEVA_S), caseless=True, script=script, currency=currency, popups=pops)

inscript("hi", "हिन्दी", 0x900, "Devanagari")
inscript("mr", "मराठी", 0x900, "Devanagari", popups={"ल": "ळ"})
inscript("ne", "नेपाली", 0x900, "Devanagari", currency="रू")
inscript("sa", "संस्कृतम्", 0x900, "Devanagari", popups={"ृ": "ॄ ॢ ॣ", "ं": "ः ँ ऽ"})
inscript("bn", "বাংলা", 0x980, "Bengali", currency="৳", popups={"ত": "ৎ", "য": "য়"})
inscript("as", "অসমীয়া", 0x980, "Bengali", remap={"র": "ৰ"}, popups={"ব": "ৱ", "ত": "ৎ", "য": "য়"})
inscript("pa", "ਪੰਜਾਬੀ", 0xA00, "Gurmukhi", popups={"ੰ": "ੱ ਂ"})
inscript("gu", "ગુજરાતી", 0xA80, "Gujarati")
inscript("or", "ଓଡ଼ିଆ", 0xB00, "Oriya", popups={"ଯ": "ୟ"})
inscript("ta", "தமிழ்", 0xB80, "Tamil", popups={"க": "ஃ"})
inscript("te", "తెలుగు", 0xC00, "Telugu")
inscript("kn", "ಕನ್ನಡ", 0xC80, "Kannada")
inscript("ml", "മലയാളം", 0xD00, "Malayalam", popups={"ന": "ൻ", "ര": "ർ", "ല": "ൽ", "ള": "ൾ", "ണ": "ൺ"})

lang("si", "සිංහල", ["ු අ ැ ර එ හ ම ස ද ච ඤ", "් ි ා ෙ ට ය ව න ක ත ග", "ං ජ ඩ ඉ බ ප ල ණ ළ"],
     shift=["ූ උ ෑ ඍ ඔ ශ ඹ ෂ ධ ඡ ඥ", "ෟ ී ෘ ෆ ඨ ් ළු ණ ඛ ථ ඝ", "ඞ ඣ ඪ ඊ භ ඵ ළ ඬ ඦ"], caseless=True, script="Sinhala", currency="රු")
lang("th", "ไทย", ["ๅ ภ ถ ุ ึ ค ต จ ข ช", "ๆ ไ ำ พ ะ ั ี ร น ย บ ล", "ฟ ห ก ด เ ้ ่ า ส ว ง", "ผ ป แ อ ิ ื ท ม ใ ฝ"],
     shift=["๑ ๒ ๓ ๔ ู ฿ ๕ ๖ ๗ ๘", "๐ ฎ ฑ ธ ํ ๊ ณ ฯ ญ ฐ ฅ", "ฤ ฆ ฏ โ ฌ ็ ๋ ษ ศ ซ", "ฉ ฮ ฺ ์ ฒ ฬ ฦ"],
     caseless=True, script="Thai", currency="฿")
lang("lo", "ລາວ", ["ຢ ຟ ໂ ຖ ຸ ູ ຄ ຕ ຈ ຂ ຊ ໍ", "ົ ໄ ຳ ພ ະ ິ ີ ຮ ນ ຍ ບ ລ", "ັ ຫ ກ ດ ເ ້ ່ າ ສ ວ ງ", "ຜ ປ ແ ອ ຶ ື ທ ມ ໃ ຝ"],
     shift=["໑ ໒ ໓ ໔ ໌ ຼ ໕ ໖ ໗ ໘ ໙ ໍ່", "ົ້ ໐ ຳ້ _ ຽ ິ້ ີ້ ຣ ໜ ຍ ໝ ລ", "ັ້ ໂ ຶ ເ ້ ່ ໊ ໋ ຫ ວ ງ", "ຜ ປ ແ ອ ຶ້ ື້ ຯ ໆ ໃ ຝ"],
     caseless=True, script="Lao", currency="₭")
lang("km", "ខ្មែរ", ["ឆ ឹ េ រ ត យ ុ ិ ោ ផ ៀ", "ា ស ដ ថ ង ហ ្ ក ល ើ ់", "ឋ ខ ច វ ប ន ម ុំ ។ ៊"],
     shift=["ឈ ឺ ែ ឬ ទ ួ ូ ី ៅ ភ ឿ", "ាំ ៃ ឌ ធ អ ះ ញ គ ឡ ោះ ៉", "ឍ ឃ ជ េះ ព ណ ំ ុះ ៕ ឱ"], caseless=True, script="Khmer", currency="៛")
lang("my", "မြန်မာ", ["ဆ တ န မ အ ပ က င သ စ ဟ", "ေ ျ ိ ် ါ ့ ြ ု ူ း", "ဖ ထ ခ လ ဘ ည ာ ယ"],
     shift=["ဈ ဝ ဣ ၎ ဤ ၌ ဥ ၍ ဿ ဏ ဧ", "ဗ ှ ီ ္ ွ ံ ဲ ဒ ဓ ဂ", "ဇ ဌ ဃ ဠ ဦ ဉ ဪ ၏"], caseless=True, script="Myanmar", currency="K")

ETH = "ሀ ለ ሐ መ ሠ ረ ሰ ሸ ቀ በ ቨ ተ ቸ ኀ ነ ኘ አ ከ ኸ ወ ዐ ዘ ዠ የ ደ ጀ ገ ጠ ጨ ጰ ጸ ፀ ፈ ፐ".split()
def ethiopic(code, name, currency):
    pops = {}
    for b in ETH:
        orders = [chr(ord(b) + i) for i in range(1, 8)]
        pops[b] = " ".join(c for c in orders if ud.name(c, None))
    rows = [" ".join(ETH[0:12]), " ".join(ETH[12:23]), " ".join(ETH[23:])]
    lang(code, name, rows, popups=pops, caseless=True, script="Ethiopic", currency=currency)
ethiopic("am", "አማርኛ", "ብር")
ethiopic("ti", "ትግርኛ", "ናቕፋ")

lang("ko", "한국어", ["ㅂㅈㄷㄱㅅㅛㅕㅑㅐㅔ", "ㅁㄴㅇㄹㅎㅗㅓㅏㅣ", "ㅋㅌㅊㅍㅠㅜㅡ"],
     shift=["ㅃㅉㄸㄲㅆㅛㅕㅑㅒㅖ", "ㅁㄴㅇㄹㅎㅗㅓㅏㅣ", "ㅋㅌㅊㅍㅠㅜㅡ"], caseless=True, script="CJK", composer="Hangul", currency="₩")
lang("ja", "日本語", QW, script="CJK", composer="Kana", currency="¥", conv="ja")
lang("zh-Hans", "中文 (拼音)", QW, script="CJK", composer="Pinyin", currency="¥", conv="zh-hans")
lang("zh-Hant", "中文 (繁體 拼音)", QW, script="CJK", composer="Pinyin", currency="NT$", conv="zh-hant")
lang("zh-TW", "中文 (注音)", ["ㄅ ㄉ ˇ ˋ ㄓ ˊ ˙ ㄚ ㄞ ㄢ ㄦ", "ㄆ ㄊ ㄍ ㄐ ㄔ ㄗ ㄧ ㄛ ㄟ ㄣ", "ㄇ ㄋ ㄎ ㄑ ㄕ ㄘ ㄨ ㄜ ㄠ ㄤ", "ㄈ ㄌ ㄏ ㄒ ㄖ ㄙ ㄩ ㄝ ㄡ ㄥ"],
     caseless=True, script="CJK", composer="Zhuyin", currency="NT$", conv="zh-hant")
lang("yue", "粵語 (粵拼)", QW, script="CJK", composer="Jyutping", currency="HK$", conv="yue")

# ---------------------------------------------------------------- emit
def rs(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'

def rows_rs(rows):
    return "&[" + ", ".join("&[" + ", ".join(rs(k) for k in r) + "]" for r in rows) + "]"

lm_alias = {"ko": "ko", "en-GB": "en", "de-CH": "de", "fr-CA": "fr", "es-419": "es", "pt-BR": "pt", "sr-Latn": "sr"}
out = ["// SPDX-License-Identifier: GPL-3.0-or-later",
       "// @generated by tools/gen_layouts.py — edit the generator, not this file.",
       "use super::{Composer, LangDef};", "",
       "pub static LANGUAGES: &[LangDef] = &["]
for e in L:
    lm = lm_alias.get(e["code"], e["code"])
    lm = lm if lm in LM and e["script"] != "CJK" else ""
    pops = ", ".join(f"({rs(k)}, &[{', '.join(rs(x) for x in v)}])" for k, v in e["pop"].items())
    out.append(
        f"    LangDef {{ code: {rs(e['code'])}, name: {rs(e['name'])}, lm: {rs(lm)}, rows: {rows_rs(e['rows'])}, "
        f"shift_rows: {rows_rs(e['shift'])}, popups: &[{pops}], caseless: {str(e['caseless']).lower()}, "
        f"script: {rs(e['script'])}, composer: Composer::{e['composer']}, turkic: {str(e['turkic']).lower()}, currency: {rs(e['currency'])}, conv: {rs(e['conv'])} }},")
out.append("];")
OUT.write_text("\n".join(out) + "\n")
print(f"{len(L)} languages, {sum(1 for e in L if (lm_alias.get(e['code'], e['code']) in LM))} with a language model")
