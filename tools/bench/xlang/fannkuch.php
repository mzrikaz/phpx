<?php
// fannkuch-redux: integer array permutations.
function fannkuch(int $n): array {
    $p = range(0, $n - 1); $q = $p; $s = $p; $sign = 1; $maxflips = 0; $sum = 0;
    while (true) {
        $q0 = $p[0];
        if ($q0 != 0) {
            for ($i = 1; $i < $n; $i++) $q[$i] = $p[$i];
            $flips = 1;
            while (true) {
                $qq = $q[$q0];
                if ($qq == 0) { $sum += $sign * $flips; if ($flips > $maxflips) $maxflips = $flips; break; }
                $q[$q0] = $q0;
                if ($q0 >= 3) { $i = 1; $j = $q0 - 1; do { $t = $q[$i]; $q[$i] = $q[$j]; $q[$j] = $t; $i++; $j--; } while ($i < $j); }
                $q0 = $qq; $flips++;
            }
        }
        if ($sign == 1) { $t = $p[1]; $p[1] = $p[0]; $p[0] = $t; $sign = -1; }
        else {
            $t = $p[1]; $p[1] = $p[2]; $p[2] = $t; $sign = 1;
            for ($i = 2; $i < $n; $i++) {
                $sx = $s[$i];
                if ($sx != 0) { $s[$i] = $sx - 1; break; }
                if ($i == $n - 1) return [$sum, $maxflips];
                $s[$i] = $i;
                $t = $p[0]; for ($j = 0; $j <= $i; $j++) { $p[$j] = $p[$j + 1]; } $p[$i + 1] = $t;
            }
        }
    }
}
$t = hrtime(true);
[$sum, $flips] = fannkuch(10);
printf("fannkuch %.0f ms %d %d\n", (hrtime(true) - $t) / 1e6, $sum, $flips);
