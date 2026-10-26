<?php
class D { function __destruct() { echo "destruct\n"; } }
$d = new D; $x = 5;
register_shutdown_function(function () { global $x; echo "shutdown ", isset($_SERVER['argv']) ? 'server' : 'noserver', " x=$x\n"; });
echo "end\n";
