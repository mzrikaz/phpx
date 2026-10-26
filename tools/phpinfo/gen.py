#!/usr/bin/env python3
"""Generate crates/interp/src/phpinfo_data.rs from a real PHP 8.5 build.

Inputs (captured from PHP itself, kept out of the repository because they
hold machine paths and environment variables), in DIR:

  fpm_phpinfo.html  phpinfo() of PHP-FPM (HTML: tells headers from rows)
  fpm_ini.json      {"all": ini_get_all(null, true), "ext": {ext: [names]}} under FPM
  cli_ini.json      the same under `php -n` (built-in defaults, CLI SAPI)
  cli_phpinfo.txt   `php -n -r 'phpinfo();'` (the text form of credits/license)

Usage: gen.py DIR > crates/interp/src/phpinfo_data.rs

Directive defaults come from `php -n` (phpx follows the CLI SAPI) and,
for extensions only FPM loads, from FPM. Only the modules phpx
implements are emitted (MODULES below).
"""
import html
import json
import re
import sys

# The extensions phpx implements (phpinfo() order is by name, case-insensitively).
MODULES = [
    "Core", "ctype", "date", "dom", "fileinfo", "filter", "hash", "intl", "json", "libxml",
    "mbstring", "mysqlnd", "openssl", "pcre", "PDO", "pdo_mysql", "random", "redis",
    "Reflection", "session", "SPL", "standard", "tokenizer", "xml", "xmlwriter", "zlib",
]

d = sys.argv[1]
page = open(f"{d}/fpm_phpinfo.html").read()
fpm = json.load(open(f"{d}/fpm_ini.json"))
cli = json.load(open(f"{d}/cli_ini.json"))


def text(cell):
    """Cell HTML -> plain text (as PHP printed it before escaping)."""
    cell = re.sub(r"<br\s*/?>", "\n", cell)
    cell = re.sub(r"<[^>]+>", "", cell)
    return html.unescape(cell)


def rows_of(table):
    out = []
    for tr in re.findall(r"<tr[^>]*>.*?</tr>", table, re.S):
        if tr.startswith('<tr class="h">'):
            span = re.match(r'<tr class="h"><th colspan="2">(.*?)</th></tr>', tr, re.S)
            if span:
                out.append(("HeaderSpan", [text(span.group(1))]))
            else:
                out.append(("Header", [text(c) for c in re.findall(r"<th>(.*?)</th>", tr, re.S)]))
        elif tr.startswith('<tr class="v">'):
            body = re.match(r'<tr class="v"><td>\n?(.*?)</td></tr>', tr, re.S).group(1)
            out.append(("Note", [text(x) for x in re.split(r"<br\s*/?>", body)]))
        else:
            cells = re.findall(r'<td class="([ev])">(.*?)</td>', tr, re.S)
            if len(cells) == 1:
                out.append(("Label", [text(cells[0][1])[:-1]]))
            else:
                # (info.c adds one space after each cell.)
                out.append(("Row", [text(c)[:-1] if text(c).endswith(" ") else text(c) for _, c in cells]))
    return out


# Module rows whose value is the current configuration: the renderer
# replaces these placeholders.
LIVE = {"Path to sendmail": "{ini:sendmail_path}", "Default timezone": "{timezone}"}
# Rows describing what is installed: phpx's own answer, not the FPM build's.
PHPX = {
    "API Extensions": "pdo_mysql",
    "PDO drivers": "mysql",
    "Registered save handlers": "files user ",
    "Registered serializer handlers": "php_serialize php php_binary ",
}

modules = {}
display = {}
# Directive -> the module whose phpinfo() section lists it.
module_of = {}
for m in re.finditer(r'<h2><a name="module_([^"]+)" href="#module_[^"]+">([^<]+)</a></h2>(.*?)(?=<h2|<h1)', page, re.S):
    name, body = m.group(2), m.group(3)
    tables = []
    for t in re.findall(r"<table>(.*?)</table>", body, re.S):
        rows = rows_of(t)
        if rows and rows[0] == ("Header", ["Directive", "Local Value", "Master Value"]):
            tables.append([("Directives", [])])
            for tr in re.findall(r"<tr>(.*?)</tr>", t, re.S):
                cells = re.findall(r'<td class="[ev]">(.*?)</td>', tr, re.S)
                display[text(cells[0])] = cells[1]
                module_of[text(cells[0])] = name
        else:
            tables.append(rows)
    modules[name.lower()] = (name, tables)

# Modules the `php -n` CLI build has: their directives are the CLI's set
# (FPM can add SAPI-specific ones, like SMTP).
cli_modules = {m.lower() for m in re.findall(r"^(\S[^\n]*)\n\n", open(f"{d}/cli_phpinfo.txt").read(), re.M)}

wanted = {m.lower() for m in MODULES}
lower_name = {m.lower(): m for m in MODULES}
ini = []
for name, info in sorted(fpm["all"].items()):
    module = module_of.get(name)
    if module is None or module.lower() not in wanted:
        continue
    if module.lower() in cli_modules and name not in cli["all"]:
        continue
    raw = cli["all"][name]["global_value"] if name in cli["all"] else info["global_value"]
    shown = display.get(name, "")
    if "<span style=\"color:" in shown:
        kind = "Color"
    elif name == "display_errors":
        kind = "DisplayErrors"
    elif shown in ("On", "Off") and (raw in (None, "", "0", "1") or str(raw).lower() in ("on", "off")):
        kind = "Bool"
    else:
        kind = "Plain"
    ini.append((name, raw, info["access"], lower_name[module.lower()], kind))


# Static page parts: the style block, logos, credits and license.
head = page[page.index("<style"):page.index("<title>")]
php_logo = re.search(r'alt="PHP logo"', page) and re.search(r'<img src="(data:image/png;base64,[^"]+)" alt="PHP logo"', page).group(1)
zend_logo = re.search(r'<img src="(data:image/png;base64,[^"]+)" alt="Zend logo"', page).group(1)
credits_html = page[page.index("<h1>PHP Credits</h1>"):page.index("<h2>PHP License</h2>")]
license_html = page[page.index("<h2>PHP License</h2>"):page.index("</div></body></html>")]
txt = open(f"{d}/cli_phpinfo.txt").read()
credits_text = txt[txt.index("PHP Credits\n"):txt.index("\nPHP License\n") + 1]
license_text = txt[txt.index("\nPHP License\n") + 1:]


def lit(s):
    return json.dumps(s, ensure_ascii=False)


out = []
w = out.append
w("//! php.ini directives and phpinfo() module sections of PHP 8.5.10, for the")
w("//! extensions phpx implements. Generated by tools/phpinfo/gen.py; do not edit.")
w("")
w("use super::phpinfo::{Display, IniDef, ModuleInfo, Row};")
w("")
w("pub const INI: &[IniDef] = &[")
for name, raw, access, module, kind in ini:
    v = "None" if raw is None else f"Some({lit(raw)})"
    w(f"    IniDef {{ name: {lit(name)}, value: {v}, access: {access}, module: {lit(module)}, display: Display::{kind} }},")
w("];")
w("")
w("pub const MODULES: &[ModuleInfo] = &[")
for m in sorted(MODULES, key=str.lower):
    if m.lower() not in modules:
        sys.exit(f"module {m} not in phpinfo()")
    name, tables = modules[m.lower()]
    w(f"    ModuleInfo {{ name: {lit(name)}, tables: &[")
    for t in tables:
        rs = []
        for kind, cells in t:
            if kind == "Directives":
                rs.append("Row::Directives")
            elif kind in ("HeaderSpan", "Label"):
                rs.append(f"Row::{kind}({lit(cells[0])})")
            else:
                # Rows that show configuration: filled in when printed.
                if kind == "Row" and cells[0] in LIVE:
                    cells = [cells[0], LIVE[cells[0]]]
                if kind == "Row" and cells[0] in PHPX:
                    cells = [cells[0], PHPX[cells[0]]]
                rs.append(f"Row::{kind}(&[{', '.join(lit(c) for c in cells)}])")
        w(f"        &[{', '.join(rs)}],")
    w("    ] },")
w("];")
w("")
for name, v in [("STYLE", head), ("PHP_LOGO", php_logo), ("ZEND_LOGO", zend_logo), ("CREDITS_HTML", credits_html),
                ("LICENSE_HTML", license_html), ("CREDITS_TEXT", credits_text), ("LICENSE_TEXT", license_text)]:
    w(f"pub const {name}: &str = {lit(v)};")
print("\n".join(out))
