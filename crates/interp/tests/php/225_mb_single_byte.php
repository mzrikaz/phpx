<?php
// Single-byte encodings: characters are bytes (Laravel Encrypter key check).
$k = "\x9d\x99\xdb\xa7\x83\x73\xc2\x29\x7a\xc3\x5d\xd6\x2c\xc3\x5c\xc6\xc7\x3b\x8b\xcd\x79\xa4\xb1\x97\x28\x83\x8e\xfc\x4d\x01\xbf\x8f";
foreach (['8bit', '8BIT', 'ISO-8859-1', 'latin1', 'Windows-1252'] as $e) {
    echo $e, ": ", mb_strlen($k, $e), " ",
        bin2hex(mb_substr($k, 3, 4, $e)), " ", bin2hex(mb_substr($k, -2, null, $e)), " ",
        var_export(mb_strpos($k, "\xc3", 0, $e), true), " ", var_export(mb_strrpos($k, "\xc3", 0, $e), true), "\n";
}
echo mb_strlen($k, 'ascii'), " ", mb_strlen($k), "\n";
echo mb_strlen("héllo", '8bit'), " ", mb_strlen("héllo"), "\n";
echo mb_substr("héllo", 0, 2, '8bit') === "h\xc3" ? "ok" : "bad", "\n";
var_dump(mb_strpos("abcabc", "c", 3, '8bit'), mb_strpos("abc", "z", 0, '8bit'));
print_r(array_map('bin2hex', mb_str_split("h\xc3\xa9llo", 2, '8bit')));
echo mb_substr_count("aaaa", "aa", '8bit'), "\n";
try { mb_strpos("abc", "a", 9, '8bit'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
