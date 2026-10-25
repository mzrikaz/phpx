<?php
// phpx async model, written so real PHP (with the polyfill below) prints the same.
if (!function_exists('async')) {
    // Sequential polyfill for stock PHP: same results, no concurrency.
    final class PolyPromise {
        private $v; private $e;
        public function __construct(callable $f) { try { $this->v = $f(); } catch (Throwable $t) { $this->e = $t; } }
        public function wait() { if ($this->e) throw $this->e; return $this->v; }
        public function then(callable $f) { return new PolyPromise(fn() => $f($this->wait())); }
        public function catch(callable $f) { return new PolyPromise(function () use ($f) { try { return $this->wait(); } catch (Throwable $t) { return $f($t); } }); }
    }
    function async(callable $f) { return new PolyPromise($f); }
    function all_wait(array $ps) { return array_map(fn($p) => $p->wait(), $ps); }
} else {
    function all_wait(array $ps) { return Promise::all($ps)->wait(); }
}
$log = [];
function fetch(string $what, int $ms) { usleep($ms * 1000); return strtoupper($what); }
$a = async(fn() => fetch('users', 30));
$b = async(fn() => fetch('posts', 10));
echo $a->wait(), " ", $b->wait(), "\n";
$p = async(fn() => 21)->then(fn($x) => $x * 2);
echo $p->wait(), "\n";
$failing = async(function () { throw new RuntimeException("boom"); })->catch(fn($e) => "caught " . $e->getMessage());
echo $failing->wait(), "\n";
var_dump(all_wait(['x' => async(fn() => 1), 'y' => async(fn() => 2)]));
try { async(fn() => throw new LogicException("direct"))->wait(); } catch (LogicException $e) { echo "rethrown: ", $e->getMessage(), "\n"; }
$total = 0;
$ps = [];
foreach ([1, 2, 3] as $i) { $ps[] = async(function () use ($i, &$total) { usleep(1000 * (4 - $i)); $total += $i; return $i; }); }
var_dump(all_wait($ps), $total);
