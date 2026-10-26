<?php
namespace App;
use Illuminate\Http\Request as R;
class P { public ?R $p = null; public function m(): R { return 1; } public function s(self $x) {} }
try { $o = new P; $o->p = 5; } catch (\TypeError $e) { echo explode(", called", $e->getMessage())[0], "\n"; }
try { (new P)->m(); } catch (\TypeError $e) { echo explode(", called", $e->getMessage())[0], "\n"; }
try { (new P)->s(1); } catch (\TypeError $e) { echo explode(", called", $e->getMessage())[0], "\n"; }
function g(R|int $x): int { return 1; } try { g([]); } catch (\TypeError $e) { echo explode(", called", $e->getMessage())[0], "\n"; }
