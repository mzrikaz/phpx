<?php
function route_get($u, $f) { return $f; }
$f = route_get('/test', function() {
        return 1;
});
$g = route_get('/x', fn() =>
   2);
$h = [function () {
    return 3;
}][0];
foreach ([$f, $g, $h] as $c) { $r = new ReflectionFunction($c); echo $r->getStartLine(), '-', $r->getEndLine(), "\n"; }
