<?php
class P1 { public $a = 1; }
class U { public $name; public $email; public $id; public function __construct($i) { $this->id = $i; $this->name = "user$i"; $this->email = "u$i@example.com"; } }
function m($label, $f) { $N = 100000; $m0 = memory_get_usage(); $x = $f($N); printf("%-10s %6.1f\n", $label, (memory_get_usage() - $m0) / $N); }
m('P1', function ($N) { $a = []; for ($i = 0; $i < $N; $i++) { $a[] = new P1; } return $a; });
m('U', function ($N) { $a = []; for ($i = 0; $i < $N; $i++) { $a[] = new U($i); } return $a; });
m('assoc3', function ($N) { $a = []; for ($i = 0; $i < $N; $i++) { $a[] = ['id' => $i, 'name' => 'n', 'x' => 1]; } return $a; });
m('list2', function ($N) { $a = []; for ($i = 0; $i < $N; $i++) { $a[] = [$i, $i]; } return $a; });
m('str10', function ($N) { $a = []; for ($i = 0; $i < $N; $i++) { $a[] = str_repeat('x', 10); } return $a; });
m('int', function ($N) { $a = []; for ($i = 0; $i < $N; $i++) { $a[] = $i; } return $a; });
m('intkeys', function ($N) { $a = []; for ($i = 0; $i < $N; $i++) { $a[$i * 2] = $i; } return $a; });
m('strkeys', function ($N) { $a = []; for ($i = 0; $i < $N; $i++) { $a["k$i"] = $i; } return $a; });
