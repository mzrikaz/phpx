<?php
$out = '';
foreach ([2,3,4,5,6,7,8,15,16,17,30,100,1100,3000] as $n) {
  $a = []; for ($i = 0; $i < $n; $i++) $a[] = ($i * 7919) % 97;
  $b = $a; usort($b, fn($x, $y) => (($x * 31 + $y * 17) % 3) - 1); $out .= md5(json_encode($b)) . "\n";
  $b = $a; usort($b, fn($x, $y) => -1); $out .= md5(json_encode($b)) . "\n";
  $b = $a; usort($b, fn($x, $y) => 1); $out .= md5(json_encode($b)) . "\n";
  $b = $a; sort($b); $out .= md5(json_encode($b)) . "\n";
  $b = $a; arsort($b); $out .= md5(json_encode($b)) . "\n";
  $b = $a; uasort($b, fn($x, $y) => $x % 5 <=> $y % 5); $out .= md5(json_encode($b)) . "\n";
}
echo md5($out), "\n";
echo md5($out), "\n";
