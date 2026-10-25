<?php
$p = proc_open(['sh', '-c', 'echo hi'], [1 => ['pipe', 'w']], $pipes);
usleep(200000);
$r = [1 => $pipes[1]]; $w = []; $e = [];
var_dump(stream_select($r, $w, $e, 0, 0), $r === [1 => $pipes[1]] || count($r));
$r = [1 => $pipes[1]]; $w = null; $e = [];
