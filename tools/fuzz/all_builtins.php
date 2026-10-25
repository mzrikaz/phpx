<?php
// Call every built-in with random, hostile arguments; report Rust panics
// inside built-ins ("phpx internal error"). Usage:
//   phpx run tools/fuzz/all_builtins.php [seed] [calls-per-function]
// A crash of the process itself is a failure too (non-zero exit without
// the final "done" line).
$seed = (int)($argv[1] ?? 1);
$per = (int)($argv[2] ?? 30);
mt_srand($seed);
// Side effects, blocking, or process control.
$deny = '/^(exit|die|sleep|usleep|time_nanosleep|time_sleep_until|set_time_limit|ignore_user_abort|header|header_remove|setcookie|setrawcookie|http_response_code'
    . '|ob_|flush|output_|error_log|syslog|openlog|mail|register_shutdown_function|register_tick_function|set_error_handler|set_exception_handler|restore_|trigger_error|user_error'
    . '|ini_set|ini_alter|ini_restore|putenv|setlocale|chdir|chroot|umask|unlink|rmdir|mkdir|rename|copy|touch|chmod|chown|chgrp|lchown|lchgrp|link|symlink|tempnam|tmpfile'
    . '|file_put_contents|fopen|fwrite|fputs|fputcsv|ftruncate|flock|popen|pclose|proc_|exec|system|passthru|shell_exec|pcntl_|posix_kill|posix_set|fsockopen|pfsockopen|stream_socket|socket_'
    . '|move_uploaded_file|readline|spl_autoload|class_alias|define|extract|parse_str|eval|assert|debug_zval_dump|var_dump|print_r|var_export|printf|vprintf|fprintf|vfprintf|echo|print|readfile|fpassthru'
    . '|session_|gc_|go|async|await|spawn|defer|defer_|sleep_|phpx_|__phpx|serve|opendir|scandir|glob|dir|file|fgets|fread|fgetc|stream_get_contents|highlight_|php_strip|show_source|dl$|apcu_'
    . '|mt_srand|srand|random_bytes|openssl_random|stream_|fclose|feof|fflush|fseek|ftell|rewind|fstat|fscanf|fgetcsv|readdir|closedir|rewinddir|curl_|disk_|realpath_cache)/i';
$obj = new ArrayObject([1, 2]);
// (No large positive numbers: as sizes they correctly hit memory_limit,
// which ends the run.)
$vals = [0, 1, -1, 2, 3, 7, 255, 256, 1000, -2147483649, PHP_INT_MIN, 1.5, -0.0, NAN, INF, -INF, -1e100,
    "", "a", "abc", "0", "1", "-1", "1e3", " 12 ", "0x1A", "\0", "a\0b", "\xff\xfe\xfd", "\xc3\x28", "é", "日本語", str_repeat("x", 300),
    "%s%d%x%b%c%e%f%%", "%1\$s %2\$s", "/(a+)+\$/", "/[/", "~^\\d+~u", "a,b;c", "2024-01-01 12:00:00", "+1 day", "UTF-8", "ASCII", "Y-m-d H:i:s",
    "strlen", "strtoupper", "nope_fn", "ArrayObject", "stdClass", "Countable",
    [], [1, 2, 3], ["a" => 1, "b" => [2, 3]], [[1], [2]], ["x", "y"], [3 => "c", 1 => "a"], null, true, false,
    new stdClass, $obj, fn($a = null, $b = null) => $a, function (...$xs) { return 0; }];
$bad = 0;
$calls = 0;
$fns = get_defined_functions()['internal'];
sort($fns);
foreach ($fns as $f) {
    if (preg_match($deny, $f)) {
        continue;
    }
    for ($k = 0; $k < $per; $k++) {
        $n = mt_rand(0, 4);
        $args = [];
        for ($j = 0; $j < $n; $j++) {
            $args[] = $vals[mt_rand(0, count($vals) - 1)];
        }
        $calls++;
        try {
            @$f(...$args);
        } catch (Throwable $e) {
            if (str_contains($e->getMessage(), 'phpx internal error')) {
                $bad++;
                $shown = array_map(fn($v) => is_object($v) ? get_class($v) : (is_string($v) ? bin2hex($v) : json_encode($v)), $args);
                echo "INTERNAL $f(", implode(", ", $shown), "): ", $e->getMessage(), "\n";
            }
        }
    }
}
echo "done: $calls calls, $bad internal errors\n";
