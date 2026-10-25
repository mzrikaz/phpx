<?php
class P1 { public $a = 1; }
class U { public $name; public $email; public $id; public function __construct($i) { $this->id = $i; $this->name = "user$i"; $this->email = "u$i@example.com"; } }
$a = []; for ($i = 0; $i < 1000000; $i++) { $a[] = new P1; }
$b = []; for ($i = 0; $i < 300000; $i++) { $b[] = new U($i); }
$c = []; for ($i = 0; $i < 300000; $i++) { $c[] = ['id' => $i, 'name' => "n$i", 'tags' => ['a', 'b']]; }
$s = []; for ($i = 0; $i < 500000; $i++) { $s[] = str_repeat('x', $i % 50); }
preg_match_all('/(VmHWM|VmRSS):\s+(\d+)/', file_get_contents('/proc/self/status'), $m); echo implode(' ', $m[0]), ' usage=', memory_get_usage(), "\n";
