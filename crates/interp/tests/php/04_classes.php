<?php
interface Shape { const SIDES = 0; public function area(): float; }
abstract class Base implements Shape {
    protected static int $count = 0;
    public function __construct(protected string $name) { static::$count++; self::log("created {$this->name}"); }
    public static function count(): int { return static::$count; }
    public static function log(string $m): void { echo "[", static::class, "] $m\n"; }
    abstract public function area(): float;
    public function describe(): string { return sprintf("%s with area %.2f", $this->name, $this->area()); }
    public function __toString(): string { return $this->describe(); }
}
final class Circle extends Base {
    const SIDES = 1;
    public function __construct(private float $r) { parent::__construct("circle"); }
    public function area(): float { return M_PI * $this->r ** 2; }
}
class Rect extends Base {
    const SIDES = 4;
    public function __construct(public readonly float $w, public readonly float $h) { parent::__construct("rect"); }
    public function area(): float { return $this->w * $this->h; }
    public static function square(float $s): static { return new static($s, $s); }
}
class Square extends Rect {}
$shapes = [new Circle(1), new Rect(2, 3), Square::square(2)];
foreach ($shapes as $s) { echo $s, " | sides=", $s::SIDES, " | ", get_class($s), "\n"; }
var_dump(Base::count(), $shapes[2] instanceof Rect, $shapes[2] instanceof Shape, $shapes[0] instanceof Rect, Shape::SIDES);
try { $shapes[1]->w = 10; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump($shapes[1]);
// Traits, static, late static binding.
trait Greets {
    public static int $greeted = 0;
    public function greet(): string { self::$greeted++; return "Hi from " . static::class . " (" . $this->who() . ")"; }
    abstract protected function who(): string;
}
trait Loud { public function greet(): string { return "LOUD"; } public function shout() { return strtoupper($this->greet()); } }
class Person { use Greets, Loud { Greets::greet insteadof Loud; Loud::greet as protected loudGreet; } protected function who(): string { return "person"; } public function both() { return $this->loudGreet(); } }
$p = new Person;
echo $p->greet(), " | ", $p->shout(), " | ", $p->both(), " | ", Person::$greeted, "\n";
// Magic methods.
class Magic implements ArrayAccess, Countable, IteratorAggregate {
    private array $data = [];
    public function __get($n) { echo "get $n\n"; return $this->data[$n] ?? null; }
    public function __set($n, $v) { echo "set $n\n"; $this->data[$n] = $v; }
    public function __isset($n) { return isset($this->data[$n]); }
    public function __unset($n) { unset($this->data[$n]); }
    public function __call($m, $args) { return "$m(" . implode(",", $args) . ")"; }
    public static function __callStatic($m, $args) { return "static $m"; }
    public function __invoke($x) { return $x * 2; }
    public function offsetExists($o): bool { return isset($this->data[$o]); }
    public function offsetGet($o): mixed { return $this->data[$o] ?? "none"; }
    public function offsetSet($o, $v): void { if ($o === null) { $this->data[] = $v; } else { $this->data[$o] = $v; } }
    public function offsetUnset($o): void { unset($this->data[$o]); }
    public function count(): int { return count($this->data); }
    public function getIterator(): Iterator { return new ArrayIterator($this->data); }
}
$m = new Magic;
$m->foo = 1;
echo $m->foo, "\n";
var_dump(isset($m->foo), isset($m->bar), $m->doThing(1, 2), Magic::stat(), $m(21));
$m['k'] = 'v';
$m[] = 'pushed';
unset($m->foo);
var_dump($m['k'], $m['missing'], isset($m['k']), count($m));
foreach ($m as $k => $v) { echo "$k=$v\n"; }
// Clone, handles, __clone.
class Node { public array $children = []; public ?Node $parent = null; public function __construct(public string $v) {} public function __clone() { $this->v .= "'"; } }
$a = new Node("a");
$b = $a;
$b->v = "b";
$c = clone $a;
var_dump($a->v, $c->v, $a === $b, $a === $c, $a == $c);
// Static vars and closures binding.
function counter() { static $n = 0; return ++$n; }
counter(); counter();
echo counter(), "\n";
class Ctx { private $secret = 's3'; }
$peek = function () { return $this->secret; };
echo Closure::bind($peek, new Ctx, Ctx::class)(), "\n";
// Enums.
enum Suit: string {
    case Hearts = 'H'; case Spades = 'S';
    const Wild = self::Spades;
    public function color(): string { return match ($this) { self::Hearts => 'Red', self::Spades => 'Black' }; }
    public static function fromChar(string $c): self { return self::from($c); }
}
enum Status { case On; case Off; }
var_dump(Suit::Hearts, Suit::Hearts->value, Suit::Hearts->name, Suit::from('S'), Suit::tryFrom('X'), Suit::Hearts->color(), Suit::Wild === Suit::Spades);
var_dump(Suit::cases(), Status::On === Status::On, Status::On instanceof UnitEnum, Suit::Hearts instanceof BackedEnum, Status::cases());
try { Suit::from('Z'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
// Anonymous class, stdClass, dynamic props.
$o = new class(5) { public function __construct(public int $x) {} public function double() { return $this->x * 2; } };
echo $o->double(), "\n";
$std = new stdClass; $std->a = 1; $std->list[] = 'x';
var_dump($std, (array) $std, (object) ['p' => 1]);
print_r(new Circle(2)); echo "\n";
print_r(['nested' => ['a' => 1, 'b' => [true, null]], 'obj' => $std]);
echo "\n";
var_export([1, 'a' => ['b' => true, 'c' => null], 2 => 1.0, 'str' => "it's"]);
echo "\n";
