<?php
// Shaped like Laravel's retry() helper.
function retry_like(int $times, callable $cb) {
    $attempts = 0;
    beginning:
    $attempts++;
    $times--;
    try {
        return $cb($attempts);
    } catch (Exception $e) {
        if ($times < 1) {
            throw $e;
        }
        echo "retrying after attempt $attempts\n";
        goto beginning;
    }
}
echo retry_like(3, function ($n) { if ($n < 3) { throw new Exception("fail $n"); } return "ok on $n"; }), "\n";
try { retry_like(2, fn($n) => throw new Exception("always $n")); } catch (Exception $e) { echo "gave up: ", $e->getMessage(), "\n"; }
// Jump out of nested loops, and forward.
for ($i = 0; $i < 10; $i++) {
    for ($j = 0; $j < 10; $j++) {
        if ($i * $j == 12) { goto found; }
    }
}
echo "not found\n";
found:
echo "found $i x $j\n";
goto skip;
echo "skipped\n";
skip:
echo "done\n";
