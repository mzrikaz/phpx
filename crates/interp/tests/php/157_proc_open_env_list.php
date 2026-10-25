<?php
$p = proc_open('echo $A-$B', [1 => ["pipe","w"]], $pipes, null, ["A=1", "B" => "2"]);
echo stream_get_contents($pipes[1]);
