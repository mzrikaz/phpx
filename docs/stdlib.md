# Standard library coverage

phpx implements PHP's built-in functions natively in Rust
(`crates/interp/src/{builtins,strings,arrays,json,preg,fs,streams}.rs`) and
built-in classes either in PHP (`crates/interp/src/prelude.php`) or as
native methods (`crates/interp/src/native.rs`, `generator.rs`).

> **Snapshot: 2026-10-26.** Regenerate this page's numbers with the
> procedure in [Regenerating the coverage data](#regenerating-the-coverage-data)
> and `tools/stdlib_coverage.py`.

## Summary

- **973** PHP functions implemented, out of the 1,910 functions exposed by
  the reference PHP 8.5 build (which includes many optional extensions).
  Counting only `Core` + `standard`, phpx has 606 of 606.
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
| standard | 544 | 544 |
| mbstring | 65 | 65 |
| Core | 62 | 62 |
| date | 48 | 48 |
| xmlwriter | 33 | 42 |
| zlib | 30 | 30 |
| intl | 29 | 187 |
| session | 23 | 23 |
| xml | 22 | 22 |
| hash | 20 | 20 |
| SPL | 15 | 15 |
| ctype | 11 | 11 |
| pcre | 11 | 11 |
| openssl | 10 | 64 |
| iconv | 10 | 10 |
| random | 9 | 9 |
| libxml | 8 | 8 |
| filter | 7 | 7 |
| fileinfo | 6 | 6 |
| json | 5 | 5 |
| tokenizer | 2 | 2 |
| posix | 1 | 39 |
| dom | 1 | 2 |
| PDO | 1 | 1 |
| sodium | 0 | 110 |
| mysqli | 0 | 106 |
| gd | 0 | 103 |
| ldap | 0 | 55 |
| gmp | 0 | 51 |
| sockets | 0 | 37 |
| ftp | 0 | 36 |
| curl | 0 | 35 |
| pcntl | 0 | 28 |
| calendar | 0 | 18 |
| dba | 0 | 15 |
| bcmath | 0 | 14 |
| readline | 0 | 13 |
| bz2 | 0 | 10 |
| zip | 0 | 10 |
| sysvmsg | 0 | 7 |
| sysvshm | 0 | 7 |
| shmop | 0 | 6 |
| exif | 0 | 4 |
| sysvsem | 0 | 4 |
| SPX | 0 | 3 |
| SimpleXML | 0 | 3 |
| soap | 0 | 2 |

## Implemented and missing functions, by extension

### standard

Implemented: `abs`, `acos`, `acosh`, `addcslashes`, `addslashes`, `array_all`, `array_any`, `array_change_key_case`, `array_chunk`, `array_column`, `array_combine`, `array_count_values`, `array_diff`, `array_diff_assoc`, `array_diff_key`, `array_diff_uassoc`, `array_diff_ukey`, `array_fill`, `array_fill_keys`, `array_filter`, `array_find`, `array_find_key`, `array_first`, `array_flip`, `array_intersect`, `array_intersect_assoc`, `array_intersect_key`, `array_intersect_uassoc`, `array_intersect_ukey`, `array_is_list`, `array_key_exists`, `array_key_first`, `array_key_last`, `array_keys`, `array_last`, `array_map`, `array_merge`, `array_merge_recursive`, `array_multisort`, `array_pad`, `array_pop`, `array_product`, `array_push`, `array_rand`, `array_reduce`, `array_replace`, `array_replace_recursive`, `array_reverse`, `array_search`, `array_shift`, `array_slice`, `array_splice`, `array_sum`, `array_udiff`, `array_udiff_assoc`, `array_udiff_uassoc`, `array_uintersect`, `array_uintersect_assoc`, `array_uintersect_uassoc`, `array_unique`, `array_unshift`, `array_values`, `array_walk`, `array_walk_recursive`, `arsort`, `asin`, `asinh`, `asort`, `assert`, `assert_options`, `atan`, `atan2`, `atanh`, `base64_decode`, `base64_encode`, `base_convert`, `basename`, `bin2hex`, `bindec`, `boolval`, `call_user_func`, `call_user_func_array`, `ceil`, `chdir`, `checkdnsrr`, `chgrp`, `chmod`, `chop`, `chown`, `chr`, `chunk_split`, `clearstatcache`, `cli_get_process_title`, `cli_set_process_title`, `closedir`, `closelog`, `compact`, `connection_aborted`, `connection_status`, `constant`, `convert_uudecode`, `convert_uuencode`, `copy`, `cos`, `cosh`, `count`, `count_chars`, `crc32`, `crypt`, `current`, `debug_zval_dump`, `decbin`, `dechex`, `decoct`, `deg2rad`, `dir`, `dirname`, `disk_free_space`, `disk_total_space`, `diskfreespace`, `dl`, `dns_check_record`, `dns_get_mx`, `dns_get_record`, `doubleval`, `end`, `error_clear_last`, `error_get_last`, `error_log`, `escapeshellarg`, `escapeshellcmd`, `exec`, `exp`, `explode`, `expm1`, `extract`, `fclose`, `fdatasync`, `fdiv`, `feof`, `fflush`, `fgetc`, `fgetcsv`, `fgets`, `file`, `file_exists`, `file_get_contents`, `file_put_contents`, `fileatime`, `filectime`, `filegroup`, `fileinode`, `filemtime`, `fileowner`, `fileperms`, `filesize`, `filetype`, `floatval`, `flock`, `floor`, `flush`, `fmod`, `fnmatch`, `fopen`, `forward_static_call`, `forward_static_call_array`, `fpassthru`, `fpow`, `fprintf`, `fputcsv`, `fputs`, `fread`, `fscanf`, `fseek`, `fsockopen`, `fstat`, `fsync`, `ftell`, `ftok`, `ftruncate`, `fwrite`, `get_browser`, `get_cfg_var`, `get_current_user`, `get_debug_type`, `get_headers`, `get_html_translation_table`, `get_include_path`, `get_meta_tags`, `getcwd`, `getenv`, `gethostbyaddr`, `gethostbyname`, `gethostbynamel`, `gethostname`, `getimagesize`, `getimagesizefromstring`, `getlastmod`, `getmxrr`, `getmygid`, `getmyinode`, `getmypid`, `getmyuid`, `getopt`, `getprotobyname`, `getprotobynumber`, `getrusage`, `getservbyname`, `getservbyport`, `gettimeofday`, `gettype`, `glob`, `header`, `header_register_callback`, `header_remove`, `headers_list`, `headers_sent`, `hebrev`, `hex2bin`, `hexdec`, `highlight_file`, `highlight_string`, `hrtime`, `html_entity_decode`, `htmlentities`, `htmlspecialchars`, `htmlspecialchars_decode`, `http_build_query`, `http_clear_last_response_headers`, `http_get_last_response_headers`, `http_response_code`, `hypot`, `ignore_user_abort`, `image_type_to_extension`, `image_type_to_mime_type`, `implode`, `in_array`, `inet_ntop`, `inet_pton`, `ini_alter`, `ini_get`, `ini_get_all`, `ini_parse_quantity`, `ini_restore`, `ini_set`, `intdiv`, `intval`, `ip2long`, `iptcembed`, `iptcparse`, `is_array`, `is_bool`, `is_callable`, `is_countable`, `is_dir`, `is_double`, `is_executable`, `is_file`, `is_finite`, `is_float`, `is_infinite`, `is_int`, `is_integer`, `is_iterable`, `is_link`, `is_long`, `is_nan`, `is_null`, `is_numeric`, `is_object`, `is_readable`, `is_resource`, `is_scalar`, `is_string`, `is_uploaded_file`, `is_writable`, `is_writeable`, `join`, `key`, `key_exists`, `krsort`, `ksort`, `lcfirst`, `lchgrp`, `lchown`, `levenshtein`, `link`, `linkinfo`, `localeconv`, `log`, `log10`, `log1p`, `long2ip`, `lstat`, `ltrim`, `mail`, `max`, `md5`, `md5_file`, `memory_get_peak_usage`, `memory_get_usage`, `memory_reset_peak_usage`, `metaphone`, `microtime`, `min`, `mkdir`, `move_uploaded_file`, `natcasesort`, `natsort`, `net_get_interfaces`, `next`, `nl2br`, `nl_langinfo`, `number_format`, `ob_clean`, `ob_end_clean`, `ob_end_flush`, `ob_flush`, `ob_get_clean`, `ob_get_contents`, `ob_get_flush`, `ob_get_length`, `ob_get_level`, `ob_get_status`, `ob_implicit_flush`, `ob_list_handlers`, `ob_start`, `octdec`, `opendir`, `openlog`, `ord`, `output_add_rewrite_var`, `output_reset_rewrite_vars`, `pack`, `parse_ini_file`, `parse_ini_string`, `parse_str`, `parse_url`, `passthru`, `password_algos`, `password_get_info`, `password_hash`, `password_needs_rehash`, `password_verify`, `pathinfo`, `pclose`, `pfsockopen`, `php_ini_loaded_file`, `php_ini_scanned_files`, `php_sapi_name`, `php_strip_whitespace`, `php_uname`, `phpcredits`, `phpinfo`, `phpversion`, `pi`, `popen`, `pos`, `pow`, `prev`, `print_r`, `printf`, `proc_close`, `proc_get_status`, `proc_nice`, `proc_open`, `proc_terminate`, `putenv`, `quoted_printable_decode`, `quoted_printable_encode`, `quotemeta`, `rad2deg`, `range`, `rawurldecode`, `rawurlencode`, `readdir`, `readfile`, `readlink`, `realpath`, `realpath_cache_get`, `realpath_cache_size`, `register_shutdown_function`, `register_tick_function`, `rename`, `request_parse_body`, `reset`, `rewind`, `rewinddir`, `rmdir`, `round`, `rsort`, `rtrim`, `scandir`, `serialize`, `set_file_buffer`, `set_include_path`, `set_time_limit`, `setcookie`, `setlocale`, `setrawcookie`, `settype`, `sha1`, `sha1_file`, `shell_exec`, `show_source`, `shuffle`, `similar_text`, `sin`, `sinh`, `sizeof`, `sleep`, `socket_get_status`, `socket_set_blocking`, `socket_set_timeout`, `sort`, `soundex`, `sprintf`, `sqrt`, `sscanf`, `stat`, `str_contains`, `str_decrement`, `str_ends_with`, `str_getcsv`, `str_increment`, `str_ireplace`, `str_pad`, `str_repeat`, `str_replace`, `str_rot13`, `str_shuffle`, `str_split`, `str_starts_with`, `str_word_count`, `strchr`, `strcoll`, `strcspn`, `stream_bucket_append`, `stream_bucket_make_writeable`, `stream_bucket_new`, `stream_bucket_prepend`, `stream_context_create`, `stream_context_get_default`, `stream_context_get_options`, `stream_context_get_params`, `stream_context_set_default`, `stream_context_set_option`, `stream_context_set_options`, `stream_context_set_params`, `stream_copy_to_stream`, `stream_filter_append`, `stream_filter_prepend`, `stream_filter_register`, `stream_filter_remove`, `stream_get_contents`, `stream_get_filters`, `stream_get_line`, `stream_get_meta_data`, `stream_get_transports`, `stream_get_wrappers`, `stream_is_local`, `stream_isatty`, `stream_register_wrapper`, `stream_resolve_include_path`, `stream_select`, `stream_set_blocking`, `stream_set_chunk_size`, `stream_set_read_buffer`, `stream_set_timeout`, `stream_set_write_buffer`, `stream_socket_accept`, `stream_socket_client`, `stream_socket_enable_crypto`, `stream_socket_get_name`, `stream_socket_pair`, `stream_socket_recvfrom`, `stream_socket_sendto`, `stream_socket_server`, `stream_socket_shutdown`, `stream_supports_lock`, `stream_wrapper_register`, `stream_wrapper_restore`, `stream_wrapper_unregister`, `strip_tags`, `stripcslashes`, `stripos`, `stripslashes`, `stristr`, `strnatcasecmp`, `strnatcmp`, `strpbrk`, `strpos`, `strptime`, `strrchr`, `strrev`, `strripos`, `strrpos`, `strspn`, `strstr`, `strtok`, `strtolower`, `strtoupper`, `strtr`, `strval`, `substr`, `substr_compare`, `substr_count`, `substr_replace`, `symlink`, `sys_get_temp_dir`, `sys_getloadavg`, `syslog`, `system`, `tan`, `tanh`, `tempnam`, `time_nanosleep`, `time_sleep_until`, `tmpfile`, `touch`, `trim`, `uasort`, `ucfirst`, `ucwords`, `uksort`, `umask`, `uniqid`, `unlink`, `unpack`, `unregister_tick_function`, `unserialize`, `urldecode`, `urlencode`, `usleep`, `usort`, `utf8_decode`, `utf8_encode`, `var_dump`, `var_export`, `version_compare`, `vfprintf`, `vprintf`, `vsprintf`, `wordwrap`

Missing: none

### mbstring

Implemented: `mb_check_encoding`, `mb_chr`, `mb_convert_case`, `mb_convert_encoding`, `mb_convert_kana`, `mb_convert_variables`, `mb_decode_mimeheader`, `mb_decode_numericentity`, `mb_detect_encoding`, `mb_detect_order`, `mb_encode_mimeheader`, `mb_encode_numericentity`, `mb_encoding_aliases`, `mb_ereg`, `mb_ereg_match`, `mb_ereg_replace`, `mb_ereg_replace_callback`, `mb_ereg_search`, `mb_ereg_search_getpos`, `mb_ereg_search_getregs`, `mb_ereg_search_init`, `mb_ereg_search_pos`, `mb_ereg_search_regs`, `mb_ereg_search_setpos`, `mb_eregi`, `mb_eregi_replace`, `mb_get_info`, `mb_http_input`, `mb_http_output`, `mb_internal_encoding`, `mb_language`, `mb_lcfirst`, `mb_list_encodings`, `mb_ltrim`, `mb_ord`, `mb_output_handler`, `mb_parse_str`, `mb_preferred_mime_name`, `mb_regex_encoding`, `mb_regex_set_options`, `mb_rtrim`, `mb_scrub`, `mb_send_mail`, `mb_split`, `mb_str_pad`, `mb_str_split`, `mb_strcut`, `mb_strimwidth`, `mb_stripos`, `mb_stristr`, `mb_strlen`, `mb_strpos`, `mb_strrchr`, `mb_strrichr`, `mb_strripos`, `mb_strrpos`, `mb_strstr`, `mb_strtolower`, `mb_strtoupper`, `mb_strwidth`, `mb_substitute_character`, `mb_substr`, `mb_substr_count`, `mb_trim`, `mb_ucfirst`

Missing: none

### Core

Implemented: `class_alias`, `class_exists`, `clone`, `debug_backtrace`, `debug_print_backtrace`, `define`, `defined`, `die`, `enum_exists`, `error_reporting`, `exit`, `extension_loaded`, `func_get_arg`, `func_get_args`, `func_num_args`, `function_exists`, `gc_collect_cycles`, `gc_disable`, `gc_enable`, `gc_enabled`, `gc_mem_caches`, `gc_status`, `get_called_class`, `get_class`, `get_class_methods`, `get_class_vars`, `get_declared_classes`, `get_declared_interfaces`, `get_declared_traits`, `get_defined_constants`, `get_defined_functions`, `get_defined_vars`, `get_error_handler`, `get_exception_handler`, `get_extension_funcs`, `get_included_files`, `get_loaded_extensions`, `get_mangled_object_vars`, `get_object_vars`, `get_parent_class`, `get_required_files`, `get_resource_id`, `get_resource_type`, `get_resources`, `interface_exists`, `is_a`, `is_subclass_of`, `method_exists`, `property_exists`, `restore_error_handler`, `restore_exception_handler`, `set_error_handler`, `set_exception_handler`, `strcasecmp`, `strcmp`, `strlen`, `strncasecmp`, `strncmp`, `trait_exists`, `trigger_error`, `user_error`, `zend_version`

Missing: none

### date

Implemented: `checkdate`, `date`, `date_add`, `date_create`, `date_create_from_format`, `date_create_immutable`, `date_create_immutable_from_format`, `date_date_set`, `date_default_timezone_get`, `date_default_timezone_set`, `date_diff`, `date_format`, `date_get_last_errors`, `date_interval_create_from_date_string`, `date_interval_format`, `date_isodate_set`, `date_modify`, `date_offset_get`, `date_parse`, `date_parse_from_format`, `date_sub`, `date_sun_info`, `date_sunrise`, `date_sunset`, `date_time_set`, `date_timestamp_get`, `date_timestamp_set`, `date_timezone_get`, `date_timezone_set`, `getdate`, `gmdate`, `gmmktime`, `gmstrftime`, `idate`, `localtime`, `mktime`, `strftime`, `strtotime`, `time`, `timezone_abbreviations_list`, `timezone_identifiers_list`, `timezone_location_get`, `timezone_name_from_abbr`, `timezone_name_get`, `timezone_offset_get`, `timezone_open`, `timezone_transitions_get`, `timezone_version_get`

Missing: none

### xmlwriter

Implemented: `xmlwriter_end_attribute`, `xmlwriter_end_cdata`, `xmlwriter_end_comment`, `xmlwriter_end_document`, `xmlwriter_end_dtd`, `xmlwriter_end_element`, `xmlwriter_end_pi`, `xmlwriter_flush`, `xmlwriter_full_end_element`, `xmlwriter_open_memory`, `xmlwriter_open_uri`, `xmlwriter_output_memory`, `xmlwriter_set_indent`, `xmlwriter_set_indent_string`, `xmlwriter_start_attribute`, `xmlwriter_start_attribute_ns`, `xmlwriter_start_cdata`, `xmlwriter_start_comment`, `xmlwriter_start_document`, `xmlwriter_start_dtd`, `xmlwriter_start_element`, `xmlwriter_start_element_ns`, `xmlwriter_start_pi`, `xmlwriter_text`, `xmlwriter_write_attribute`, `xmlwriter_write_attribute_ns`, `xmlwriter_write_cdata`, `xmlwriter_write_comment`, `xmlwriter_write_dtd`, `xmlwriter_write_element`, `xmlwriter_write_element_ns`, `xmlwriter_write_pi`, `xmlwriter_write_raw`

Missing: `xmlwriter_end_dtd_attlist`, `xmlwriter_end_dtd_element`, `xmlwriter_end_dtd_entity`, `xmlwriter_start_dtd_attlist`, `xmlwriter_start_dtd_element`, `xmlwriter_start_dtd_entity`, `xmlwriter_write_dtd_attlist`, `xmlwriter_write_dtd_element`, `xmlwriter_write_dtd_entity`

### zlib

Implemented: `deflate_add`, `deflate_init`, `gzclose`, `gzcompress`, `gzdecode`, `gzdeflate`, `gzencode`, `gzeof`, `gzfile`, `gzgetc`, `gzgets`, `gzinflate`, `gzopen`, `gzpassthru`, `gzputs`, `gzread`, `gzrewind`, `gzseek`, `gztell`, `gzuncompress`, `gzwrite`, `inflate_add`, `inflate_get_read_len`, `inflate_get_status`, `inflate_init`, `ob_gzhandler`, `readgzfile`, `zlib_decode`, `zlib_encode`, `zlib_get_coding_type`

Missing: none

### intl

Implemented: `intl_error_name`, `intl_get_error_code`, `intl_get_error_message`, `intl_is_failure`, `locale_accept_from_http`, `locale_canonicalize`, `locale_compose`, `locale_get_default`, `locale_get_keywords`, `locale_get_primary_language`, `locale_get_region`, `locale_get_script`, `locale_is_right_to_left`, `locale_parse`, `locale_set_default`, `numfmt_create`, `numfmt_format`, `numfmt_format_currency`, `numfmt_get_attribute`, `numfmt_get_error_code`, `numfmt_get_error_message`, `numfmt_get_locale`, `numfmt_get_pattern`, `numfmt_get_symbol`, `numfmt_get_text_attribute`, `numfmt_set_attribute`, `numfmt_set_pattern`, `numfmt_set_symbol`, `numfmt_set_text_attribute`

Missing: `collator_asort`, `collator_compare`, `collator_create`, `collator_get_attribute`, `collator_get_error_code`, `collator_get_error_message`, `collator_get_locale`, `collator_get_sort_key`, `collator_get_strength`, `collator_set_attribute`, `collator_set_strength`, `collator_sort`, `collator_sort_with_sort_keys`, `datefmt_create`, `datefmt_format`, `datefmt_format_object`, `datefmt_get_calendar`, `datefmt_get_calendar_object`, `datefmt_get_datetype`, `datefmt_get_error_code`, `datefmt_get_error_message`, `datefmt_get_locale`, `datefmt_get_pattern`, `datefmt_get_timetype`, `datefmt_get_timezone`, `datefmt_get_timezone_id`, `datefmt_is_lenient`, `datefmt_localtime`, `datefmt_parse`, `datefmt_set_calendar`, `datefmt_set_lenient`, `datefmt_set_pattern`, `datefmt_set_timezone`, `grapheme_extract`, `grapheme_levenshtein`, `grapheme_str_split`, `grapheme_stripos`, `grapheme_stristr`, `grapheme_strlen`, `grapheme_strpos`, `grapheme_strripos`, `grapheme_strrpos`, `grapheme_strstr`, `grapheme_substr`, `idn_to_ascii`, `idn_to_utf8`, `intlcal_add`, `intlcal_after`, `intlcal_before`, `intlcal_clear`, `intlcal_create_instance`, `intlcal_equals`, `intlcal_field_difference`, `intlcal_from_date_time`, `intlcal_get`, `intlcal_get_actual_maximum`, `intlcal_get_actual_minimum`, `intlcal_get_available_locales`, `intlcal_get_day_of_week_type`, `intlcal_get_error_code`, `intlcal_get_error_message`, `intlcal_get_first_day_of_week`, `intlcal_get_greatest_minimum`, `intlcal_get_keyword_values_for_locale`, `intlcal_get_least_maximum`, `intlcal_get_locale`, `intlcal_get_maximum`, `intlcal_get_minimal_days_in_first_week`, `intlcal_get_minimum`, `intlcal_get_now`, `intlcal_get_repeated_wall_time_option`, `intlcal_get_skipped_wall_time_option`, `intlcal_get_time`, `intlcal_get_time_zone`, `intlcal_get_type`, `intlcal_get_weekend_transition`, `intlcal_in_daylight_time`, `intlcal_is_equivalent_to`, `intlcal_is_lenient`, `intlcal_is_set`, `intlcal_is_weekend`, `intlcal_roll`, `intlcal_set`, `intlcal_set_first_day_of_week`, `intlcal_set_lenient`, `intlcal_set_minimal_days_in_first_week`, `intlcal_set_repeated_wall_time_option`, `intlcal_set_skipped_wall_time_option`, `intlcal_set_time`, `intlcal_set_time_zone`, `intlcal_to_date_time`, `intlgregcal_create_instance`, `intlgregcal_get_gregorian_change`, `intlgregcal_is_leap_year`, `intlgregcal_set_gregorian_change`, `intltz_count_equivalent_ids`, `intltz_create_default`, `intltz_create_enumeration`, `intltz_create_time_zone`, `intltz_create_time_zone_id_enumeration`, `intltz_from_date_time_zone`, `intltz_get_canonical_id`, `intltz_get_display_name`, `intltz_get_dst_savings`, `intltz_get_equivalent_id`, `intltz_get_error_code`, `intltz_get_error_message`, `intltz_get_gmt`, `intltz_get_iana_id`, `intltz_get_id`, `intltz_get_id_for_windows_id`, `intltz_get_offset`, `intltz_get_raw_offset`, `intltz_get_region`, `intltz_get_tz_data_version`, `intltz_get_unknown`, `intltz_get_windows_id`, `intltz_has_same_rules`, `intltz_to_date_time_zone`, `intltz_use_daylight_time`, `locale_add_likely_subtags`, `locale_filter_matches`, `locale_get_all_variants`, `locale_get_display_language`, `locale_get_display_name`, `locale_get_display_region`, `locale_get_display_script`, `locale_get_display_variant`, `locale_lookup`, `locale_minimize_subtags`, `msgfmt_create`, `msgfmt_format`, `msgfmt_format_message`, `msgfmt_get_error_code`, `msgfmt_get_error_message`, `msgfmt_get_locale`, `msgfmt_get_pattern`, `msgfmt_parse`, `msgfmt_parse_message`, `msgfmt_set_pattern`, `normalizer_get_raw_decomposition`, `normalizer_is_normalized`, `normalizer_normalize`, `numfmt_parse`, `numfmt_parse_currency`, `resourcebundle_count`, `resourcebundle_create`, `resourcebundle_get`, `resourcebundle_get_error_code`, `resourcebundle_get_error_message`, `resourcebundle_locales`, `transliterator_create`, `transliterator_create_from_rules`, `transliterator_create_inverse`, `transliterator_get_error_code`, `transliterator_get_error_message`, `transliterator_list_ids`, `transliterator_transliterate`

### session

Implemented: `session_abort`, `session_cache_expire`, `session_cache_limiter`, `session_commit`, `session_create_id`, `session_decode`, `session_destroy`, `session_encode`, `session_gc`, `session_get_cookie_params`, `session_id`, `session_module_name`, `session_name`, `session_regenerate_id`, `session_register_shutdown`, `session_reset`, `session_save_path`, `session_set_cookie_params`, `session_set_save_handler`, `session_start`, `session_status`, `session_unset`, `session_write_close`

Missing: none

### xml

Implemented: `xml_error_string`, `xml_get_current_byte_index`, `xml_get_current_column_number`, `xml_get_current_line_number`, `xml_get_error_code`, `xml_parse`, `xml_parse_into_struct`, `xml_parser_create`, `xml_parser_create_ns`, `xml_parser_free`, `xml_parser_get_option`, `xml_parser_set_option`, `xml_set_character_data_handler`, `xml_set_default_handler`, `xml_set_element_handler`, `xml_set_end_namespace_decl_handler`, `xml_set_external_entity_ref_handler`, `xml_set_notation_decl_handler`, `xml_set_object`, `xml_set_processing_instruction_handler`, `xml_set_start_namespace_decl_handler`, `xml_set_unparsed_entity_decl_handler`

Missing: none

### hash

Implemented: `hash`, `hash_algos`, `hash_copy`, `hash_equals`, `hash_file`, `hash_final`, `hash_hkdf`, `hash_hmac`, `hash_hmac_algos`, `hash_hmac_file`, `hash_init`, `hash_pbkdf2`, `hash_update`, `hash_update_file`, `hash_update_stream`, `mhash`, `mhash_count`, `mhash_get_block_size`, `mhash_get_hash_name`, `mhash_keygen_s2k`

Missing: none

### SPL

Implemented: `class_implements`, `class_parents`, `class_uses`, `iterator_apply`, `iterator_count`, `iterator_to_array`, `spl_autoload`, `spl_autoload_call`, `spl_autoload_extensions`, `spl_autoload_functions`, `spl_autoload_register`, `spl_autoload_unregister`, `spl_classes`, `spl_object_hash`, `spl_object_id`

Missing: none

### ctype

Implemented: `ctype_alnum`, `ctype_alpha`, `ctype_cntrl`, `ctype_digit`, `ctype_graph`, `ctype_lower`, `ctype_print`, `ctype_punct`, `ctype_space`, `ctype_upper`, `ctype_xdigit`

Missing: none

### pcre

Implemented: `preg_filter`, `preg_grep`, `preg_last_error`, `preg_last_error_msg`, `preg_match`, `preg_match_all`, `preg_quote`, `preg_replace`, `preg_replace_callback`, `preg_replace_callback_array`, `preg_split`

Missing: none

### openssl

Implemented: `openssl_cipher_iv_length`, `openssl_cipher_key_length`, `openssl_decrypt`, `openssl_digest`, `openssl_encrypt`, `openssl_error_string`, `openssl_get_cipher_methods`, `openssl_get_md_methods`, `openssl_pbkdf2`, `openssl_random_pseudo_bytes`

Missing: `openssl_cms_decrypt`, `openssl_cms_encrypt`, `openssl_cms_read`, `openssl_cms_sign`, `openssl_cms_verify`, `openssl_csr_export`, `openssl_csr_export_to_file`, `openssl_csr_get_public_key`, `openssl_csr_get_subject`, `openssl_csr_new`, `openssl_csr_sign`, `openssl_dh_compute_key`, `openssl_free_key`, `openssl_get_cert_locations`, `openssl_get_curve_names`, `openssl_get_privatekey`, `openssl_get_publickey`, `openssl_open`, `openssl_pkcs12_export`, `openssl_pkcs12_export_to_file`, `openssl_pkcs12_read`, `openssl_pkcs7_decrypt`, `openssl_pkcs7_encrypt`, `openssl_pkcs7_read`, `openssl_pkcs7_sign`, `openssl_pkcs7_verify`, `openssl_pkey_derive`, `openssl_pkey_export`, `openssl_pkey_export_to_file`, `openssl_pkey_free`, `openssl_pkey_get_details`, `openssl_pkey_get_private`, `openssl_pkey_get_public`, `openssl_pkey_new`, `openssl_private_decrypt`, `openssl_private_encrypt`, `openssl_public_decrypt`, `openssl_public_encrypt`, `openssl_seal`, `openssl_sign`, `openssl_spki_export`, `openssl_spki_export_challenge`, `openssl_spki_new`, `openssl_spki_verify`, `openssl_verify`, `openssl_x509_check_private_key`, `openssl_x509_checkpurpose`, `openssl_x509_export`, `openssl_x509_export_to_file`, `openssl_x509_fingerprint`, `openssl_x509_free`, `openssl_x509_parse`, `openssl_x509_read`, `openssl_x509_verify`

### iconv

Implemented: `iconv`, `iconv_get_encoding`, `iconv_mime_decode`, `iconv_mime_decode_headers`, `iconv_mime_encode`, `iconv_set_encoding`, `iconv_strlen`, `iconv_strpos`, `iconv_strrpos`, `iconv_substr`

Missing: none

### random

Implemented: `getrandmax`, `lcg_value`, `mt_getrandmax`, `mt_rand`, `mt_srand`, `rand`, `random_bytes`, `random_int`, `srand`

Missing: none

### libxml

Implemented: `libxml_clear_errors`, `libxml_disable_entity_loader`, `libxml_get_errors`, `libxml_get_external_entity_loader`, `libxml_get_last_error`, `libxml_set_external_entity_loader`, `libxml_set_streams_context`, `libxml_use_internal_errors`

Missing: none

### filter

Implemented: `filter_has_var`, `filter_id`, `filter_input`, `filter_input_array`, `filter_list`, `filter_var`, `filter_var_array`

Missing: none

### fileinfo

Implemented: `finfo_buffer`, `finfo_close`, `finfo_file`, `finfo_open`, `finfo_set_flags`, `mime_content_type`

Missing: none

### json

Implemented: `json_decode`, `json_encode`, `json_last_error`, `json_last_error_msg`, `json_validate`

Missing: none

### tokenizer

Implemented: `token_get_all`, `token_name`

Missing: none

### posix

Implemented: `posix_isatty`

Missing: `posix_access`, `posix_ctermid`, `posix_eaccess`, `posix_errno`, `posix_get_last_error`, `posix_getcwd`, `posix_getegid`, `posix_geteuid`, `posix_getgid`, `posix_getgrgid`, `posix_getgrnam`, `posix_getgroups`, `posix_getlogin`, `posix_getpgid`, `posix_getpgrp`, `posix_getpid`, `posix_getppid`, `posix_getpwnam`, `posix_getpwuid`, `posix_getrlimit`, `posix_getsid`, `posix_getuid`, `posix_initgroups`, `posix_kill`, `posix_mkfifo`, `posix_mknod`, `posix_setegid`, `posix_seteuid`, `posix_setgid`, `posix_setpgid`, `posix_setrlimit`, `posix_setsid`, `posix_setuid`, `posix_strerror`, `posix_sysconf`, `posix_times`, `posix_ttyname`, `posix_uname`

### dom

Implemented: `dom_import_simplexml`

Missing: `dom\import_simplexml`

### PDO

Implemented: `pdo_drivers`

Missing: none

Every other extension in the table above (sodium, mysqli, gd, ldap, gmp, sockets, ftp, curl, pcntl, calendar, dba, bcmath, readline, bz2, zip, sysvmsg, sysvshm, shmop, exif, sysvsem, SPX, SimpleXML, soap) has
**no** functions implemented yet.

## Notable gaps

Functions that real applications hit early, grouped by what needs them
([roadmap.md](roadmap.md)).

| Area | Missing functions | Needed by |
|---|---|---|
| Date/time | `strtotime`, `gmdate`, `getdate`, `localtime`, `idate`, `date_*`, `timezone_*`, classes `DateTime`, `DateTimeImmutable`, `DateTimeZone`, `DateInterval`, `DatePeriod` | Carbon (Laravel), WordPress `current_time`, Drupal |
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
