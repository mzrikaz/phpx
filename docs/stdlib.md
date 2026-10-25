# Standard library coverage

phpx implements PHP's built-in functions natively in Rust
(`crates/interp/src/{builtins,strings,arrays,json,preg,fs,streams}.rs`) and
built-in classes either in PHP (`crates/interp/src/prelude.php`) or as
native methods (`crates/interp/src/native.rs`, `generator.rs`).

> **Snapshot: 2026-10-25.** The stdlib is being extended right now (the
> WordPress boot work below), so counts move quickly. Regenerate this page's
> numbers with the procedure in [Regenerating the coverage
> data](#regenerating-the-coverage-data).

## Summary

- **398** PHP functions implemented, out of the 1,970 functions exposed by
  the reference PHP 8.5 build (which includes many optional extensions).
  Counting only `Core` + `standard`, phpx has 323 of 606.
- 2 phpx-only functions: `async()`, `defer()` ([async.md](async.md)).
- 3 internal names that leak into user space and should not:
  `log2`, `lcg`, `__phpx_stack_remaining` (PHP has none of these).
- Language constructs (`isset`, `empty`, `unset`, `eval`, `include`,
  `exit`/`die`, `list`, `print`, `echo`) are handled by the parser and
  interpreter, not as functions.
- Functions that read or write the caller's scope (`compact`, `extract`,
  `eval`, `func_get_args`, `func_num_args`, `get_called_class`,
  `get_defined_vars`) are intercepted by the call machinery; calling them
  dynamically (e.g. `call_user_func('extract', …)`) throws
  `Error: Cannot call scope-reading function dynamically`.

## Coverage by extension

Function counts per extension as reported by `get_extension_funcs()` in the
reference PHP 8.5 build. Debug/profiling extensions (xdebug, pcov, SPX,
OPcache, igbinary, mongodb) are omitted.

| Extension | Implemented | PHP 8.5 total |
|---|---:|---:|
| standard | 290 | 544 |
| Core | 33 | 62 |
| mbstring | 26 | 65 |
| ctype | 11 | 11 |
| pcre | 9 | 11 |
| random | 8 | 9 |
| SPL | 7 | 15 |
| date | 6 | 48 |
| json | 5 | 5 |
| iconv | 2 | 10 |
| posix | 1 | 39 |
| intl | 0 | 187 |
| sodium | 0 | 110 |
| mysqli | 0 | 106 |
| gd | 0 | 103 |
| openssl | 0 | 64 |
| ldap | 0 | 55 |
| gmp | 0 | 51 |
| xmlwriter | 0 | 42 |
| sockets | 0 | 37 |
| ftp | 0 | 36 |
| curl | 0 | 35 |
| zlib | 0 | 30 |
| pcntl | 0 | 28 |
| session | 0 | 23 |
| xml | 0 | 22 |
| hash | 0 | 20 |
| calendar | 0 | 18 |
| dba | 0 | 15 |
| bcmath | 0 | 14 |
| readline | 0 | 13 |
| bz2 | 0 | 10 |
| zip | 0 | 10 |
| libxml | 0 | 8 |
| filter | 0 | 7 |
| sysvmsg | 0 | 7 |
| sysvshm | 0 | 7 |
| fileinfo | 0 | 6 |
| shmop | 0 | 6 |
| exif | 0 | 4 |
| sysvsem | 0 | 4 |
| SimpleXML | 0 | 3 |
| dom | 0 | 2 |
| soap | 0 | 2 |
| tokenizer | 0 | 2 |
| PDO | 0 | 1 |

## Implemented and missing functions, by extension

### Core

Implemented: `class_exists`, `define`, `defined`, `enum_exists`, `error_reporting`, `exit`, `extension_loaded`, `func_get_args`, `func_num_args`, `function_exists`, `gc_collect_cycles`, `get_called_class`, `get_class`, `get_defined_vars`, `get_loaded_extensions`, `get_object_vars`, `get_parent_class`, `get_resource_id`, `get_resource_type`, `interface_exists`, `is_a`, `is_subclass_of`, `method_exists`, `property_exists`, `set_error_handler`, `set_exception_handler`, `strcasecmp`, `strcmp`, `strlen`, `strncasecmp`, `strncmp`, `trait_exists`, `trigger_error`

Missing: `class_alias`, `clone`, `debug_backtrace`, `debug_print_backtrace`, `die`, `func_get_arg`, `gc_disable`, `gc_enable`, `gc_enabled`, `gc_mem_caches`, `gc_status`, `get_class_methods`, `get_class_vars`, `get_declared_classes`, `get_declared_interfaces`, `get_declared_traits`, `get_defined_constants`, `get_defined_functions`, `get_error_handler`, `get_exception_handler`, `get_extension_funcs`, `get_included_files`, `get_mangled_object_vars`, `get_required_files`, `get_resources`, `restore_error_handler`, `restore_exception_handler`, `user_error`, `zend_version`

### standard

Implemented: `abs`, `addslashes`, `array_all`, `array_any`, `array_chunk`, `array_column`, `array_combine`, `array_count_values`, `array_diff`, `array_diff_assoc`, `array_diff_key`, `array_fill`, `array_fill_keys`, `array_filter`, `array_find`, `array_find_key`, `array_first`, `array_flip`, `array_intersect`, `array_intersect_key`, `array_is_list`, `array_key_exists`, `array_key_first`, `array_key_last`, `array_keys`, `array_last`, `array_map`, `array_merge`, `array_merge_recursive`, `array_pad`, `array_pop`, `array_product`, `array_push`, `array_rand`, `array_reduce`, `array_replace`, `array_reverse`, `array_search`, `array_shift`, `array_slice`, `array_splice`, `array_sum`, `array_unique`, `array_unshift`, `array_values`, `array_walk`, `arsort`, `asort`, `atan`, `atan2`, `base64_decode`, `base64_encode`, `base_convert`, `basename`, `bin2hex`, `bindec`, `boolval`, `call_user_func`, `call_user_func_array`, `ceil`, `chdir`, `chgrp`, `chmod`, `chop`, `chown`, `chr`, `clearstatcache`, `closedir`, `compact`, `constant`, `copy`, `cos`, `count`, `crc32`, `current`, `decbin`, `dechex`, `decoct`, `dirname`, `disk_free_space`, `disk_total_space`, `doubleval`, `end`, `error_log`, `exp`, `explode`, `extract`, `fclose`, `feof`, `fflush`, `fgetc`, `fgets`, `file`, `fileatime`, `filectime`, `file_exists`, `file_get_contents`, `filegroup`, `fileinode`, `filemtime`, `fileowner`, `fileperms`, `file_put_contents`, `filesize`, `filetype`, `floatval`, `flock`, `floor`, `flush`, `fmod`, `fnmatch`, `fopen`, `fpassthru`, `fputs`, `fread`, `fseek`, `fstat`, `ftell`, `ftruncate`, `fwrite`, `getcwd`, `get_debug_type`, `getenv`, `gettype`, `glob`, `hex2bin`, `hexdec`, `hrtime`, `htmlspecialchars`, `htmlspecialchars_decode`, `http_build_query`, `implode`, `in_array`, `ini_get`, `ini_set`, `intdiv`, `intval`, `is_array`, `is_bool`, `is_callable`, `is_countable`, `is_dir`, `is_double`, `is_executable`, `is_file`, `is_finite`, `is_float`, `is_infinite`, `is_int`, `is_integer`, `is_iterable`, `is_link`, `is_long`, `is_nan`, `is_null`, `is_numeric`, `is_object`, `is_readable`, `is_resource`, `is_scalar`, `is_string`, `is_uploaded_file`, `is_writable`, `join`, `key`, `key_exists`, `krsort`, `ksort`, `lcfirst`, `lchgrp`, `lchown`, `link`, `log`, `log10`, `lstat`, `ltrim`, `max`, `md5`, `memory_get_peak_usage`, `memory_get_usage`, `microtime`, `min`, `mkdir`, `move_uploaded_file`, `natcasesort`, `natsort`, `nl2br`, `number_format`, `ob_end_clean`, `ob_end_flush`, `ob_get_clean`, `ob_get_contents`, `ob_get_level`, `ob_start`, `octdec`, `opendir`, `ord`, `pathinfo`, `php_sapi_name`, `phpversion`, `pi`, `pow`, `printf`, `print_r`, `range`, `rawurldecode`, `rawurlencode`, `readdir`, `readfile`, `readlink`, `realpath`, `realpath_cache_size`, `rename`, `reset`, `rewind`, `rewinddir`, `rmdir`, `round`, `rsort`, `rtrim`, `scandir`, `settype`, `shuffle`, `sin`, `sizeof`, `sleep`, `sort`, `sprintf`, `sqrt`, `stat`, `str_contains`, `stream_context_create`, `stream_context_get_default`, `stream_context_get_options`, `stream_context_set_option`, `stream_get_contents`, `stream_get_meta_data`, `stream_get_wrappers`, `stream_isatty`, `stream_is_local`, `stream_set_blocking`, `stream_set_timeout`, `stream_set_write_buffer`, `str_ends_with`, `stripos`, `stripslashes`, `strip_tags`, `str_ireplace`, `strnatcasecmp`, `strnatcmp`, `str_pad`, `strpos`, `strrchr`, `str_repeat`, `str_replace`, `strrev`, `strrpos`, `str_split`, `str_starts_with`, `strstr`, `strtolower`, `strtoupper`, `strtr`, `strval`, `str_word_count`, `substr`, `substr_count`, `substr_replace`, `symlink`, `sys_get_temp_dir`, `tan`, `tempnam`, `tmpfile`, `touch`, `trim`, `uasort`, `ucfirst`, `ucwords`, `uksort`, `umask`, `uniqid`, `unlink`, `urldecode`, `urlencode`, `usleep`, `usort`, `var_dump`, `var_export`, `vsprintf`, `wordwrap`

Missing: `acos`, `acosh`, `addcslashes`, `array_change_key_case`, `array_diff_uassoc`, `array_diff_ukey`, `array_intersect_assoc`, `array_intersect_uassoc`, `array_intersect_ukey`, `array_multisort`, `array_replace_recursive`, `array_udiff`, `array_udiff_assoc`, `array_udiff_uassoc`, `array_uintersect`, `array_uintersect_assoc`, `array_uintersect_uassoc`, `array_walk_recursive`, `asin`, `asinh`, `assert`, `assert_options`, `atanh`, `checkdnsrr`, `chunk_split`, `cli_get_process_title`, `cli_set_process_title`, `closelog`, `connection_aborted`, `connection_status`, `convert_uudecode`, `convert_uuencode`, `cosh`, `count_chars`, `crypt`, `debug_zval_dump`, `deg2rad`, `dir`, `diskfreespace`, `dl`, `dns_check_record`, `dns_get_mx`, `dns_get_record`, `error_clear_last`, `error_get_last`, `escapeshellarg`, `escapeshellcmd`, `exec`, `expm1`, `fdatasync`, `fdiv`, `fgetcsv`, `forward_static_call`, `forward_static_call_array`, `fpow`, `fprintf`, `fputcsv`, `fscanf`, `fsockopen`, `fsync`, `ftok`, `get_browser`, `get_cfg_var`, `get_current_user`, `get_headers`, `gethostbyaddr`, `gethostbyname`, `gethostbynamel`, `gethostname`, `get_html_translation_table`, `getimagesize`, `getimagesizefromstring`, `get_include_path`, `getlastmod`, `get_meta_tags`, `getmxrr`, `getmygid`, `getmyinode`, `getmypid`, `getmyuid`, `getopt`, `getprotobyname`, `getprotobynumber`, `getrusage`, `getservbyname`, `getservbyport`, `gettimeofday`, `header`, `header_register_callback`, `header_remove`, `headers_list`, `headers_sent`, `hebrev`, `highlight_file`, `highlight_string`, `htmlentities`, `html_entity_decode`, `http_clear_last_response_headers`, `http_get_last_response_headers`, `http_response_code`, `hypot`, `ignore_user_abort`, `image_type_to_extension`, `image_type_to_mime_type`, `inet_ntop`, `inet_pton`, `ini_alter`, `ini_get_all`, `ini_parse_quantity`, `ini_restore`, `ip2long`, `iptcembed`, `iptcparse`, `is_writeable`, `levenshtein`, `linkinfo`, `localeconv`, `log1p`, `long2ip`, `mail`, `md5_file`, `memory_reset_peak_usage`, `metaphone`, `net_get_interfaces`, `next`, `nl_langinfo`, `ob_clean`, `ob_flush`, `ob_get_flush`, `ob_get_length`, `ob_get_status`, `ob_implicit_flush`, `ob_list_handlers`, `openlog`, `output_add_rewrite_var`, `output_reset_rewrite_vars`, `pack`, `parse_ini_file`, `parse_ini_string`, `parse_str`, `parse_url`, `passthru`, `password_algos`, `password_get_info`, `password_hash`, `password_needs_rehash`, `password_verify`, `pclose`, `pfsockopen`, `phpcredits`, `phpinfo`, `php_ini_loaded_file`, `php_ini_scanned_files`, `php_strip_whitespace`, `php_uname`, `popen`, `pos`, `prev`, `proc_close`, `proc_get_status`, `proc_nice`, `proc_open`, `proc_terminate`, `putenv`, `quoted_printable_decode`, `quoted_printable_encode`, `quotemeta`, `rad2deg`, `realpath_cache_get`, `register_shutdown_function`, `register_tick_function`, `request_parse_body`, `serialize`, `setcookie`, `set_file_buffer`, `set_include_path`, `setlocale`, `setrawcookie`, `set_time_limit`, `sha1`, `sha1_file`, `shell_exec`, `show_source`, `similar_text`, `sinh`, `socket_get_status`, `socket_set_blocking`, `socket_set_timeout`, `soundex`, `sscanf`, `strchr`, `strcoll`, `strcspn`, `str_decrement`, `stream_bucket_append`, `stream_bucket_make_writeable`, `stream_bucket_new`, `stream_bucket_prepend`, `stream_context_get_params`, `stream_context_set_default`, `stream_context_set_options`, `stream_context_set_params`, `stream_copy_to_stream`, `stream_filter_append`, `stream_filter_prepend`, `stream_filter_register`, `stream_filter_remove`, `stream_get_filters`, `stream_get_line`, `stream_get_transports`, `stream_register_wrapper`, `stream_resolve_include_path`, `stream_select`, `stream_set_chunk_size`, `stream_set_read_buffer`, `stream_socket_accept`, `stream_socket_client`, `stream_socket_enable_crypto`, `stream_socket_get_name`, `stream_socket_pair`, `stream_socket_recvfrom`, `stream_socket_sendto`, `stream_socket_server`, `stream_socket_shutdown`, `stream_supports_lock`, `stream_wrapper_register`, `stream_wrapper_restore`, `stream_wrapper_unregister`, `str_getcsv`, `str_increment`, `stripcslashes`, `stristr`, `strpbrk`, `strptime`, `strripos`, `str_rot13`, `str_shuffle`, `strspn`, `strtok`, `substr_compare`, `sys_getloadavg`, `syslog`, `system`, `tanh`, `time_nanosleep`, `time_sleep_until`, `unpack`, `unregister_tick_function`, `unserialize`, `utf8_decode`, `utf8_encode`, `version_compare`, `vfprintf`, `vprintf`

### mbstring

Implemented: `mb_check_encoding`, `mb_chr`, `mb_convert_case`, `mb_convert_encoding`, `mb_detect_encoding`, `mb_internal_encoding`, `mb_lcfirst`, `mb_ltrim`, `mb_ord`, `mb_rtrim`, `mb_scrub`, `mb_strimwidth`, `mb_stripos`, `mb_strlen`, `mb_str_pad`, `mb_strpos`, `mb_strrpos`, `mb_str_split`, `mb_strstr`, `mb_strtolower`, `mb_strtoupper`, `mb_strwidth`, `mb_substr`, `mb_substr_count`, `mb_trim`, `mb_ucfirst`

Missing: `mb_convert_kana`, `mb_convert_variables`, `mb_decode_mimeheader`, `mb_decode_numericentity`, `mb_detect_order`, `mb_encode_mimeheader`, `mb_encode_numericentity`, `mb_encoding_aliases`, `mb_ereg`, `mb_eregi`, `mb_eregi_replace`, `mb_ereg_match`, `mb_ereg_replace`, `mb_ereg_replace_callback`, `mb_ereg_search`, `mb_ereg_search_getpos`, `mb_ereg_search_getregs`, `mb_ereg_search_init`, `mb_ereg_search_pos`, `mb_ereg_search_regs`, `mb_ereg_search_setpos`, `mb_get_info`, `mb_http_input`, `mb_http_output`, `mb_language`, `mb_list_encodings`, `mb_output_handler`, `mb_parse_str`, `mb_preferred_mime_name`, `mb_regex_encoding`, `mb_regex_set_options`, `mb_send_mail`, `mb_split`, `mb_strcut`, `mb_stristr`, `mb_strrchr`, `mb_strrichr`, `mb_strripos`, `mb_substitute_character`

### ctype

Implemented: `ctype_alnum`, `ctype_alpha`, `ctype_cntrl`, `ctype_digit`, `ctype_graph`, `ctype_lower`, `ctype_print`, `ctype_punct`, `ctype_space`, `ctype_upper`, `ctype_xdigit`

### pcre

Implemented: `preg_grep`, `preg_last_error`, `preg_last_error_msg`, `preg_match`, `preg_match_all`, `preg_quote`, `preg_replace`, `preg_replace_callback`, `preg_split`

Missing: `preg_filter`, `preg_replace_callback_array`

### json

Implemented: `json_decode`, `json_encode`, `json_last_error`, `json_last_error_msg`, `json_validate`

### random

Implemented: `getrandmax`, `lcg_value`, `mt_getrandmax`, `mt_rand`, `mt_srand`, `rand`, `random_bytes`, `random_int`

Missing: `srand`

### SPL

Implemented: `class_implements`, `iterator_to_array`, `spl_autoload_functions`, `spl_autoload_register`, `spl_autoload_unregister`, `spl_object_hash`, `spl_object_id`

Missing: `class_parents`, `class_uses`, `iterator_apply`, `iterator_count`, `spl_autoload`, `spl_autoload_call`, `spl_autoload_extensions`, `spl_classes`

### date

Implemented: `checkdate`, `date`, `date_default_timezone_get`, `date_default_timezone_set`, `mktime`, `time`

Missing: `date_add`, `date_create`, `date_create_from_format`, `date_create_immutable`, `date_create_immutable_from_format`, `date_date_set`, `date_diff`, `date_format`, `date_get_last_errors`, `date_interval_create_from_date_string`, `date_interval_format`, `date_isodate_set`, `date_modify`, `date_offset_get`, `date_parse`, `date_parse_from_format`, `date_sub`, `date_sun_info`, `date_sunrise`, `date_sunset`, `date_time_set`, `date_timestamp_get`, `date_timestamp_set`, `date_timezone_get`, `date_timezone_set`, `getdate`, `gmdate`, `gmmktime`, `gmstrftime`, `idate`, `localtime`, `strftime`, `strtotime`, `timezone_abbreviations_list`, `timezone_identifiers_list`, `timezone_location_get`, `timezone_name_from_abbr`, `timezone_name_get`, `timezone_offset_get`, `timezone_open`, `timezone_transitions_get`, `timezone_version_get`

### iconv

Implemented: `iconv`, `iconv_strlen`

Missing: `iconv_get_encoding`, `iconv_mime_decode`, `iconv_mime_decode_headers`, `iconv_mime_encode`, `iconv_set_encoding`, `iconv_strpos`, `iconv_strrpos`, `iconv_substr`

### posix

Implemented: `posix_isatty`

Missing: `posix_access`, `posix_ctermid`, `posix_eaccess`, `posix_errno`, `posix_getcwd`, `posix_getegid`, `posix_geteuid`, `posix_getgid`, `posix_getgrgid`, `posix_getgrnam`, `posix_getgroups`, `posix_get_last_error`, `posix_getlogin`, `posix_getpgid`, `posix_getpgrp`, `posix_getpid`, `posix_getppid`, `posix_getpwnam`, `posix_getpwuid`, `posix_getrlimit`, `posix_getsid`, `posix_getuid`, `posix_initgroups`, `posix_kill`, `posix_mkfifo`, `posix_mknod`, `posix_setegid`, `posix_seteuid`, `posix_setgid`, `posix_setpgid`, `posix_setrlimit`, `posix_setsid`, `posix_setuid`, `posix_strerror`, `posix_sysconf`, `posix_times`, `posix_ttyname`, `posix_uname`

Every other extension in the table above (intl, sodium, mysqli, gd,
openssl, ldap, gmp, xmlwriter, sockets, ftp, curl, zlib, pcntl, session,
xml, hash, calendar, dba, bcmath, readline, bz2, zip, libxml, filter,
sysv*, fileinfo, shmop, exif, SimpleXML, dom, soap, tokenizer, PDO) has
**no** functions implemented yet.

## Notable gaps

Functions that real applications hit early, grouped by what needs them.
Items marked *(in progress)* are being implemented as part of the WordPress
boot milestone ([roadmap.md](roadmap.md)).

| Area | Missing functions | Needed by |
|---|---|---|
| HTTP / SAPI | `header`, `headers_sent`, `headers_list`, `header_remove`, `http_response_code`, `setcookie`, `setrawcookie` *(in progress)* | Every web app |
| Serialization | `serialize`, `unserialize` *(in progress)* | WordPress options/transients, Laravel cache/session/queue |
| Binary data | `pack`, `unpack` *(in progress)* | WordPress, PHPMailer, hashing libraries |
| URLs | `parse_url`, `parse_str` *(in progress)* | All CMSs, routers |
| Versions | `version_compare` *(in progress)* | WordPress boot, Composer packages |
| String scanning | `strspn`, `strcspn`, `sscanf`, `strtok`, `strpbrk`, `stristr`, `strripos`, `substr_compare`, `similar_text`, `levenshtein`, `addcslashes`, `chunk_split`, `str_getcsv` | Various |
| HTML entities | `htmlentities`, `html_entity_decode`, `get_html_translation_table` *(in progress)* | WordPress formatting (`kses`, `esc_*`) |
| Hashing | `sha1`, `md5_file`, `sha1_file`, `hash`, `hash_hmac`, `hash_equals`, `hash_algos`, `password_hash`/`password_verify`, `crypt` *(partly in progress)* | Auth, nonces, cache keys |
| Date/time | `strtotime`, `gmdate`, `getdate`, `localtime`, `idate`, `date_*`, `timezone_*`, classes `DateTime`, `DateTimeImmutable`, `DateTimeZone`, `DateInterval`, `DatePeriod` *(in progress)* | Carbon (Laravel), WordPress `current_time`, Drupal |
| Arrays | `array_replace_recursive` *(in progress)*, `array_walk_recursive`, `array_multisort`, `array_change_key_case`, `array_intersect_assoc`, `array_udiff*`, `array_uintersect*`, `array_*_ukey`, `next`, `prev`, `pos` | WordPress (`wp_parse_args`), Laravel `Arr` |
| Output buffering | `ob_get_length`, `ob_get_flush`, `ob_flush`, `ob_clean`, `ob_get_status`, `ob_implicit_flush`, `ob_list_handlers` | Templates, WordPress |
| Errors / debugging | `error_get_last`, `error_clear_last`, `restore_error_handler`, `restore_exception_handler`, `debug_backtrace`, `debug_print_backtrace`, `register_shutdown_function` | Laravel, WordPress, Drupal error handling |
| Class introspection | `get_class_methods`, `get_class_vars`, `get_declared_classes`, `class_alias`, `class_parents`, `class_uses`, `iterator_count`, `iterator_apply`, `get_included_files`, `get_defined_constants`, `get_defined_functions` | Frameworks, plugin loaders |
| Processes / system | `exec`, `shell_exec`, `system`, `passthru`, `proc_open`, `popen`, `escapeshellarg`, `getmypid`, `php_uname`, `set_time_limit`, `ignore_user_abort`, `gethostname` | WP-CLI, Laravel `Process`, cron |
| Locale / misc | `setlocale`, `localeconv`, `putenv`, `get_include_path`/`set_include_path`, `filter_var` (filter ext), `mail` | Various |
| Regex | `preg_replace_callback_array`, `preg_filter` | Laravel, Symfony |
| Math | `acos`, `asin`, `sinh`/`cosh`/`tanh`, `hypot`, `deg2rad`, `rad2deg`, `fdiv`, `srand` | Various |
| Streams / network | `fsockopen`, `stream_socket_client`/`_server`, `stream_select`, `stream_copy_to_stream`, `stream_get_line`, `stream_wrapper_register`, stream filters, `fgetcsv`/`fputcsv`, `fprintf`, `vprintf` | HTTP clients, CSV import/export |
| Extensions (whole) | mysqli, PDO, curl, gd/imagick, xml/xmlreader/xmlwriter/dom/SimpleXML, sodium, openssl, intl, zip, zlib, session, filter, fileinfo, bcmath, gmp, hash | Roadmap items 2, 3 and 6 |

## Built-in classes and interfaces

| Group | Implemented | Missing (notable) |
|---|---|---|
| Core interfaces | `Traversable`, `Iterator`, `IteratorAggregate`, `ArrayAccess`, `Countable`, `Stringable`, `JsonSerializable`, `UnitEnum`, `BackedEnum`, `Throwable` | `Serializable`, `OuterIterator`, `RecursiveIterator`, `SeekableIterator` |
| Core classes | `stdClass`, `Closure` (`bind`, `bindTo`, `call`, `fromCallable`, `__invoke`), `Generator` (`current`, `key`, `next`, `send`, `throw`, `valid`, `rewind`, `getReturn`) | `WeakMap`, `WeakReference`, `Fiber`, `__PHP_Incomplete_Class` |
| Errors | `Error`, `TypeError`, `ValueError`, `ArithmeticError`, `DivisionByZeroError`, `CompileError`, `ParseError`, `ArgumentCountError`, `UnhandledMatchError` | — |
| Exceptions | `Exception`, `ErrorException`, `JsonException`, SPL: `LogicException`, `BadFunctionCallException`, `BadMethodCallException`, `DomainException`, `InvalidArgumentException`, `LengthException`, `OutOfRangeException`, `RuntimeException`, `OutOfBoundsException`, `OverflowException`, `RangeException`, `UnderflowException`, `UnexpectedValueException` | `getTrace()` returns `[]` (no real backtraces) |
| SPL iterators / data structures | `ArrayIterator` (written in PHP) | `ArrayObject`, `SplStack`, `SplQueue`, `SplObjectStorage`, `SplFixedArray`, `SplPriorityQueue`, `SplHeap`, `SplDoublyLinkedList`, `IteratorIterator`, `FilterIterator`, `RecursiveArrayIterator`, `RecursiveIteratorIterator`, `RecursiveDirectoryIterator`, `DirectoryIterator`, `SplFileInfo`, `SplFileObject`, `LimitIterator`, `CallbackFilterIterator`, `AppendIterator`, `CachingIterator` |
| Reflection | — | All `Reflection*` classes |
| Date | — | `DateTime`, `DateTimeImmutable`, `DateTimeZone`, `DateInterval`, `DatePeriod`, `DateTimeInterface` |
| phpx async | `Promise`, `Concurrency`, `TimeoutException`, `CancelledException`, `AggregateException` | — |

Enum methods `cases()`, `from()`, `tryFrom()` are provided natively for
every enum.

## Regenerating the coverage data

The numbers above come from asking real PHP for every function it defines
and asking phpx which of those exist. The `php` command in this environment
is a lerd wrapper that runs inside a container and cannot see `/tmp`, so
keep the working files under `$HOME`:

```sh
mkdir -p ~/.cache/phpxdocs && cd ~/.cache/phpxdocs
cat > exts.php <<'PHP'
<?php
foreach (get_loaded_extensions() as $e)
    foreach ((get_extension_funcs($e) ?: []) as $f) echo $e, "\t", strtolower($f), "\n";
PHP
cat > probe.php <<'PHP'
<?php
foreach (file($argv[1]) as $l) { $n = trim($l); if ($n !== '' && function_exists($n)) echo $n, "\n"; }
PHP
php exts.php > exts.tsv                     # extension<TAB>function from PHP 8.5
cut -f2 exts.tsv > names.txt
/path/to/php/target/release/phpx run probe.php "$PWD/names.txt" > have.txt
wc -l < have.txt                             # functions phpx implements
# per-extension counts:
awk -F'\t' 'NR==FNR{h[$1]=1;next}{t[$1]++; if($2 in h) g[$1]++}
  END{for(e in t) printf "%s\t%d\t%d\n", e, g[e]+0, t[e]}' have.txt exts.tsv | sort -k2 -rn
```

A quick source-level count (includes phpx-only and internal names, misses
names registered in loops) is:

```sh
grep -ho 't\.insert("[^"]*"' crates/interp/src/*.rs | sort -u | wc -l
```
