<?php
class AppException extends RuntimeException {
    public function __construct(string $m, private string $extra = "e") { parent::__construct($m, 42); }
    public function extra() { return $this->extra; }
}
function risky($n) {
    if ($n === 1) throw new InvalidArgumentException("bad arg");
    if ($n === 2) throw new AppException("app failed", "ctx");
    if ($n === 3) return intdiv(1, 0);
    if ($n === 4) return [] > 0 ? 1 : strlen();
    return "ok";
}
foreach ([0, 1, 2, 3, 4] as $n) {
    try {
        echo risky($n), "\n";
    } catch (InvalidArgumentException | DivisionByZeroError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    } catch (AppException $e) {
        echo "App: ", $e->getMessage(), " code=", $e->getCode(), " extra=", $e->extra(), "\n";
    } catch (Throwable $e) {
        echo "Other: ", get_class($e), "\n";
    } finally {
        echo "finally $n\n";
    }
}
function nested() {
    try {
        try { throw new LogicException("inner"); }
        finally { echo "inner finally\n"; }
    } catch (LogicException $e) {
        throw new RuntimeException("outer", 0, $e);
    }
}
try { nested(); } catch (Exception $e) { echo $e->getMessage(), " <- ", $e->getPrevious()->getMessage(), "\n"; }
function finallyReturn() { try { return "try"; } finally { echo "cleanup\n"; } }
echo finallyReturn(), "\n";
function finallyOverride() { try { throw new Exception("lost"); } finally { return "finally wins"; } }
echo finallyOverride(), "\n";
for ($i = 0; $i < 5; $i++) {
    try { if ($i == 1) continue; if ($i == 3) break; echo "loop $i\n"; } finally { echo "f$i\n"; }
}
$r = null;
try { $val = $r ?? throw new Exception("thrown in expr"); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
echo "before uncaught\n";
throw new DomainException("uncaught!");
echo "never printed\n";
