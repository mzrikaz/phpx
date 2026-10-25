<?php
$tpl = sys_get_temp_dir() . '/phpx_tpl_' . getmypid() . '.php';
file_put_contents($tpl, '<?php foreach ([1, 2] as $i) { echo $this->name($i), "\n"; } $k = 0; while ($k < 600) { $k++; } echo $k, "\n";');
class R { function name($i) { return "n$i"; } function render($t) { include $t; } }
(new R)->render($tpl);
unlink($tpl);
