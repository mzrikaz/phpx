<?php
$vals = [0, -1, PHP_INT_MIN, PHP_INT_MAX, 1.9, -2.5, "12abc", "x", true, false, null, "", 3e20];
foreach (['%d', '%s', '[%d|%s]', '%%d%d%%', 'a%sb%dc', '%5d', '%-3s|', '%1$s %1$d', '%d%d', '%s%'] as $f) {
  foreach ($vals as $v) { try { echo sprintf($f, $v, $v), "\n"; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; } }
}
echo sprintf('no conversions'), sprintf(''), sprintf('%s', new class { function __toString() { return 'obj'; } }), "\n";
try { echo sprintf('%d %d', 1); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
