<?php
// JSON encode/decode of nested data.
$t = hrtime(true); $tot = 0;
$doc = []; for ($i = 0; $i < 500; $i++) { $doc[] = ['id' => $i, 'name' => "user$i", 'active' => $i % 2 == 0, 'score' => $i * 1.5, 'tags' => ['x', 'y', "t$i"], 'meta' => ['a' => $i, 'b' => null]]; }
for ($r = 0; $r < 100; $r++) {
    $s = json_encode($doc);
    $back = json_decode($s, true);
    $tot = ($tot + strlen($s) + count($back) + $back[$r]['id']) % 1000000007;
}
printf("json %.0f ms %d\n", (hrtime(true) - $t) / 1e6, $tot);
