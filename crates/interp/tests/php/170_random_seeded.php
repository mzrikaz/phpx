<?php
use Random\Randomizer; use Random\Engine\{Mt19937, PcgOneseq128XslRr64, Xoshiro256StarStar};
use Random\IntervalBoundary as IB;
mt_srand(42);
echo mt_rand(), " ", mt_rand(1, 100), " ", mt_rand(-5, 5), " ", rand(10, 1), " ", rand(), " ", mt_rand(0, PHP_INT_MAX), " ", mt_rand(PHP_INT_MIN, PHP_INT_MAX), "\n";
$a = range(1, 20); shuffle($a); echo implode(",", $a), "\n";
echo str_shuffle("abcdefghijklmnop"), "\n";
echo json_encode(array_rand(['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4, 'e' => 5], 3)), " ", array_rand([5, 6, 7]), " ", json_encode(array_rand(range(0, 9), 8)), "\n";
srand(7); echo rand(1, 6), rand(1, 6), rand(1, 6), "\n";
foreach ([new Mt19937(1234), new PcgOneseq128XslRr64(1234), new Xoshiro256StarStar(1234), new PcgOneseq128XslRr64(str_repeat("\x01", 16)), new Xoshiro256StarStar(str_repeat("ab", 16))] as $e) {
  $r = new Randomizer($e);
  echo get_class($e), ": ", bin2hex($e->generate()), " ", $r->nextInt(), " ", $r->getInt(1, 1000), " ", $r->getInt(PHP_INT_MIN, PHP_INT_MAX), " ", $r->nextFloat(), "\n";
  echo "  ", $r->getFloat(0, 1), " ", $r->getFloat(-10.5, 3.25, IB::ClosedClosed), " ", $r->getFloat(1, 2, IB::OpenOpen), " ", $r->getFloat(-1e10, -5, IB::OpenClosed), "\n";
  echo "  ", bin2hex($r->getBytes(13)), " ", $r->getBytesFromString("abcdefghijklmnopqrstuvwxyz0123456789", 16), " ", strlen($r->getBytesFromString(str_repeat("xy", 300), 5)), "\n";
  echo "  ", implode(",", $r->shuffleArray(['x' => 1, 2, 3, 4, 5, 6])), " ", $r->shuffleBytes("hello world"), " ", json_encode($r->pickArrayKeys(['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4], 2)), "\n";
}
$p = new PcgOneseq128XslRr64(5); $p->jump(1000); echo bin2hex($p->generate()), "\n";
$x = new Xoshiro256StarStar(5); $x->jump(); echo bin2hex($x->generate()), " "; $x->jumpLong(); echo bin2hex($x->generate()), "\n";
$m = new Mt19937(3); $c = clone $m; echo bin2hex($m->generate()), bin2hex($c->generate()), "\n";
class MyEngine implements Random\Engine { private $i = 0; function generate(): string { return chr($this->i++ % 256) . "\x00\x10"; } }
$r = new Randomizer(new MyEngine); echo $r->getInt(1, 100), " ", $r->nextInt(), " ", bin2hex($r->getBytes(7)), " ", implode(",", $r->shuffleArray([1,2,3,4])), "\n";
try { $r->getInt(5, 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { new PcgOneseq128XslRr64("short"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { array_rand([1, 2], 5); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { mt_rand(5, 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
mt_srand(3, MT_RAND_PHP); echo mt_rand(1, 10), mt_rand(), "\n";
var_dump(mt_getrandmax(), MT_RAND_MT19937);
