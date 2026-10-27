<?php
// String building, replace, split/join, case, substring search.
$t = hrtime(true); $h = 0;
for ($r = 0; $r < 200; $r++) {
    $parts = [];
    for ($i = 0; $i < 1000; $i++) { $parts[] = 'item-' . $i . ':' . ($i * 7 % 13); }
    $s = implode(',', $parts);
    $s = str_replace('item', 'Elem', $s);
    $up = strtoupper($s);
    $cnt = 0; foreach (explode(',', $up) as $p) { if (str_contains($p, ':1')) $cnt++; $cnt += strlen(substr($p, 2, 3)); }
    $buf = ''; for ($i = 0; $i < 2000; $i++) { $buf .= "x{$i}y"; }
    $h = ($h + $cnt + strlen($buf) + strpos($up, 'ELEM-999')) % 1000000007;
}
printf("strings %.0f ms %d\n", (hrtime(true) - $t) / 1e6, $h);
