<?php
class B { function __construct() { echo "B "; } }
class A { public $b; function __construct($b) { echo "A "; $this->b = $b; } }
class P { public $x = 1; }
function mk() { echo "arg "; return 5; }
for ($i = 0; $i < 3; $i++) { $a = new A(new B); echo spl_object_id($a), spl_object_id($a->b), " "; $p = new P(mk()); echo spl_object_id($p), "\n"; }
var_dump(new A(new B));
