<?php
$f = new Fiber(function (string $x): string {
    echo "start $x\n";
    $y = Fiber::suspend('s1');
    echo "got $y\n";
    try { Fiber::suspend('s2'); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
    $z = Fiber::suspend('s3');
    return "ret $z";
});
var_dump($f->isStarted(), Fiber::getCurrent());
var_dump($f->start('a'), $f->isSuspended(), $f->isRunning());
var_dump($f->resume('b'));
var_dump($f->throw(new Exception('boom')));
var_dump($f->resume('c'), $f->isTerminated(), $f->getReturn());
try { $f->resume(); } catch (FiberError $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { Fiber::suspend(); } catch (FiberError $e) { echo $e->getMessage(), "\n"; }
$g = new Fiber(function () { echo Fiber::getCurrent() instanceof Fiber ? "in fiber\n" : "no\n"; throw new LogicException('inside'); });
try { $g->start(); } catch (LogicException $e) { echo 'propagated: ', $e->getMessage(), "\n"; }
// nested fibers
$outer = new Fiber(function () { $inner = new Fiber(function () { Fiber::suspend(1); return 2; }); $a = $inner->start(); Fiber::suspend($a + 10); $inner->resume(); return $inner->getReturn(); });
var_dump($outer->start(), $outer->resume(), $outer->getReturn());
