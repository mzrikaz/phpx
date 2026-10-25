<?php
class K { function m() {} static function s() {} }
$a = function () {}; $b = function () {}; $k = new K; $k2 = new K;
$f = fn() => 1;
var_dump($a == $b, $a == $a, $a != $b, array_search($b, [$a, $b]), in_array($a, [$b]));
var_dump(strlen(...) == strlen(...), strlen(...) == strtoupper(...), $k->m(...) == $k->m(...), $k->m(...) == $k2->m(...), K::s(...) == K::s(...));
var_dump(Closure::fromCallable('strlen') == strlen(...), Closure::fromCallable([$k, 'm']) == $k->m(...), $f == $f);
$c1 = $a->bindTo($k); var_dump($c1 == $a);
