<?php
class R { function __construct(public readonly array &$a) {} }
$arr = [1];
try { $r = new R($arr); echo "constructed\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
