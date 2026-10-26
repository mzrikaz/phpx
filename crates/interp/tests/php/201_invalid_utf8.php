<?php
// Built-ins given invalid UTF-8 (lossy conversions must terminate and match
// PHP). Strings are shown as hex: the output itself stays valid UTF-8.
function show(...$vs) { foreach ($vs as $v) { echo is_string($v) ? bin2hex($v) : json_encode($v), "\n"; } }
$bad = ["a\xffb", "\xc3", "\xe2\x82", "ok\x80\x80end", "\xf0\x9f\x98"];
foreach ($bad as $s) {
    show(mb_strlen($s), mb_substr($s, 1), mb_strtoupper($s), strtoupper($s), json_encode($s), htmlspecialchars($s));
    show(str_contains($s, 'b'), ucfirst($s), explode("\xff", $s), sprintf('%s|%5s', $s, $s), md5($s));
    $a = [$s => 1]; show(array_key_exists($s, $a), array_keys($a) === [$s]);
}
show(htmlspecialchars("a\xffb", ENT_QUOTES), htmlspecialchars("a\xffb\xc3", ENT_QUOTES | ENT_IGNORE), htmlspecialchars("<\xff>", ENT_SUBSTITUTE), htmlspecialchars("ok\xff", ENT_QUOTES, 'ISO-8859-1'));
