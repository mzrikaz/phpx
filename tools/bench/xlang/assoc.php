<?php
// Associative arrays: string keys, counting, nested maps, sorting.
$t = hrtime(true);
$words = []; for ($i = 0; $i < 5000; $i++) { $words[] = 'w' . ($i * 7919 % 1500); }
$tot = 0;
for ($r = 0; $r < 60; $r++) {
    $count = [];
    foreach ($words as $w) { $count[$w] = ($count[$w] ?? 0) + 1; }
    $byLen = [];
    foreach ($count as $w => $c) { $byLen[strlen($w)][$w] = $c; }
    arsort($count);
    $top = array_slice($count, 0, 10, true);
    $rows = [];
    for ($i = 0; $i < 2000; $i++) { $rows[] = ['id' => $i, 'name' => 'n' . $i, 'score' => $i % 97, 'tags' => ['a' => $i % 3, 'b' => $i % 5]]; }
    $sum = 0; foreach ($rows as $row) { if (isset($row['tags']['a']) && $row['score'] > 50) $sum += $row['tags']['b'] + $row['id']; }
    $tot = ($tot + count($count) + count($byLen) + array_sum($top) + $sum) % 1000000007;
}
printf("assoc %.0f ms %d\n", (hrtime(true) - $t) / 1e6, $tot);
