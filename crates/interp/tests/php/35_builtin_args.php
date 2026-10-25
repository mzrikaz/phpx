<?php
function t($f) { try { $r = $f(); echo "ok ", var_export($r, true), "\n"; } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } }
for ($i = 0; $i < 2; $i++) {
    t(fn() => str_repeat("x", "3"));
    t(fn() => str_repeat("x", "3abc"));
    t(fn() => str_repeat("x", "abc"));
    t(fn() => str_repeat("x", 2.0));
    t(fn() => str_repeat("x", true));
    t(fn() => str_repeat([], 2));
    t(fn() => strlen(12345));
    t(fn() => strlen(1.5));
    t(fn() => strlen());
    t(fn() => strlen("a", "b"));
    t(fn() => substr("abc"));
    t(fn() => substr("abc", 1, 2, 3));
    t(fn() => in_array(1, "x"));
    t(fn() => array_map("nope", [1]));
    t(fn() => usort($a, "nope"));
    t(fn() => implode(",", "x"));
    t(fn() => max());
    t(fn() => count(5));
    t(fn() => round("1.5x"));
    t(fn() => round("1.5"));
    t(fn() => abs("x"));
    t(fn() => abs("-3"));
    t(fn() => str_pad("a", 5, ...["pad_type" => STR_PAD_LEFT]));
    t(fn() => str_pad(string: "a", length: 3, pad_string: "-"));
    t(fn() => json_encode(value: [1, 2], flags: JSON_PRETTY_PRINT));
    t(fn() => strlen(str: "x"));
    t(fn() => htmlspecialchars("<a href='x'>&amp;</a>", double_encode: false));
    t(fn() => array_slice([1, 2, 3, 4], 1, preserve_keys: true));
    t(fn() => in_array("1", [1, 2], strict: true));
    t(fn() => array_fill(start_index: 5, count: 2, value: 'v'));
    t(fn() => number_format(1234.5678, decimals: 2));
    t(fn() => explode(separator: ",", string: "a,b,c", limit: 2));
    t(fn() => strlen(string: "x", string2: "y"));
    t(fn() => array_keys([1, 2, 1], filter_value: 1));
    t(fn() => preg_split('/,/', 'a,b', flags: PREG_SPLIT_NO_EMPTY));
    t(fn() => strtoupper(new class { function __toString() { return "obj"; } }));
    t(fn() => intdiv(7, "2"));
    t(fn() => is_numeric(null));
    t(fn() => in_array(needle: 2, haystack: [1, 2]));
}
