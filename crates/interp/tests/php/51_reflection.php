<?php
namespace App;

use Attribute;

#[Attribute(Attribute::TARGET_CLASS | Attribute::TARGET_METHOD)]
class Route { public function __construct(public string $path, public array $methods = ['GET']) {} }

interface Repo { public function find(int $id): ?array; }
trait Logs { protected array $log = []; public function log(string $m): void { $this->log[] = $m; } }

abstract class Base implements Repo {
    const VERSION = '1.0';
    protected static int $count = 0;
    abstract protected function table(): string;
}

/**
 * A user repository.
 */
#[Route('/users', methods: ['GET', 'POST'])]
final class UserRepo extends Base implements \Countable {
    use Logs;
    public const LIMIT = 10;
    private const SECRET = 's';
    public ?string $name = null;
    private int $hits = 0;
    public function __construct(private Repo|null $inner = null, public readonly int $size = self::LIMIT, string ...$tags) {}
    protected function table(): string { return 'users'; }
    public function find(int $id, bool $strict = false, array $opts = [], ?self $other = null): ?array { $this->hits++; return ['id' => $id]; }
    public function count(): int { return $this->hits; }
    #[Route('/x')] public static function make(int|string $a = PHP_INT_MAX, \Closure|null $cb = null): static { return new static(); }
    private function secret(): string { return 'hidden ' . $this->hits; }
}

$r = new \ReflectionClass(UserRepo::class);
var_dump($r->getName(), $r->getShortName(), $r->getNamespaceName(), $r->isFinal(), $r->isAbstract(), $r->isInstantiable(), $r->isInterface());
var_dump($r->getParentClass()->getName(), $r->getInterfaceNames(), $r->getTraitNames(), $r->implementsInterface('Countable'), $r->isSubclassOf(Base::class));
var_dump($r->getDocComment(), $r->getStartLine(), $r->getEndLine(), is_string($r->getFileName()));
var_dump($r->getConstants(), $r->hasConstant('SECRET'), $r->getConstant('VERSION'));
$ctor = $r->getConstructor();
foreach ($ctor->getParameters() as $p) {
    echo $p->getPosition(), ' ', $p->getName(), ' type=', $p->getType() ?? 'none', ' builtin=', var_export($p->getType() instanceof \ReflectionNamedType ? $p->getType()->isBuiltin() : null, true),
        ' null=', var_export($p->allowsNull(), true), ' opt=', var_export($p->isOptional(), true), ' var=', var_export($p->isVariadic(), true), ' prom=', var_export($p->isPromoted(), true);
    if ($p->isDefaultValueAvailable()) { echo ' def=', var_export($p->getDefaultValue(), true), ' const=', var_export($p->getDefaultValueConstantName(), true); }
    echo "\n";
}
var_dump($ctor->getNumberOfParameters(), $ctor->getNumberOfRequiredParameters());
$m = $r->getMethod('find');
foreach ($m->getParameters() as $p) { echo $p->getName(), ':', $p->getType(), ' ', $p->isDefaultValueAvailable() ? var_export($p->getDefaultValue(), true) : '-', "\n"; }
var_dump((string) $m->getReturnType(), $m->getReturnType()->allowsNull(), $m->isPublic(), $m->getDeclaringClass()->getName());
$mk = $r->getMethod('make');
var_dump($mk->isStatic(), (string) $mk->getReturnType(), (string) $mk->getParameters()[0]->getType(), $mk->getParameters()[0]->getDefaultValueConstantName());
var_dump(array_map(fn($m) => $m->getName() . '@' . $m->class, $r->getMethods()));
var_dump(array_map(fn($m) => $m->getName(), $r->getMethods(\ReflectionMethod::IS_STATIC)));
var_dump(array_map(fn($p) => $p->getName() . '@' . $p->class, $r->getProperties()));
$o = $r->newInstanceArgs([null, 5]);
var_dump($o->size);
$s = $r->getMethod('secret'); var_dump($s->invoke($o));
$h = $r->getProperty('hits'); var_dump($h->getValue($o), $h->isPrivate(), (string) $h->getType()); $h->setValue($o, 42); var_dump($s->invoke($o), $h->hasDefaultValue(), $h->getDefaultValue());
var_dump($r->getProperty('size')->isReadOnly(), $r->getProperty('size')->isPromoted(), $r->getProperty('name')->getDefaultValue());
var_dump($r->getStaticPropertyValue('count'));
$x = $r->newInstanceWithoutConstructor(); var_dump($r->getProperty('size')->isInitialized($x));
foreach ($r->getAttributes() as $a) { var_dump($a->getName(), $a->getArguments()); $i = $a->newInstance(); var_dump($i->path, $i->methods); }
var_dump(count($mk->getAttributes(Route::class)), count($r->getAttributes(\stdClass::class)));
$f = new \ReflectionFunction(function (int $a, $b = [1, 2]) use ($o): int { return $a; });
var_dump($f->getNumberOfParameters(), $f->getParameters()[1]->getDefaultValue(), (string) $f->getReturnType(), $f->isClosure(), array_keys($f->getStaticVariables()));
$g = new \ReflectionFunction('str_replace'); var_dump($g->isInternal(), $g->getNumberOfParameters(), $g->getParameters()[3]->isPassedByReference());
try { new \ReflectionClass('Nope\Missing'); } catch (\ReflectionException $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { $r->getMethod('zzz'); } catch (\ReflectionException $e) { echo $e->getMessage(), "\n"; }
try { $r->getProperty('zzz'); } catch (\ReflectionException $e) { echo $e->getMessage(), "\n"; }
var_dump((new \ReflectionClass(Base::class))->isInstantiable(), (new \ReflectionClass(Repo::class))->isInterface(), (new \ReflectionClass(Logs::class))->isTrait());
enum Suit: string { case Hearts = 'H'; case Spades = 'S'; const Wild = self::Spades; }
$e = new \ReflectionEnum(Suit::class);
var_dump($e->isBacked(), (string) $e->getBackingType(), count($e->getCases()), $e->getCase('Spades')->getBackingValue(), $e->isFinal());
var_dump((\ReflectionMethod::createFromMethodName('App\UserRepo::count'))->getName(), (new \ReflectionNamedType('int', false, true)) instanceof \ReflectionType);
