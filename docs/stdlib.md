# Standard library coverage

phpx implements PHP's built-in functions natively in Rust
(`crates/interp/src/{builtins,strings,arrays,json,preg,fs,streams}.rs`) and
built-in classes either in PHP (`crates/interp/src/prelude.php`) or as
native methods (`crates/interp/src/native.rs`, `generator.rs`).

> **Snapshot: 2026-10-25.** Regenerate this page's numbers with the
> procedure in [Regenerating the coverage data](#regenerating-the-coverage-data)
> and `tools/stdlib_coverage.py`.

## Summary

- **541** PHP functions implemented, out of the 1,910 functions exposed by
  the reference PHP 8.5 build (which includes many optional extensions).
  Counting only `Core` + `standard`, phpx has 437 of 606.
- 2 phpx-only functions: `async()`, `defer()` ([async.md](async.md)).
- Internal helpers used by the PHP-written built-in classes are named
  `__phpx_*` and are invisible to `function_exists()` and
  `get_defined_functions()`.
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
| standard | 390 | 544 |
| Core | 47 | 62 |
| mbstring | 26 | 65 |
| SPL | 11 | 15 |
| ctype | 11 | 11 |
| hash | 10 | 20 |
| pcre | 9 | 11 |
| random | 9 | 9 |
| zlib | 8 | 30 |
| date | 6 | 48 |
| filter | 5 | 7 |
| json | 5 | 5 |
| iconv | 2 | 10 |
| openssl | 1 | 64 |
| posix | 1 | 39 |
| intl | 0 | 187 |
| sodium | 0 | 110 |
| mysqli | 0 | 106 |
| gd | 0 | 103 |
| ldap | 0 | 55 |
| gmp | 0 | 51 |
| xmlwriter | 0 | 42 |
| sockets | 0 | 37 |
| ftp | 0 | 36 |
| curl | 0 | 35 |
| pcntl | 0 | 28 |
| session | 0 | 23 |
| xml | 0 | 22 |
| calendar | 0 | 18 |
| dba | 0 | 15 |
| bcmath | 0 | 14 |
| readline | 0 | 13 |
| bz2 | 0 | 10 |
| zip | 0 | 10 |
| libxml | 0 | 8 |
| sysvmsg | 0 | 7 |
| sysvshm | 0 | 7 |
| fileinfo | 0 | 6 |
| shmop | 0 | 6 |
| exif | 0 | 4 |
| sysvsem | 0 | 4 |
| SPX | 0 | 3 |
| SimpleXML | 0 | 3 |
| dom | 0 | 2 |
| soap | 0 | 2 |
| tokenizer | 0 | 2 |
| PDO | 0 | 1 |

## Implemented and missing functions, by extension

### standard

Implemented: `abs`, `addcslashes`, `addslashes`, `array_all`, `array_any`, `array_chunk`, `array_column`, `array_combine`, `array_count_values`, `array_diff`, `array_diff_assoc`, `array_diff_key`, `array_diff_ukey`, `array_fill`, `array_fill_keys`, `array_filter`, `array_find`, `array_find_key`, `array_first`, `array_flip`, `array_intersect`, `array_intersect_key`, `array_intersect_ukey`, `array_is_list`, `array_key_exists`, `array_key_first`, `array_key_last`, `array_keys`, `array_last`, `array_map`, `array_merge`, `array_merge_recursive`, `array_pad`, `array_pop`, `array_product`, `array_push`, `array_rand`, `array_reduce`, `array_replace`, `array_replace_recursive`, `array_reverse`, `array_search`, `array_shift`, `array_slice`, `array_splice`, `array_sum`, `array_udiff`, `array_uintersect`, `array_unique`, `array_unshift`, `array_values`, `array_walk`, `array_walk_recursive`, `arsort`, `asort`, `assert`, `atan`, `atan2`, `base64_decode`, `base64_encode`, `base_convert`, `basename`, `bin2hex`, `bindec`, `boolval`, `call_user_func`, `call_user_func_array`, `ceil`, `chdir`, `chgrp`, `chmod`, `chop`, `chown`, `chr`, `chunk_split`, `clearstatcache`, `closedir`, `compact`, `connection_aborted`, `connection_status`, `constant`, `copy`, `cos`, `count`, `crc32`, `crypt`, `current`, `decbin`, `dechex`, `decoct`, `dirname`, `disk_free_space`, `disk_total_space`, `doubleval`, `end`, `error_clear_last`, `error_get_last`, `error_log`, `escapeshellarg`, `escapeshellcmd`, `exp`, `explode`, `extract`, `fclose`, `feof`, `fflush`, `fgetc`, `fgetcsv`, `fgets`, `file`, `file_exists`, `file_get_contents`, `file_put_contents`, `fileatime`, `filectime`, `filegroup`, `fileinode`, `filemtime`, `fileowner`, `fileperms`, `filesize`, `filetype`, `floatval`, `flock`, `floor`, `flush`, `fmod`, `fnmatch`, `fopen`, `fpassthru`, `fprintf`, `fputcsv`, `fputs`, `fread`, `fseek`, `fstat`, `ftell`, `ftruncate`, `fwrite`, `get_cfg_var`, `get_current_user`, `get_debug_type`, `get_html_translation_table`, `get_include_path`, `getcwd`, `getenv`, `gethostbyname`, `gethostbynamel`, `gethostname`, `getmygid`, `getmyinode`, `getmypid`, `getmyuid`, `getrusage`, `gettype`, `glob`, `header`, `header_register_callback`, `header_remove`, `headers_list`, `headers_sent`, `hex2bin`, `hexdec`, `hrtime`, `html_entity_decode`, `htmlentities`, `htmlspecialchars`, `htmlspecialchars_decode`, `http_build_query`, `http_response_code`, `ignore_user_abort`, `implode`, `in_array`, `inet_ntop`, `inet_pton`, `ini_alter`, `ini_get`, `ini_get_all`, `ini_restore`, `ini_set`, `intdiv`, `intval`, `ip2long`, `is_array`, `is_bool`, `is_callable`, `is_countable`, `is_dir`, `is_double`, `is_executable`, `is_file`, `is_finite`, `is_float`, `is_infinite`, `is_int`, `is_integer`, `is_iterable`, `is_link`, `is_long`, `is_nan`, `is_null`, `is_numeric`, `is_object`, `is_readable`, `is_resource`, `is_scalar`, `is_string`, `is_uploaded_file`, `is_writable`, `is_writeable`, `join`, `key`, `key_exists`, `krsort`, `ksort`, `lcfirst`, `lchgrp`, `lchown`, `levenshtein`, `link`, `localeconv`, `log`, `log10`, `long2ip`, `lstat`, `ltrim`, `mail`, `max`, `md5`, `md5_file`, `memory_get_peak_usage`, `memory_get_usage`, `memory_reset_peak_usage`, `microtime`, `min`, `mkdir`, `move_uploaded_file`, `natcasesort`, `natsort`, `next`, `nl2br`, `number_format`, `ob_end_clean`, `ob_end_flush`, `ob_get_clean`, `ob_get_contents`, `ob_get_level`, `ob_start`, `octdec`, `opendir`, `ord`, `pack`, `parse_ini_file`, `parse_ini_string`, `parse_str`, `parse_url`, `password_get_info`, `password_hash`, `password_needs_rehash`, `password_verify`, `pathinfo`, `php_ini_loaded_file`, `php_ini_scanned_files`, `php_sapi_name`, `php_uname`, `phpversion`, `pi`, `pos`, `pow`, `prev`, `print_r`, `printf`, `putenv`, `quoted_printable_encode`, `range`, `rawurldecode`, `rawurlencode`, `readdir`, `readfile`, `readlink`, `realpath`, `realpath_cache_size`, `register_shutdown_function`, `rename`, `reset`, `rewind`, `rewinddir`, `rmdir`, `round`, `rsort`, `rtrim`, `scandir`, `serialize`, `set_include_path`, `set_time_limit`, `setcookie`, `setlocale`, `setrawcookie`, `settype`, `sha1`, `sha1_file`, `shuffle`, `similar_text`, `sin`, `sizeof`, `sleep`, `sort`, `soundex`, `sprintf`, `sqrt`, `sscanf`, `stat`, `str_contains`, `str_ends_with`, `str_getcsv`, `str_ireplace`, `str_pad`, `str_repeat`, `str_replace`, `str_split`, `str_starts_with`, `str_word_count`, `strcspn`, `stream_context_create`, `stream_context_get_default`, `stream_context_get_options`, `stream_context_set_option`, `stream_get_contents`, `stream_get_meta_data`, `stream_get_wrappers`, `stream_is_local`, `stream_isatty`, `stream_resolve_include_path`, `stream_set_blocking`, `stream_set_timeout`, `stream_set_write_buffer`, `strip_tags`, `stripcslashes`, `stripos`, `stripslashes`, `stristr`, `strnatcasecmp`, `strnatcmp`, `strpbrk`, `strpos`, `strrchr`, `strrev`, `strrpos`, `strspn`, `strstr`, `strtok`, `strtolower`, `strtoupper`, `strtr`, `strval`, `substr`, `substr_compare`, `substr_count`, `substr_replace`, `symlink`, `sys_get_temp_dir`, `sys_getloadavg`, `tan`, `tempnam`, `time_nanosleep`, `tmpfile`, `touch`, `trim`, `uasort`, `ucfirst`, `ucwords`, `uksort`, `umask`, `uniqid`, `unlink`, `unpack`, `unserialize`, `urldecode`, `urlencode`, `usleep`, `usort`, `utf8_decode`, `utf8_encode`, `var_dump`, `var_export`, `version_compare`, `vfprintf`, `vprintf`, `vsprintf`, `wordwrap`

Missing: `acos`, `acosh`, `array_change_key_case`, `array_diff_uassoc`, `array_intersect_assoc`, `array_intersect_uassoc`, `array_multisort`, `array_udiff_assoc`, `array_udiff_uassoc`, `array_uintersect_assoc`, `array_uintersect_uassoc`, `asin`, `asinh`, `assert_options`, `atanh`, `checkdnsrr`, `cli_get_process_title`, `cli_set_process_title`, `closelog`, `convert_uudecode`, `convert_uuencode`, `cosh`, `count_chars`, `debug_zval_dump`, `deg2rad`, `dir`, `diskfreespace`, `dl`, `dns_check_record`, `dns_get_mx`, `dns_get_record`, `exec`, `expm1`, `fdatasync`, `fdiv`, `forward_static_call`, `forward_static_call_array`, `fpow`, `fscanf`, `fsockopen`, `fsync`, `ftok`, `get_browser`, `get_headers`, `get_meta_tags`, `gethostbyaddr`, `getimagesize`, `getimagesizefromstring`, `getlastmod`, `getmxrr`, `getopt`, `getprotobyname`, `getprotobynumber`, `getservbyname`, `getservbyport`, `gettimeofday`, `hebrev`, `highlight_file`, `highlight_string`, `http_clear_last_response_headers`, `http_get_last_response_headers`, `hypot`, `image_type_to_extension`, `image_type_to_mime_type`, `ini_parse_quantity`, `iptcembed`, `iptcparse`, `linkinfo`, `log1p`, `metaphone`, `net_get_interfaces`, `nl_langinfo`, `ob_clean`, `ob_flush`, `ob_get_flush`, `ob_get_length`, `ob_get_status`, `ob_implicit_flush`, `ob_list_handlers`, `openlog`, `output_add_rewrite_var`, `output_reset_rewrite_vars`, `passthru`, `password_algos`, `pclose`, `pfsockopen`, `php_strip_whitespace`, `phpcredits`, `phpinfo`, `popen`, `proc_close`, `proc_get_status`, `proc_nice`, `proc_open`, `proc_terminate`, `quoted_printable_decode`, `quotemeta`, `rad2deg`, `realpath_cache_get`, `register_tick_function`, `request_parse_body`, `set_file_buffer`, `shell_exec`, `show_source`, `sinh`, `socket_get_status`, `socket_set_blocking`, `socket_set_timeout`, `str_decrement`, `str_increment`, `str_rot13`, `str_shuffle`, `strchr`, `strcoll`, `stream_bucket_append`, `stream_bucket_make_writeable`, `stream_bucket_new`, `stream_bucket_prepend`, `stream_context_get_params`, `stream_context_set_default`, `stream_context_set_options`, `stream_context_set_params`, `stream_copy_to_stream`, `stream_filter_append`, `stream_filter_prepend`, `stream_filter_register`, `stream_filter_remove`, `stream_get_filters`, `stream_get_line`, `stream_get_transports`, `stream_register_wrapper`, `stream_select`, `stream_set_chunk_size`, `stream_set_read_buffer`, `stream_socket_accept`, `stream_socket_client`, `stream_socket_enable_crypto`, `stream_socket_get_name`, `stream_socket_pair`, `stream_socket_recvfrom`, `stream_socket_sendto`, `stream_socket_server`, `stream_socket_shutdown`, `stream_supports_lock`, `stream_wrapper_register`, `stream_wrapper_restore`, `stream_wrapper_unregister`, `strptime`, `strripos`, `syslog`, `system`, `tanh`, `time_sleep_until`, `unregister_tick_function`

### Core

Implemented: `class_alias`, `class_exists`, `debug_backtrace`, `debug_print_backtrace`, `define`, `defined`, `enum_exists`, `error_reporting`, `exit`, `extension_loaded`, `func_get_arg`, `func_get_args`, `func_num_args`, `function_exists`, `gc_collect_cycles`, `gc_disable`, `gc_enable`, `gc_enabled`, `gc_mem_caches`, `gc_status`, `get_called_class`, `get_class`, `get_class_methods`, `get_class_vars`, `get_defined_functions`, `get_defined_vars`, `get_loaded_extensions`, `get_object_vars`, `get_parent_class`, `get_resource_id`, `get_resource_type`, `interface_exists`, `is_a`, `is_subclass_of`, `method_exists`, `property_exists`, `restore_error_handler`, `restore_exception_handler`, `set_error_handler`, `set_exception_handler`, `strcasecmp`, `strcmp`, `strlen`, `strncasecmp`, `strncmp`, `trait_exists`, `trigger_error`

Missing: `clone`, `die`, `get_declared_classes`, `get_declared_interfaces`, `get_declared_traits`, `get_defined_constants`, `get_error_handler`, `get_exception_handler`, `get_extension_funcs`, `get_included_files`, `get_mangled_object_vars`, `get_required_files`, `get_resources`, `user_error`, `zend_version`

### mbstring

Implemented: `mb_check_encoding`, `mb_chr`, `mb_convert_case`, `mb_convert_encoding`, `mb_detect_encoding`, `mb_internal_encoding`, `mb_lcfirst`, `mb_ltrim`, `mb_ord`, `mb_rtrim`, `mb_scrub`, `mb_str_pad`, `mb_str_split`, `mb_strimwidth`, `mb_stripos`, `mb_strlen`, `mb_strpos`, `mb_strrpos`, `mb_strstr`, `mb_strtolower`, `mb_strtoupper`, `mb_strwidth`, `mb_substr`, `mb_substr_count`, `mb_trim`, `mb_ucfirst`

Missing: `mb_convert_kana`, `mb_convert_variables`, `mb_decode_mimeheader`, `mb_decode_numericentity`, `mb_detect_order`, `mb_encode_mimeheader`, `mb_encode_numericentity`, `mb_encoding_aliases`, `mb_ereg`, `mb_ereg_match`, `mb_ereg_replace`, `mb_ereg_replace_callback`, `mb_ereg_search`, `mb_ereg_search_getpos`, `mb_ereg_search_getregs`, `mb_ereg_search_init`, `mb_ereg_search_pos`, `mb_ereg_search_regs`, `mb_ereg_search_setpos`, `mb_eregi`, `mb_eregi_replace`, `mb_get_info`, `mb_http_input`, `mb_http_output`, `mb_language`, `mb_list_encodings`, `mb_output_handler`, `mb_parse_str`, `mb_preferred_mime_name`, `mb_regex_encoding`, `mb_regex_set_options`, `mb_send_mail`, `mb_split`, `mb_strcut`, `mb_stristr`, `mb_strrchr`, `mb_strrichr`, `mb_strripos`, `mb_substitute_character`

### SPL

Implemented: `class_implements`, `class_parents`, `class_uses`, `iterator_apply`, `iterator_count`, `iterator_to_array`, `spl_autoload_functions`, `spl_autoload_register`, `spl_autoload_unregister`, `spl_object_hash`, `spl_object_id`

Missing: `spl_autoload`, `spl_autoload_call`, `spl_autoload_extensions`, `spl_classes`

### ctype

Implemented: `ctype_alnum`, `ctype_alpha`, `ctype_cntrl`, `ctype_digit`, `ctype_graph`, `ctype_lower`, `ctype_print`, `ctype_punct`, `ctype_space`, `ctype_upper`, `ctype_xdigit`

Missing: none

### hash

Implemented: `hash`, `hash_algos`, `hash_copy`, `hash_equals`, `hash_file`, `hash_final`, `hash_hmac`, `hash_hmac_algos`, `hash_init`, `hash_update`

Missing: `hash_hkdf`, `hash_hmac_file`, `hash_pbkdf2`, `hash_update_file`, `hash_update_stream`, `mhash`, `mhash_count`, `mhash_get_block_size`, `mhash_get_hash_name`, `mhash_keygen_s2k`

### pcre

Implemented: `preg_grep`, `preg_last_error`, `preg_last_error_msg`, `preg_match`, `preg_match_all`, `preg_quote`, `preg_replace`, `preg_replace_callback`, `preg_split`

Missing: `preg_filter`, `preg_replace_callback_array`

### random

Implemented: `getrandmax`, `lcg_value`, `mt_getrandmax`, `mt_rand`, `mt_srand`, `rand`, `random_bytes`, `random_int`, `srand`

Missing: none

### zlib

Implemented: `gzcompress`, `gzdecode`, `gzdeflate`, `gzencode`, `gzinflate`, `gzuncompress`, `zlib_decode`, `zlib_encode`

Missing: `deflate_add`, `deflate_init`, `gzclose`, `gzeof`, `gzfile`, `gzgetc`, `gzgets`, `gzopen`, `gzpassthru`, `gzputs`, `gzread`, `gzrewind`, `gzseek`, `gztell`, `gzwrite`, `inflate_add`, `inflate_get_read_len`, `inflate_get_status`, `inflate_init`, `ob_gzhandler`, `readgzfile`, `zlib_get_coding_type`

### date

Implemented: `checkdate`, `date`, `date_default_timezone_get`, `date_default_timezone_set`, `mktime`, `time`

Missing: `date_add`, `date_create`, `date_create_from_format`, `date_create_immutable`, `date_create_immutable_from_format`, `date_date_set`, `date_diff`, `date_format`, `date_get_last_errors`, `date_interval_create_from_date_string`, `date_interval_format`, `date_isodate_set`, `date_modify`, `date_offset_get`, `date_parse`, `date_parse_from_format`, `date_sub`, `date_sun_info`, `date_sunrise`, `date_sunset`, `date_time_set`, `date_timestamp_get`, `date_timestamp_set`, `date_timezone_get`, `date_timezone_set`, `getdate`, `gmdate`, `gmmktime`, `gmstrftime`, `idate`, `localtime`, `strftime`, `strtotime`, `timezone_abbreviations_list`, `timezone_identifiers_list`, `timezone_location_get`, `timezone_name_from_abbr`, `timezone_name_get`, `timezone_offset_get`, `timezone_open`, `timezone_transitions_get`, `timezone_version_get`

### filter

Implemented: `filter_has_var`, `filter_id`, `filter_input`, `filter_list`, `filter_var`

Missing: `filter_input_array`, `filter_var_array`

### json

Implemented: `json_decode`, `json_encode`, `json_last_error`, `json_last_error_msg`, `json_validate`

Missing: none

### iconv

Implemented: `iconv`, `iconv_strlen`

Missing: `iconv_get_encoding`, `iconv_mime_decode`, `iconv_mime_decode_headers`, `iconv_mime_encode`, `iconv_set_encoding`, `iconv_strpos`, `iconv_strrpos`, `iconv_substr`

### openssl

Implemented: `openssl_random_pseudo_bytes`

Missing: `openssl_cipher_iv_length`, `openssl_cipher_key_length`, `openssl_cms_decrypt`, `openssl_cms_encrypt`, `openssl_cms_read`, `openssl_cms_sign`, `openssl_cms_verify`, `openssl_csr_export`, `openssl_csr_export_to_file`, `openssl_csr_get_public_key`, `openssl_csr_get_subject`, `openssl_csr_new`, `openssl_csr_sign`, `openssl_decrypt`, `openssl_dh_compute_key`, `openssl_digest`, `openssl_encrypt`, `openssl_error_string`, `openssl_free_key`, `openssl_get_cert_locations`, `openssl_get_cipher_methods`, `openssl_get_curve_names`, `openssl_get_md_methods`, `openssl_get_privatekey`, `openssl_get_publickey`, `openssl_open`, `openssl_pbkdf2`, `openssl_pkcs12_export`, `openssl_pkcs12_export_to_file`, `openssl_pkcs12_read`, `openssl_pkcs7_decrypt`, `openssl_pkcs7_encrypt`, `openssl_pkcs7_read`, `openssl_pkcs7_sign`, `openssl_pkcs7_verify`, `openssl_pkey_derive`, `openssl_pkey_export`, `openssl_pkey_export_to_file`, `openssl_pkey_free`, `openssl_pkey_get_details`, `openssl_pkey_get_private`, `openssl_pkey_get_public`, `openssl_pkey_new`, `openssl_private_decrypt`, `openssl_private_encrypt`, `openssl_public_decrypt`, `openssl_public_encrypt`, `openssl_seal`, `openssl_sign`, `openssl_spki_export`, `openssl_spki_export_challenge`, `openssl_spki_new`, `openssl_spki_verify`, `openssl_verify`, `openssl_x509_check_private_key`, `openssl_x509_checkpurpose`, `openssl_x509_export`, `openssl_x509_export_to_file`, `openssl_x509_fingerprint`, `openssl_x509_free`, `openssl_x509_parse`, `openssl_x509_read`, `openssl_x509_verify`

### posix

Implemented: `posix_isatty`

Missing: `posix_access`, `posix_ctermid`, `posix_eaccess`, `posix_errno`, `posix_get_last_error`, `posix_getcwd`, `posix_getegid`, `posix_geteuid`, `posix_getgid`, `posix_getgrgid`, `posix_getgrnam`, `posix_getgroups`, `posix_getlogin`, `posix_getpgid`, `posix_getpgrp`, `posix_getpid`, `posix_getppid`, `posix_getpwnam`, `posix_getpwuid`, `posix_getrlimit`, `posix_getsid`, `posix_getuid`, `posix_initgroups`, `posix_kill`, `posix_mkfifo`, `posix_mknod`, `posix_setegid`, `posix_seteuid`, `posix_setgid`, `posix_setpgid`, `posix_setrlimit`, `posix_setsid`, `posix_setuid`, `posix_strerror`, `posix_sysconf`, `posix_times`, `posix_ttyname`, `posix_uname`

Every other extension in the table above (intl, sodium, mysqli, gd, ldap, gmp, xmlwriter, sockets, ftp, curl, pcntl, session, xml, calendar, dba, bcmath, readline, bz2, zip, libxml, sysvmsg, sysvshm, fileinfo, shmop, exif, sysvsem, SPX, SimpleXML, dom, soap, tokenizer, PDO) has
**no** functions implemented yet.

## Notable gaps

Functions that real applications hit early, grouped by what needs them
([roadmap.md](roadmap.md)).

| Area | Missing functions | Needed by |
|---|---|---|
| String scanning | `strripos` | Various |
| Date/time | `strtotime`, `gmdate`, `getdate`, `localtime`, `idate`, `date_*`, `timezone_*`, classes `DateTime`, `DateTimeImmutable`, `DateTimeZone`, `DateInterval`, `DatePeriod` | Carbon (Laravel), WordPress `current_time`, Drupal |
| Arrays | `array_multisort`, `array_change_key_case`, `array_intersect_assoc` | WordPress (`wp_parse_args`), Laravel `Arr` |
| Output buffering | `ob_get_length`, `ob_get_flush`, `ob_flush`, `ob_clean`, `ob_get_status`, `ob_implicit_flush`, `ob_list_handlers` | Templates, WordPress |
| Class introspection | `get_declared_classes`, `get_included_files`, `get_defined_constants` | Frameworks, plugin loaders |
| Processes / system | `exec`, `shell_exec`, `system`, `passthru`, `proc_open`, `popen` | WP-CLI, Laravel `Process`, cron |
| Regex | `preg_replace_callback_array`, `preg_filter` | Laravel, Symfony |
| Math | `acos`, `asin`, `sinh`, `cosh`, `tanh`, `hypot`, `deg2rad`, `rad2deg`, `fdiv` | Various |
| Streams / network | `fsockopen`, `stream_socket_client`, `stream_socket_server`, `stream_select`, `stream_copy_to_stream`, `stream_get_line`, `stream_wrapper_register` | HTTP clients, CSV import/export |
| Extensions (whole) | mysqli, PDO, curl, gd/imagick, xml/xmlreader/xmlwriter/dom/SimpleXML, sodium, openssl, intl, zip, zlib, session, filter, fileinfo, bcmath, gmp, hash | Roadmap items 2, 3 and 6 |

## Built-in classes and interfaces

| Group | Implemented | Missing (notable) |
|---|---|---|
| Core interfaces | `Traversable`, `Iterator`, `IteratorAggregate`, `ArrayAccess`, `Countable`, `Stringable`, `JsonSerializable`, `UnitEnum`, `BackedEnum`, `Throwable`, `Serializable`, `OuterIterator`, `RecursiveIterator`, `SeekableIterator`, `SplObserver`, `SplSubject` | — |
| Core classes | `stdClass`, `Closure` (`bind`, `bindTo`, `call`, `fromCallable`, `__invoke`), `Generator` (`current`, `key`, `next`, `send`, `throw`, `valid`, `rewind`, `getReturn`), `__PHP_Incomplete_Class` | `WeakMap`, `WeakReference`, `Fiber` |
| Errors | `Error`, `TypeError`, `ValueError`, `ArithmeticError`, `DivisionByZeroError`, `CompileError`, `ParseError`, `ArgumentCountError`, `UnhandledMatchError` | — |
| Exceptions | `Exception`, `ErrorException`, `JsonException`, SPL: `LogicException`, `BadFunctionCallException`, `BadMethodCallException`, `DomainException`, `InvalidArgumentException`, `LengthException`, `OutOfRangeException`, `RuntimeException`, `OutOfBoundsException`, `OverflowException`, `RangeException`, `UnderflowException`, `UnexpectedValueException` | `getTrace()` returns `[]` (no real backtraces) |
| SPL data structures | `ArrayObject`, `ArrayIterator`, `RecursiveArrayIterator`, `SplDoublyLinkedList`, `SplStack`, `SplQueue`, `SplHeap`, `SplMinHeap`, `SplMaxHeap`, `SplPriorityQueue`, `SplFixedArray`, `SplObjectStorage` (written in PHP, `prelude_spl.php`) | `SplTempFileObject` |
| SPL iterators | `IteratorIterator`, `FilterIterator`, `CallbackFilterIterator`, `RecursiveFilterIterator`, `RecursiveCallbackFilterIterator`, `ParentIterator`, `LimitIterator`, `InfiniteIterator`, `NoRewindIterator`, `AppendIterator`, `CachingIterator`, `RecursiveCachingIterator`, `RecursiveIteratorIterator`, `EmptyIterator` | `RegexIterator`, `MultipleIterator`, `RecursiveTreeIterator`, `GlobIterator`, `DirectoryIterator`, `FilesystemIterator`, `RecursiveDirectoryIterator`, `SplFileInfo`, `SplFileObject` |
| Reflection | — | All `Reflection*` classes |
| Date | — | `DateTime`, `DateTimeImmutable`, `DateTimeZone`, `DateInterval`, `DatePeriod`, `DateTimeInterface` |
| phpx async | `Promise`, `Concurrency`, `TimeoutException`, `CancelledException`, `AggregateException` | — |

Enum methods `cases()`, `from()`, `tryFrom()` are provided natively for
every enum.

Built-ins are checked against PHP's own signatures (generated from PHP's
Reflection into `arginfo_table.rs` by `tools/arginfo/`): wrong argument
counts throw `ArgumentCountError`, wrong types throw `TypeError` with
PHP's coercive-mode juggling and `strict_types` handling, and named
arguments work.

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
