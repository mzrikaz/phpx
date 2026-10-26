<?php
function run() {
    $a = [];
    for ($i = 0; $i < 20; $i++) { $a["k$i"] = $i; if ($i % 3 == 0) $a[$i * 10] = "n$i"; }
    unset($a['k3'], $a['k0'], $a[90], $a['k19']);
    $a['k3'] = 'back';
    echo implode(',', array_keys($a)), "\n";
    foreach (['k1', 'k3', 'k0', 'k18', 0, 30, 90, '30', 'nope'] as $k) { var_dump(isset($a[$k]), array_key_exists($k, $a), $a[$k] ?? '-'); }
    echo array_pop($a), ' ', count($a), ' ', array_key_last($a), "\n";
    $a[] = 'next'; echo array_key_last($a), "\n";
    $b = $a; $b['k1'] = 'changed'; echo $a['k1'], ' ', $b['k1'], "\n";
    while (count($a) > 3) { unset($a[array_key_first($a)]); }
    var_dump($a);
    $c = ['x' => 1, 'y' => 2]; reset($c); next($c); echo key($c), "\n";
    for ($i = 0; $i < 12; $i++) $c["z$i"] = $i;
    echo key($c), ' ', current($c), "\n";
    $r = &$c['x']; $r = 99; $d = $c; $r = 100; echo $d['x'], ' ', $c['x'], "\n";
    $m = array_merge(['id' => 'int'], ['a' => 'bool', 'id' => 'string', 5 => 'five']);
    var_dump($m);
    $f = array_flip(range('a', 'm')); echo $f['j'], ' ', count($f), "\n";
    unset($f['a']); unset($f['m']); echo implode('', array_keys($f)), ' ', $f['l'], "\n";
}
run(); run();
