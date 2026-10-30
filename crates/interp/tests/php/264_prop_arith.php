<?php
// $obj->p op= v on int and float properties, typed and untyped.
class C {
    public int $i = 0; public float $f = 0.0; public $u = 0; public ?int $ni = 1; public readonly int $ro; public string $s = "3";
    private int $priv = 10;
    function __construct() { $this->ro = 1; }
    function run($k) {
        $this->i += 2; $this->i -= 1; $this->i *= 3;
        $this->f += 1.5; $this->f -= 0.25; $this->f *= 2; $this->f /= 4; $this->f += 1;
        $this->u += 1; $this->u += 0.5; $this->u *= 2;
        $this->ni += 1; $this->priv += $k; $this->s += 1;
        $x = ($this->f += 10);
        try { $this->ro += 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
        try { $this->i += 1.5; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
        try { $this->i += "abc"; } catch (TypeError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
        $this->i = PHP_INT_MAX - 1;
        try { $this->i += 5; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
        $this->i = 0;
        $this->i++; ++$this->i; $this->i--;
        return [$this->i, $this->f, $this->u, $this->ni, $this->priv, $this->s, $x];
    }
}
function outside(C $o) {
    $o->f += 1; $o->i *= 2; $o->u -= 3;
    try { $o->priv += 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
    $o->dyn = 1; $o->dyn += 2;
    return [$o->f, $o->i, $o->u, $o->dyn];
}
for ($k = 0; $k < 3; $k++) { $o = new C; echo json_encode($o->run($k)), "\n"; echo json_encode(outside($o)), "\n"; }
$n = null; try { $n->x += 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
