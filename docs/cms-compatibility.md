# Framework and CMS compatibility

The end goal is that these projects run on phpx **unmodified**, with output
identical to PHP 8.5. Status as of 2026-10-25.

| Project | Parses (`phpx check`) | Runs | Status |
|---|---|---|---|
| Laravel | yes | Probe scripts byte-identical to PHP | Full application request cycle needs the HTTP server mode and DB drivers |
| WordPress (latest) | **100%** of 1,512 files | `wp-admin/setup-config.php` renders (no DB) — comparison with PHP in progress | Boot blocked on missing built-ins, then MySQL |
| Drupal 11.2.5 core | **100%** of 10,656 files | Not attempted | Needs PDO, Symfony components' extension use, Reflection |
| Joomla | to verify | Not attempted | Expected to need the same as WordPress plus PDO |
| WooCommerce + major WP plugins | to verify | Not attempted | Same requirements as WP core, plus more extensions (curl, gd/imagick, intl, …) |

## Laravel

- Probe scripts exercising the framework run byte-identical to PHP.
- Composer's autoloader is replaced by a native implementation
  (classmap, PSR-4, PSR-0, `files`) — see
  [architecture.md](architecture.md#composer).
- Everything Laravel's core leans on in the language is supported:
  references, generators (LazyCollection), closure binding
  (`Closure::bind`), magic methods, enums, readonly,
  first-class callables, `static` closures.
- Remaining for real applications:

| Requirement | Status |
|---|---|
| HTTP server mode with per-request superglobals, headers, cookies, sessions | Not started (roadmap 3) |
| PDO (MySQL, SQLite, Postgres) | Not started (roadmap 2) |
| `serialize`/`unserialize` (cache, queue, session) | In progress |
| DateTime family (Carbon) | In progress |
| Reflection (container autowiring) | Not started — **blocker** for the service container resolving constructor dependencies |
| `hash`/`hash_hmac`/`openssl_*` (encryption, signed URLs) | `hash*` in progress; openssl not started |
| SPL: `ArrayObject`, `SplObjectStorage`, iterators | Not started |
| `mb_*` completeness, `intl` | Partial / not started |

## WordPress

### Parsing

Before this round of work, 251 WordPress files were rejected, almost all
because of `global` (1,147 uses) and `$GLOBALS` (490 uses), which phpx used
to refuse. Supporting `global`, `$GLOBALS`, variable variables, `extract()`
and `eval()` brought WordPress to 100% parse-OK.

```sh
phpx check --summary path/to/wordpress
```

### Running

The probe target is `wp-admin/setup-config.php` (the install wizard), which
renders without a database. Blockers, in the order they are hit:

| # | Blocker | Status |
|---|---|---|
| 1 | File streams: `fopen`/`fread`/`fgets`/`fwrite`/…, `php://` streams, directory handles, `stat` family, stream contexts | **Done** (`streams.rs`) |
| 2 | HTTP output: `header`, `headers_sent`, `setcookie`, `http_response_code` | In progress |
| 3 | `serialize` / `unserialize` | In progress |
| 4 | `pack` / `unpack` | In progress |
| 5 | `parse_url` / `parse_str` | In progress |
| 6 | `version_compare` | In progress |
| 7 | `strspn` / `strcspn` / `sscanf` and other string scanners | In progress |
| 8 | `htmlentities` / `html_entity_decode` | In progress |
| 9 | `hash` / `hash_hmac` / `sha1` / `hash_equals` | In progress |
| 10 | `strtotime` / `gmdate` / `DateTime` | In progress |
| 11 | `array_replace_recursive` and other array gaps | In progress |
| 12 | mysqli (WordPress's `wpdb`) — and PDO for drop-ins | Not started |
| 13 | HTTP server mode (`phpx serve`): real per-request superglobals, headers, cookies, sessions | Not started |
| 14 | curl (WP HTTP API, updates) | Not started |
| 15 | gd / imagick (media uploads, thumbnails) | Not started |
| 16 | XML parser (`xml_parser_create`, SimpleXML — feeds, oEmbed, importers) | Not started |
| 17 | sodium (signature verification for updates) | Not started |
| 18 | mbstring / intl completeness | Partial |

About 150 missing built-ins make up items 2–11. Milestone: WordPress setup
page output identical to PHP, then a full install + front page against
MySQL.

## WooCommerce and plugins

Same requirements as WordPress core plus heavier use of extensions:
curl (payment gateways, REST calls), gd/imagick, intl (`NumberFormatter`),
`DateTime`, `ArrayObject`/SPL, Action Scheduler (cron, `serialize`), and
`mb_*`. Expected to follow once core WordPress runs. `phpx check` results
for WooCommerce and popular plugins: to verify.

## Drupal

Drupal 11.2.5 core (10,656 files) parses 100%. Running it needs PDO,
Reflection, the DateTime family, SPL data structures and iterators, and the
Symfony components' expectations (`ArrayObject`, `SplObjectStorage`,
`hash`, `openssl`, `intl`). Not attempted yet.

## Joomla

Not yet checked. Expected requirements: as WordPress plus PDO/mysqli,
Reflection and `DateTime`.

## How progress is measured

1. `phpx check --summary` on each codebase (parse-level).
2. Probe scripts / entry points run with `phpx run` and compared with
   `php` output using `tools/difftest/compare.sh` (output-level).
3. Eventually: full request flows in CI against PHP 8.5 output
   (roadmap item 8).
