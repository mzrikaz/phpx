<?php
function counter(int $from, int $to) { for ($i = $from; $i <= $to; $i++) { yield $i; } return "counted"; }
foreach (counter(1, 3) as $k => $v) { echo "$k=>$v "; }
echo "\n";
$g = counter(5, 6);
foreach ($g as $v) {}
echo $g->getReturn(), "\n";
function kv() { yield 'a' => 1; yield 'b' => 2; yield 10 => 'x'; yield 'y'; }
foreach (kv() as $k => $v) { echo "$k:$v "; }
echo "\n";
function echoer() { $received = []; while (true) { $x = yield count($received); if ($x === null) break; $received[] = $x; } return $received; }
$e = echoer();
var_dump($e->current(), $e->send('a'), $e->send('b'));
$e->next();
var_dump($e->valid(), $e->getReturn());
function inner() { yield 1; yield 2; return 3; }
function outer() { $r = yield from inner(); yield $r; yield from ['x' => 'arr']; yield 4; }
foreach (outer() as $k => $v) { echo "$k=$v "; }
echo "\n";
function naturals() { $n = 0; while (true) { yield $n++; } }
foreach (naturals() as $n) { if ($n > 4) break; echo $n; }
echo "\n";
function withFinally() { try { yield 1; yield 2; } finally { echo "[gen cleanup]"; } }
foreach (withFinally() as $v) { echo $v; }
echo "\n";
function failing() { yield 1; throw new RuntimeException("gen broke"); }
try { foreach (failing() as $v) { echo $v; } } catch (RuntimeException $e) { echo " caught: ", $e->getMessage(), "\n"; }
function catcher() { while (true) { try { yield; } catch (LogicException $e) { echo "inside caught ", $e->getMessage(), "\n"; yield "recovered"; } } }
$c = catcher(); $c->current();
var_dump($c->throw(new LogicException("x")));
function lazyMap(iterable $it, callable $f) { foreach ($it as $k => $v) { yield $k => $f($v); } }
function lazyFilter(iterable $it, callable $f) { foreach ($it as $k => $v) { if ($f($v)) yield $k => $v; } }
$pipeline = lazyFilter(lazyMap(naturals(), fn($x) => $x * $x), fn($x) => $x % 2 === 1);
foreach ($pipeline as $v) { if ($v > 50) break; echo $v, " "; }
echo "\n";
var_dump(iterator_to_array(kv()), iterator_to_array(counter(1, 3), false));
$gen = (function () { $x = yield 1; echo "got $x\n"; })();
foreach ($gen as $v) { echo "v=$v\n"; }
try { $r = counter(1, 2); $r->current(); $r->next(); $r->rewind(); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$fn = fn() => yield 'arrow';
foreach ($fn() as $v) { echo $v, "\n"; }
echo get_class(counter(1, 1)), " ", counter(1, 1) instanceof Traversable ? "traversable" : "no", "\n";
