<?php
// preg_quote/addslashes/quotemeta: unchanged input, escapes, NUL, delimiter
foreach (['/products', 'plain', 'a.b*c?', "x\0y", 'it\'s "q" \\', '#hash#', ''] as $s) {
    var_dump(preg_quote($s), preg_quote($s, '#'), preg_quote($s, '/'), addslashes($s), quotemeta($s));
}
// short-key lookups of every length class
$a = [];
foreach (['', 'a', 'ab', 'abc', 'abcd', 'abcdefg', 'abcdefgh', 'abcdefghijklmnop', 'abcdefghijklmnopq'] as $k) { $a[$k] = strlen($k); }
foreach (['', 'a', 'ab', 'abd', 'abcd', 'abcdefh', 'abcdefgh', 'abcdefghijklmnoq', 'abcdefghijklmnopq', 'x'] as $k) {
    echo json_encode($k), ' ', isset($a[$k]) ? $a[$k] : '-', ' ', array_key_exists($k, $a) ? 'y' : 'n', "\n";
}
// negative literals in compiled code
function neg($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s += -3 + abs(-2) - -1; $f = -0.5; $g = -$f; } return [$s, $f, $g, -9223372036854775807 - 1, -PHP_INT_MAX, -0, -0.0]; }
for ($j = 0; $j < 3; $j++) var_dump(neg(1500));
