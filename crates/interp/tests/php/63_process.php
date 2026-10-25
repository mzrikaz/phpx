<?php
var_dump(exec('printf "a\nb  \nc"', $out, $code), $out, $code);
var_dump(shell_exec('echo hi; echo there'), shell_exec('true'));
$last = system('echo one; echo two; exit 3', $rc); var_dump($last, $rc);
passthru('echo pass', $rc2); var_dump($rc2);
$p = proc_open('cat; echo err >&2; exit 5', [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes);
fwrite($pipes[0], "hello\nworld\n"); fclose($pipes[0]);
var_dump(stream_get_contents($pipes[1]), stream_get_contents($pipes[2]));
fclose($pipes[1]); fclose($pipes[2]);
$st = proc_get_status($p); var_dump(is_int($st['pid']), $st['command']);
var_dump(proc_close($p));
$p = proc_open(['printf', '%s|%s', 'x y', 'z'], [1 => ['pipe', 'w']], $pipes); var_dump(stream_get_contents($pipes[1]), proc_close($p));
$p = proc_open('sleep 5', [], $pipes); var_dump(proc_terminate($p)); usleep(100000); $s = proc_get_status($p); var_dump($s['running'], $s['signaled'], $s['termsig']);
$h = popen('echo from popen', 'r'); var_dump(stream_get_contents($h), pclose($h));
$p = proc_open('echo $FOO; pwd', [1 => ['pipe', 'w']], $pipes, '/tmp', ['FOO' => 'bar']); var_dump(stream_get_contents($pipes[1])); proc_close($p);
$d = [1 => ['pipe', 'w']]; $p = proc_open('echo x', $d, $pipes); var_dump(stream_get_meta_data($pipes[1])['stream_type'], stream_get_meta_data($pipes[1])['mode'], stream_get_contents($pipes[1])); proc_close($p);
