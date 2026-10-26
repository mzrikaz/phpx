<?php
$q = implode('&', array_map(fn($i) => "v$i=$i", range(1, 1005)));
parse_str($q, $out); echo count($out), "\n";
$deep = 'a' . str_repeat('[x]', 64) . '=1&b' . str_repeat('[y]', 65) . '=2&c=3';
parse_str($deep, $o2); echo json_encode(array_keys($o2)), "\n";
$t = hrtime(true); parse_str(str_repeat('a[]=1&', 999), $o3); echo count($o3['a']), "\n";
parse_str('x[a][]=1&x[a][]=2&x[b]=3&x=4&y[]=5&y[k][z]=6', $o4); echo json_encode($o4), "\n";
