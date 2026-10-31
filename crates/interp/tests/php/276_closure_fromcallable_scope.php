<?php
class O {
    private $opts = [];
    function conf() { $r = []; $r['dsn'] = \Closure::fromCallable([$this, 'v']); return $r['dsn']('x'); }
    function conf2() { return \Closure::fromCallable([$this, 'v'])('y'); }
    private function v($d) { return "v:$d"; }
}
$o = new O; $s = '';
for ($i = 0; $i < 3000; $i++) { $s = $o->conf() . $o->conf2(); }
echo $s, "\n";
function outside($o) { try { return \Closure::fromCallable([$o, 'v']); } catch (Error $e) { return get_class($e) . ": " . $e->getMessage(); } }
echo outside($o), "\n";
class Sub extends O { function viaParent() { try { return \Closure::fromCallable([$this, 'v'])('z'); } catch (Error $e) { return get_class($e) . ": " . $e->getMessage(); } } }
echo (new Sub)->viaParent(), "\n";
echo \Closure::fromCallable('strtoupper')('ok'), "\n";
