<?php
enum Suit { case Hearts; }
class P { public $a = [1, [2]]; public $o; public $e = Suit::Hearts; }
$p = new P; $p->o = new stdClass; $p->o->z = [3];
var_export([$p, Suit::Hearts, [[]], (object)[], 1.0, "a'b", null, false]); echo "\n";
var_export(new stdClass); echo "\n"; var_export([]); echo "\n";
