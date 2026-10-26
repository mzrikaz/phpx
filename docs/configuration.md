# Configuration

phpx keeps PHP's configuration model so existing projects and hosting
setups work unchanged, and adds one optional file for phpx-only settings.

| What | File | Status |
|---|---|---|
| Runtime settings (`memory_limit`, `date.timezone`, `error_reporting`, ...) | `php.ini` (same syntax as PHP) | implemented |
| One-off overrides | `phpx run -d key=value`, `-n` (ignore php.ini) | implemented |
| In-script changes | `ini_set()`, `ini_get()`, `ini_restore()`, `ini_get_all()` | implemented |
| Dependencies / autoload (the `package.json` of PHP) | `composer.json` + `vendor/autoload.php` | used as-is (Composer's own autoloader; `PHPX_NATIVE_COMPOSER=1` swaps in a built-in one) |
| phpx-only settings (server, workers, compiler tiers) | `phpx.json` next to `composer.json` | planned |

## php.ini lookup

First match wins:

1. `PHPRC`: a php.ini file, or a directory containing `php.ini`.
2. `php.ini` in the script's directory, then each parent directory up to the
   project root (the nearest directory containing `composer.json`).
3. Built-in defaults (PHP 8.5 CLI defaults without a php.ini, e.g.
   `memory_limit=128M`, `max_execution_time=0`, `error_reporting=E_ALL`).

So a project-level `php.ini` at the repository root (next to
`composer.json`) applies to every script in the project, much like a
`package.json`-scoped setting in Node. `-d key=value` overrides the file;
`-n` skips it. `php_ini_loaded_file()` reports which file was used.

Supported syntax: `;`/`#` comments, `[sections]` (ignored for settings,
returned by `parse_ini_*` with `$process_sections`), quoted values,
`on/off/yes/no/true/false/none` booleans, inline `;` comments.

## Settings with effect today

| Setting | Effect |
|---|---|
| `memory_limit` | Enforced with PHP's fatal error (see note) |
| `date.timezone` | `date_default_timezone_get()` (DateTime support is in progress) |
| `include_path` | `get_include_path()`, `stream_resolve_include_path()` |
| `error_reporting` | `error_reporting()` value (warning filtering: to do) |
| `zend.enable_gc` | Turns the cycle collector on/off |
| `unserialize_max_depth` | default depth of `unserialize()` |

Every other PHP setting is stored and returned by `ini_get()` so code that
reads it works, but has no effect yet.

Note on `memory_limit`: phpx objects currently take about 3x PHP's memory
(see [security-and-memory.md](security-and-memory.md)), so the limit is
enforced at 3x the configured value; error messages show the configured
value. This goes back to 1x once compact object layout lands.

## Planned: phpx.json

```json
{
  "serve": { "listen": "0.0.0.0:8080", "root": "public", "workers": "auto" },
  "native": { "mode": "auto" },
  "ini": { "memory_limit": "256M" }
}
```

phpx-specific options only; PHP settings stay in php.ini (or `ini` here as a
convenience). Not implemented yet.
