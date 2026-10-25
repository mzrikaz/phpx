<?php
class P { public array $pipes = []; public $u; }
$o = new P; preg_match('/(a)/', 'a', $o->pipes); var_dump(count($o->pipes));
$o2 = new P; preg_match('/(a)/', 'a', $o2->u); var_dump(count($o2->u));
class Q { public ?P $p = null; }
$q = new Q; $q->p = new P; preg_match('/(a)/', 'a', $q->p->u); var_dump(count($q->p->u));
preg_match('/(a)/', 'a', $q->p->pipes); var_dump(count($q->p->pipes));
