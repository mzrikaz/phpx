<?php
// Closures: array_map / array_filter / usort / array_reduce with captured values.
$t = hrtime(true); $tot = 0;
$data = []; for ($i = 0; $i < 3000; $i++) { $data[] = ($i * 2654435761) % 10007; }
for ($r = 0; $r < 100; $r++) {
    $k = $r % 7 + 1;
    $m = array_map(fn($x) => $x * $k + 1, $data);
    $f = array_filter($m, function ($x) use ($k) { return $x % ($k + 2) === 0; });
    usort($f, fn($a, $b) => $b <=> $a);
    $s = array_reduce($f, fn($c, $x) => ($c + $x) % 1000003, 0);
    $tot = ($tot + $s + count($f)) % 1000000007;
}
printf("closures %.0f ms %d\n", (hrtime(true) - $t) / 1e6, $tot);
