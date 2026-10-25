<?php
echo ini_get('precision'), ' ', ini_get('serialize_precision'), ' ', ini_get('max_execution_time'), "\n";
var_dump(ini_get('no_such_setting'), ini_set('no_such_setting', 1));
echo ini_set('display_errors', '0'), ' ', ini_get('display_errors'), "\n";
ini_restore('display_errors'); echo ini_get('display_errors'), "\n";
echo error_reporting(), ' ', error_reporting(E_ALL & ~E_NOTICE), ' ', error_reporting(), "\n";
var_dump(date_default_timezone_set('Asia/Tokyo'), date_default_timezone_get());
print_r(parse_ini_string("; c\n[db]\nhost = localhost\nport=3306\ndebug = on\nlist[] = a\nlist[] = b\nname = \"x y\"\n", true));
var_dump(set_time_limit(30), ini_get('max_execution_time'));
var_dump(is_int(memory_get_usage()), memory_get_peak_usage() >= memory_get_usage());
