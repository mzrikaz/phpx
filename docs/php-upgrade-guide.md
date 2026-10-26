# Upgrading phpx to a new PHP version

phpx promises byte-identical output to one PHP release (today **8.5.10**,
recorded in `crates/interp/tests/php/PHP_VERSION`). When PHP ships a new
version (8.6, 9.0, or a patch release), this is the procedure to move the
target. It assumes you know the pieces in [anatomy.md](anatomy.md) and the
procedures in [contributing-recipes.md](contributing-recipes.md).

The order matters: switch the reference binary, regenerate everything that
is generated, let the differential tests show what changed, then work
through PHP's change list.

## 1. Get the new reference PHP

- Install the new PHP CLI with **the same extensions** the generators read
  (at least: intl, mbstring, iconv, openssl, zlib, dom/libxml, xml,
  xmlwriter, fileinfo, tokenizer, session, PDO + pdo_mysql, redis). On the
  development machine `php` is a lerd wrapper (`~/.local/share/lerd/bin/php`)
  that runs PHP in a container and only sees files under `$HOME`
  ([testing.md](testing.md#the-php-command-in-this-environment)).
- Keep the old binary available while you work: diffing the two versions'
  behaviour is the fastest way to find changes.
- Every tool takes the binary explicitly or uses `php` from `PATH`:
  `tools/difftest/regen.sh /path/to/php`, `php tools/...`.

## 2. Collect what changed

Sources, most useful first:

| Source | Use it for |
|---|---|
| `UPGRADING` in php-src (branch `PHP-8.6` etc.) | The authoritative list: *Backward Incompatible Changes*, *New Features*, *Changed Functions*, *New Functions*, *New Classes and Interfaces*, *Removed Extensions and SAPIs*, *Other Changes to Extensions*, *New Global Constants*, *Changes to INI File Handling*, *Deprecated Functionality*. Work through it section by section; the checklist below follows it. |
| RFCs (wiki.php.net/rfc, "Implemented" for the version) | Exact semantics and edge cases of each new feature, and the precise deprecation messages. |
| `NEWS` in php-src | Per-release bug fixes. For **patch releases** (8.5.10 → 8.5.11) this is usually the whole list: grep it for behaviour phpx copies (date parsing, `json`, `mb_*`, DOM output, ...). |
| `.phpt` tests in php-src (`Zend/tests/`, `ext/*/tests/`) for the new features | Ready-made cases. `--FILE--` is the script, `--EXPECT--` the output; `--EXPECTF--` contains patterns (`%s`, `%d`) and needs a real run with the new PHP to get concrete output. Convert interesting ones into differential tests. |
| The php.net migration guide (appendix "Migrating from PHP 8.x") | A readable summary of `UPGRADING`. |
| `UPGRADING.INTERNALS` | Mostly C-API changes (irrelevant), but also signals changes in bundled libraries (PCRE2, libxml2, timelib, ICU, Unicode) whose output phpx reproduces. |

Then diff the two PHP builds mechanically. Save under `$HOME` (lerd):

```php
<?php // ~/.cache/phpx-upgrade/surface.php: run with each PHP version
$out = [];
foreach (get_defined_functions()['internal'] as $f) {
    $r = new ReflectionFunction($f);
    $out['function'][$f] = (string)$r . ($r->isDeprecated() ? ' DEPRECATED' : '');
}
foreach (array_merge(get_declared_classes(), get_declared_interfaces(), get_declared_traits()) as $c) { // enums are classes
    $r = new ReflectionClass($c);
    if ($r->isInternal()) $out['class'][$c] = (string)$r;
}
foreach (get_defined_constants(true) as $ext => $cs) {
    if ($ext === 'user') continue;
    foreach ($cs as $k => $v) $out['const'][$ext][$k] = var_export($v, true);
}
$out['ini'] = ini_get_all(null, false);
$out['ext'] = get_loaded_extensions();
echo json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES), "\n";
```

```sh
php-old -n surface.php > old.json; php-new -n surface.php > new.json
diff old.json new.json | less     # new/removed functions, classes, constants, ini; changed signatures
```

Run both with the same `-d extension=...` flags. `(string)ReflectionFunction`
includes parameter types and defaults, so changed signatures show up as
text diffs.

## 3. Bump the version identity

Everything that states the version, as of PHP 8.5.10. Find them all with:

```sh
grep -rn '8\.5\.10\|80510\|4\.5\.10' crates tools --include=*.rs --include=*.php --include=*.py
grep -rn '"8\.5\|PHP 8\.5\|8, 5\|MINOR_VERSION' crates --include=*.rs | grep -v _table.rs
```

| Where | What |
|---|---|
| `crates/interp/src/builtins.rs` `constants()` | `PHP_VERSION`, `PHP_VERSION_ID`, `PHP_MAJOR_VERSION`, `PHP_MINOR_VERSION`, `PHP_RELEASE_VERSION`, `PHP_EXTRA_VERSION` |
| `crates/interp/src/builtins.rs` | `phpversion()` |
| `crates/interp/src/core_more.rs` | `zend_version()` (Zend Engine version: 8.5 ↔ 4.5) |
| `crates/interp/src/misc_more.rs` | `phpinfo()` header |
| `crates/interp/src/server.rs` | `SERVER_SOFTWARE`, `X-Powered-By`, the "Development Server ... started" log line |
| `crates/cli/src/main.rs` | `phpx -v` text |
| `crates/interp/src/pdo.rs` | `PDO::ATTR_CLIENT_VERSION` (`mysqlnd 8.5.10`) |
| `crates/interp/src/ini.rs` | `DEFAULTS` ("PHP 8.5 CLI defaults") — compare with `ini_get_all()` of the new build |
| generated files | `consts_table.rs` (also contains `PHP_VERSION`), `arginfo_table.rs`, `date/zones_table.rs` headers — regenerated in step 4 |
| `crates/interp/tests/php/PHP_VERSION` | written by `tools/difftest/regen.sh` |
| comments | messages like "Constant ... is deprecated since 8.5" are PHP's text and stay as they are |

## 4. Regenerate generated tables

Run with the new PHP (outputs under the repo are fine; scratch files under
`$HOME`):

```sh
php -n tools/arginfo/dump.php > ~/.cache/phpx-upgrade/arginfo.json
tools/arginfo/gen.py ~/.cache/phpx-upgrade/arginfo.json          # -> crates/interp/src/arginfo_table.rs
php -n -d extension=intl tools/consts/gen.php > crates/interp/src/consts_table.rs
php -n tools/date/zones.php > crates/interp/src/date/zones_table.rs
php -n -d extension=intl tools/mbstring/case.php > crates/interp/src/mbcase_table.rs
php tools/intl/numfmt.php > crates/interp/src/intl/locales_table.rs
php -n -d extension=intl tools/intl/prelude.php > crates/interp/src/prelude_intl.php
tools/difftest/regen.sh       # every tests/php/*.out, *.exit and PHP_VERSION
```

(Adjust `-d extension=` flags to how the new build loads extensions; the
generator headers state what each needs. Fix the header string in
`tools/arginfo/gen.py`, which hard-codes "PHP 8.5 Reflection".)

Then read the diffs; each is a to-do list:

| Diff | Meaning |
|---|---|
| `arginfo_table.rs` | New functions (implement them), removed ones (remove them), changed parameter types, defaults, by-ref flags or names (named arguments!). phpx gets the new checks and error messages for free; built-in bodies that read a new parameter must be updated. Re-check the hand-written `BY_REF` list in `call.rs`. |
| `consts_table.rs` | New/removed/changed constants and the `DEPRECATED` list with PHP's exact messages. |
| `zones_table.rs`, `mbcase_table.rs`, `locales_table.rs` | New timezonedb, Unicode or ICU data. |
| `tests/php/*.out` | **Behaviour PHP changed** in code phpx already covers. Every changed `.out` is a failing test for phpx until fixed (or a test that printed something version-dependent — make it deterministic). |

## 5. Work through the checklist

Use `UPGRADING` and the surface diff from step 2. For each item: write a
differential test first (output generated by the new PHP), then implement.

### Syntax

- New tokens, operators, statements, modifiers, attributes:
  [recipe 7](contributing-recipes.md#7-add-a-syntax-feature) (lexer →
  parser → AST → interpreter → compiler reject/lower).
- Precedence: add the operator to `binary_op()` in `parser.rs` at the
  position `Zend/zend_language_parser.y` gives it; verify with the parser
  oracle once nikic/php-parser supports the version.
- Removed syntax becomes a parse error: match PHP's message where tests or
  users can see it, with an `E0002` diagnostic.
- Accepted-but-unsupported syntax gets an `E1xxx` code in `checks.rs`
  until implemented, so `phpx check` reports it.
- Outstanding from earlier versions (do these first, a newer PHP builds on
  them): the pipe operator `|>` (8.5) and property hooks (8.4, `E1008`);
  see [language-support.md](language-support.md) and
  [roadmap.md](roadmap.md).

### Semantics and engine behaviour

- Conversions, comparisons, arithmetic, string offsets: `ops.rs`,
  `value.rs` (`loose_eq`, `compare`, `parse_numeric`), and the int fast
  paths in `compiler/lower.rs` and `crates/jit` — a semantic change there
  must be mirrored in all three engines (or the compiler must decline).
- Class model rules (inheritance checks, readonly, enums, visibility,
  property/method resolution): `call.rs` `build_class`, `expr.rs`
  property access. Error texts for declaration-time errors come from
  `build_class`.
- Call semantics (named args, variadics, by-ref, type juggling, `strict_types`):
  `call.rs` `bind_args`, `check_param`, `coerce`, `type_matches`.
- PHP compiles some calls specially ("frameless" functions since 8.4:
  two-argument `min`/`max` with their own tie rules,
  `call_builtin_named` in `call.rs` and `fastbi::frameless_pick`). Check
  `Zend/zend_compile.c` for new special-cased functions.
- Uncaught-error and warning formats: `errors.rs`, `Interp::uncaught_message`.

### Deprecations

Get every message by running the new PHP; phpx must print the same text at
the same moment.

| Kind | Where phpx emits it |
|---|---|
| Deprecated constant (`#[\Deprecated]` on internal constants) | Automatic from `consts_table::DEPRECATED` after regenerating; class constants via `#[\Deprecated]` in the prelude (`const_deprecation` in `call.rs`) |
| Deprecated function | Not automatic: `dump.php` records `deprecated` but the table doesn't use it. Add `it.deprecated("Function x() is deprecated since 8.N, ...")` at the top of the built-in (see `ext.rs` `utf8_encode`). The surface diff's `DEPRECATED` markers list them. |
| Deprecated method of a built-in class | `__phpx_deprecated("...")` in the prelude method, or `it.deprecated` in a native method |
| Deprecated parameter value / argument combination | In the built-in body (e.g. `ord()` in `strings.rs`) or in `arginfo.rs` if it is a generic rule (the null-to-non-nullable rule lives there) |
| Deprecated language construct | Interpreter (`expr.rs`/`interp.rs`) at run time. PHP raises many of these **at compile time**, i.e. when the file is compiled, before any of its output and once per occurrence — reproduce that order (the backtick deprecation in `process.rs` shows a once-per-site pattern). |

### New functions, classes, constants

- Functions: [recipe 3](contributing-recipes.md#3-add-a-built-in-function).
  Prioritize what Laravel, Symfony, WordPress and Drupal start using
  (they adopt new functions behind `function_exists` polyfills, so a
  missing function usually still works through the polyfill — test with
  and without).
- Classes and interfaces: [recipe 4](contributing-recipes.md#4-add-a-built-in-class).
  New methods on existing classes go in the same prelude declaration.
- Constants: regenerated in step 4; phpx-decided ones in
  `builtins::constants()`.
- New bundled extension: add it to `builtins::EXTENSIONS` once its
  functions exist, and to `$exts` in `tools/consts/gen.php`.

### Changed signatures and behaviour of existing functions

- The `arginfo_table.rs` diff shows type/default/name changes.
- `UPGRADING` → *Changed Functions* lists behaviour changes (new flags,
  new return values, exceptions instead of warnings): update the
  built-in and test both old and new cases.
- `ValueError`/`TypeError` messages: grep the old text in
  `crates/interp/src`.

### Removed features

- Removed functions/classes/constants: unregister them (`t.insert` lines,
  prelude declarations) so `function_exists()`/`class_exists()`/
  `defined()` return false and calls throw PHP's "Call to undefined
  function". Regenerated tables drop them automatically.
- Removed ini settings: drop from `DEFAULTS` (`ini_set` then returns false).

### ini

- New directives and changed defaults: `DEFAULTS` in `ini.rs`; effects in
  `ini_apply` ([recipe 6](contributing-recipes.md#6-add-an-ini-setting)).

### Bundled libraries

phpx reimplements several libraries byte-for-byte; when PHP bundles a new
version, compare outputs:

| Library | phpx port |
|---|---|
| timelib / timezonedb | `date/` (`parse.rs`, `fromformat.rs`, `zones_table.rs`, `abbr_map.rs`) |
| libxml2 (DOM, XML, XMLWriter) | `dom/html_parse.rs`, `dom/tables.rs`, `dom/save.rs`, `dom/xml_parse.rs`, `xmlwriter.rs` (tied to libxml2 2.13 today) |
| PCRE2 | `preg.rs` uses the real library (check the crate version against PHP's bundled PCRE2 when behaviour differs) |
| ICU / Unicode | `intl/`, `mbcase.rs` + tables, `intl/grapheme.rs` |
| Zend sort | `zsort.rs` |
| ext/random engines | `random.rs` |

### Native compiler and JIT

New AST nodes must at least be **rejected** in `compiler/analyze.rs` and
the JIT's `infer.rs`; compile them later. Any semantic change to an
operation that has an inline fast path (int arithmetic, comparisons,
packed-array reads, property caches, `fastbi.rs`) must be applied there
too or the fast path must decline that case.

## 6. Verify

1. Differential suite in all three modes ([testing.md](testing.md)):

   ```sh
   cargo test -p phpx-interp --test differential
   PHPX_NATIVE=all cargo test -p phpx-interp --test differential
   PHPX_NATIVE=off PHPX_NOJIT=1 cargo test -p phpx-interp --test differential
   ```

2. Whole workspace: `cargo test`.
3. Parser over real code: `phpx check --summary` on WordPress and Drupal
   core, and on the new PHP version's own test corpus if useful
   ([cms-compatibility.md](cms-compatibility.md)).
4. Frameworks: re-run the Laravel/Filament page sweeps and test suites
   described in [cms-compatibility.md](cms-compatibility.md) against the
   new PHP.
5. Robustness: `phpx tools/fuzz/all_builtins.php` (new built-ins must
   not panic) and `cargo test -p phpx-interp --test fuzz`.
6. Performance: `tools/bench/run.sh` against the new PHP
   ([performance.md](performance.md)); a new PHP is often faster, so the
   baseline table needs re-measuring.

## 7. Update the docs

Find every mention of the old version: `grep -rn '8\.5' docs README.md`
(historical entries in `changelog.md` stay as they are).

| Doc | Update |
|---|---|
| [php-compatibility.md](php-compatibility.md) | Target version and reference build; remove fixed gaps; add new known gaps |
| [language-support.md](language-support.md) | Rows for new syntax (interpreter and native-compiler columns) |
| [stdlib.md](stdlib.md) | Rerun the coverage procedure with the new PHP and `tools/stdlib_coverage.py` |
| [testing.md](testing.md) | The reference build named there |
| [configuration.md](configuration.md) | New or changed ini defaults |
| [roadmap.md](roadmap.md) | Check off the version items, add what is left |
| [changelog.md](changelog.md) | A milestone entry: "Target PHP 8.N" with what was added |
| [README.md](README.md), top-level `README.md` | "byte-identical to PHP 8.N" |
| [anatomy.md](anatomy.md), [contributing-recipes.md](contributing-recipes.md), this guide | Version numbers and any new file or step |
