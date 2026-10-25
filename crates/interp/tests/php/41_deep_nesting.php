<?php
echo eval('return 1' . str_repeat(' + 1', 2000) . ';'), "\n";
echo eval('return ' . str_repeat('(', 2000) . '7' . str_repeat(')', 2000) . ';'), "\n";
$a = 1; for ($i = 0; $i < 3000; $i++) { $a = [$a]; }
echo strlen(serialize($a)), " ", strlen(json_encode($a, 0, 5000)), " ", json_encode($a) === false ? json_last_error_msg() : "ok", "\n";
foreach ([[1], [[1]], [[[1]]], 5, []] as $v) { foreach ([0, 1, 2, 3] as $d) { echo json_encode($v, 0, $d) === false ? "F" : "T"; } echo " "; }
echo "\nend\n";
