<?php
$re = '{^(?|/app/([^/]++)/(?|products(*:30)|brands(*:45))|/x(*:60))/?$}sDu';
foreach (['/app/abc/products', '/app/abc/brands', '/x', '/nope'] as $s) {
    var_dump(preg_match($re, $s, $m), $m);
}
var_dump(preg_match('/a(*MARK:foo)b|c(*MARK:bar)d/', 'cd', $m), $m);
preg_match_all('/(?:a(*:A)|b(*:B))/', 'ab', $mm, PREG_SET_ORDER); var_dump($mm);
preg_match_all('/(?:a(*:A)|b(*:B)|c)/', 'abc', $mm); var_dump($mm);
echo preg_replace_callback('/x(*:one)|y(*:two)/', fn($m) => '[' . $m['MARK'] . ']', 'xy'), "\n";
var_dump(preg_match('/(*MARK:A)x/i', 'zX', $m), $m);
