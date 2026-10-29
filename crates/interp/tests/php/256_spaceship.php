<?php
// `<=>` on two ints compares in registers; anything else takes the general path.
function c($a, $b) { return $a <=> $b; }
$r = [];
for ($i = 0; $i < 1200; $i++) { $s = c($i % 7, 3) + c(3, $i % 5); }
$r[] = $s;
foreach ([[1,2],[2,1],[3,3],[PHP_INT_MIN,PHP_INT_MAX],[PHP_INT_MAX,PHP_INT_MIN],[-1,0],[1.5,1],["a","b"],[[1],[0]],[null,0],["10","9"],[true,false]] as [$a,$b]) $r[] = c($a,$b);
echo json_encode($r), "\n";
