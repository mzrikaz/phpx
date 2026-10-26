
// ---- Attributes ---------------------------------------------------------------

#[Attribute(Attribute::TARGET_CLASS)]
final class Attribute
{
    const TARGET_CLASS = 1;
    const TARGET_FUNCTION = 2;
    const TARGET_METHOD = 4;
    const TARGET_PROPERTY = 8;
    const TARGET_CLASS_CONSTANT = 16;
    const TARGET_PARAMETER = 32;
    const TARGET_CONSTANT = 64;
    const TARGET_ALL = 127;
    const IS_REPEATABLE = 128;

    public function __construct(public int $flags = Attribute::TARGET_ALL) {}
}

#[Attribute(Attribute::TARGET_METHOD)]
final class ReturnTypeWillChange { public function __construct() {} }

#[Attribute(Attribute::TARGET_CLASS)]
final class AllowDynamicProperties { public function __construct() {} }

#[Attribute(Attribute::TARGET_PARAMETER)]
final class SensitiveParameter { public function __construct() {} }

#[Attribute(Attribute::TARGET_METHOD)]
final class Override { public function __construct() {} }

#[Attribute(Attribute::TARGET_METHOD | Attribute::TARGET_FUNCTION | Attribute::TARGET_CLASS_CONSTANT)]
final class Deprecated
{
    public function __construct(public readonly ?string $message = null, public readonly ?string $since = null) {}
}

#[Attribute(Attribute::TARGET_METHOD | Attribute::TARGET_FUNCTION)]
final class NoDiscard
{
    public function __construct(public readonly ?string $message = null) {}
}

// ---- Reflection -----------------------------------------------------------------

class ReflectionException extends Exception {}

interface Reflector extends Stringable {}

/** Shared helpers (not part of PHP's API). */
final class __PhpxRefl
{
    public static function type(?array $t): ?ReflectionType
    {
        if ($t === null) {
            return null;
        }
        return match ($t[0]) {
            'named' => new ReflectionNamedType($t[1], $t[2], $t[3]),
            'union' => new ReflectionUnionType(array_map([self::class, 'type'], $t[1]), $t[2]),
            default => new ReflectionIntersectionType(array_map([self::class, 'type'], $t[1]), $t[2]),
        };
    }

    public static function attributes(array $list, ?string $filter, int $flags, int $target): array
    {
        $out = [];
        foreach ($list as [$name, $id]) {
            if ($filter !== null) {
                if ($flags & ReflectionAttribute::IS_INSTANCEOF) {
                    if (!is_a($name, $filter, true)) {
                        continue;
                    }
                } elseif (strcasecmp(ltrim($filter, '\\'), $name) !== 0) {
                    continue;
                }
            }
            $repeated = 0;
            foreach ($list as [$other]) {
                if (strcasecmp($other, $name) === 0) {
                    $repeated++;
                }
            }
            $out[] = new ReflectionAttribute($name, $id, $target, $repeated > 1);
        }
        return $out;
    }

    public static function modifiers(array $d): int
    {
        $m = $d['vis'];
        if (!empty($d['static'])) {
            $m |= 16;
        }
        if (!empty($d['final'])) {
            $m |= 32;
        }
        if (!empty($d['abstract'])) {
            $m |= 64;
        }
        if (!empty($d['readonly'])) {
            $m |= 128;
        }
        return $m;
    }
}

abstract class ReflectionType implements Stringable
{
    abstract public function allowsNull(): bool;
}

class ReflectionNamedType extends ReflectionType
{
    public function __construct(private string $typeName, private bool $nullable, private bool $builtin) {}

    public function getName(): string { return $this->typeName; }
    public function allowsNull(): bool { return $this->nullable; }
    public function isBuiltin(): bool { return $this->builtin; }
    public function __toString(): string
    {
        $n = $this->typeName;
        return $this->nullable && $n !== 'mixed' && $n !== 'null' ? '?' . $n : $n;
    }
}

class ReflectionUnionType extends ReflectionType
{
    public function __construct(private array $types, private bool $nullable) {}

    public function getTypes(): array { return $this->types; }
    public function allowsNull(): bool { return $this->nullable; }
    public function __toString(): string
    {
        return implode('|', array_map(fn($t) => $t instanceof ReflectionIntersectionType ? '(' . $t . ')' : (string) $t, $this->types));
    }
}

class ReflectionIntersectionType extends ReflectionType
{
    public function __construct(private array $types, private bool $nullable) {}

    public function getTypes(): array { return $this->types; }
    public function allowsNull(): bool { return $this->nullable; }
    public function __toString(): string { return implode('&', array_map('strval', $this->types)); }
}

class ReflectionAttribute implements Reflector
{
    const IS_INSTANCEOF = 2;

    public function __construct(private string $attrName, private int $id, private int $target, private bool $repeated) {}

    public function getName(): string { return $this->attrName; }
    public function getArguments(): array { return __phpx_refl_attr_args($this->id); }
    public function getTarget(): int { return $this->target; }
    public function isRepeated(): bool { return $this->repeated; }
    public function newInstance(): object
    {
        if (!class_exists($this->attrName)) {
            throw new Error(sprintf('Attribute class "%s" not found', $this->attrName));
        }
        return new ($this->attrName)(...$this->getArguments());
    }
    public function __toString(): string { return 'Attribute [ ' . $this->attrName . " ]\n"; }
}

abstract class ReflectionFunctionAbstract implements Reflector
{
    public string $name = '';
    /** @internal */
    protected array $d = [];

    public function getName(): string { return $this->d['name']; }
    public function getShortName(): string
    {
        $n = $this->d['name'];
        $i = strrpos($n, '\\');
        return $i === false ? $n : substr($n, $i + 1);
    }
    public function getNamespaceName(): string
    {
        $i = strrpos($this->d['name'], '\\');
        return $i === false ? '' : substr($this->d['name'], 0, $i);
    }
    public function inNamespace(): bool { return str_contains($this->d['name'], '\\'); }
    public function isClosure(): bool { return !empty($this->d['closure']); }
    public function isInternal(): bool { return $this->d['internal']; }
    public function isUserDefined(): bool { return !$this->d['internal']; }
    public function isGenerator(): bool { return $this->d['generator']; }
    public function isVariadic(): bool
    {
        foreach ($this->d['params'] as $p) {
            if ($p['variadic']) {
                return true;
            }
        }
        return false;
    }
    public function isStatic(): bool { return !empty($this->d['static']); }
    public function isDeprecated(): bool
    {
        return $this->getAttributes(Deprecated::class) !== [];
    }
    public function returnsReference(): bool { return $this->d['byref']; }
    public function getFileName(): string|false { return $this->d['file']; }
    public function getStartLine(): int|false { return $this->d['start']; }
    public function getEndLine(): int|false { return $this->d['end']; }
    public function getDocComment(): string|false { return $this->d['doc']; }
    public function getExtensionName(): string|false { return $this->d['internal'] ? 'standard' : false; }
    public function getNumberOfParameters(): int { return count($this->d['params']); }
    public function getNumberOfRequiredParameters(): int
    {
        $n = 0;
        foreach ($this->d['params'] as $p) {
            if (!$p['optional']) {
                $n++;
            }
        }
        return $n;
    }
    /** @return ReflectionParameter[] */
    public function getParameters(): array
    {
        $out = [];
        foreach ($this->d['params'] as $p) {
            $out[] = ReflectionParameter::__fromDesc($this, $p);
        }
        return $out;
    }
    public function hasReturnType(): bool { return $this->d['ret'] !== null; }
    public function getReturnType(): ?ReflectionType { return __PhpxRefl::type($this->d['ret']); }
    public function hasTentativeReturnType(): bool { return false; }
    public function getTentativeReturnType(): ?ReflectionType { return null; }
    public function getAttributes(?string $name = null, int $flags = 0): array
    {
        return __PhpxRefl::attributes($this->d['attrs'], $name, $flags, $this instanceof ReflectionMethod ? Attribute::TARGET_METHOD : Attribute::TARGET_FUNCTION);
    }
    public function getStaticVariables(): array { return $this->d['used'] ?? []; }
    public function getClosureThis(): ?object { return $this->d['this'] ?? null; }
    public function getClosureScopeClass(): ?ReflectionClass
    {
        return isset($this->d['scope']) ? new ReflectionClass($this->d['scope']) : null;
    }
    public function getClosureCalledClass(): ?ReflectionClass
    {
        $t = $this->d['this'] ?? null;
        return $t !== null ? new ReflectionClass($t) : $this->getClosureScopeClass();
    }
    public function getClosureUsedVariables(): array { return $this->d['used'] ?? []; }
}

class ReflectionFunction extends ReflectionFunctionAbstract
{
    const IS_DEPRECATED = 2048;

    private ?Closure $closure = null;

    public function __construct(Closure|string $function)
    {
        $d = __phpx_refl_function($function);
        if ($d === null) {
            throw new ReflectionException(sprintf('Function %s() does not exist', $function));
        }
        if ($function instanceof Closure) {
            $this->closure = $function;
            $d['closure'] = true;
        } elseif (!$d['internal'] && $d['ns'] !== '' && !str_contains($d['name'], '\\')) {
            $d['name'] = $d['ns'] . '\\' . $d['name'];
        }
        $this->d = $d;
        $this->name = $d['name'];
    }

    public function isAnonymous(): bool { return $this->closure !== null && str_starts_with($this->d['name'], '{closure'); }
    public function isDisabled(): bool { return false; }
    public function invoke(mixed ...$args): mixed
    {
        return ($this->closure ?? $this->d['name'])(...$args);
    }
    public function invokeArgs(array $args = []): mixed
    {
        return ($this->closure ?? $this->d['name'])(...$args);
    }
    public function getClosure(): Closure
    {
        return $this->closure ?? Closure::fromCallable($this->d['name']);
    }
    public function __toString(): string { return 'Function [ <user> function ' . $this->d['name'] . " ]\n"; }
}

class ReflectionMethod extends ReflectionFunctionAbstract
{
    const IS_STATIC = 16;
    const IS_PUBLIC = 1;
    const IS_PROTECTED = 2;
    const IS_PRIVATE = 4;
    const IS_ABSTRACT = 64;
    const IS_FINAL = 32;

    public string $class = '';
    /** @internal */
    protected array $m = [];

    public function __construct(object|string $objectOrMethod, ?string $method = null)
    {
        if ($method === null) {
            if (!is_string($objectOrMethod) || !str_contains($objectOrMethod, '::')) {
                throw new ReflectionException('ReflectionMethod::__construct(): Argument #1 ($objectOrMethod) must be a valid method name');
            }
            __phpx_deprecated('Calling ReflectionMethod::__construct() with 1 argument is deprecated, use ReflectionMethod::createFromMethodName() instead');
            [$objectOrMethod, $method] = explode('::', $objectOrMethod, 2);
        }
        $cls = __phpx_refl_class($objectOrMethod);
        if ($cls === null) {
            throw new ReflectionException(sprintf('Class "%s" does not exist', $objectOrMethod));
        }
        $m = __phpx_refl_method($objectOrMethod, $method);
        if ($m === null) {
            throw new ReflectionException(sprintf('Method %s::%s() does not exist', $cls['name'], $method));
        }
        $this->init($m);
    }

    public static function createFromMethodName(string $method): static
    {
        if (!str_contains($method, '::')) {
            throw new ReflectionException('ReflectionMethod::createFromMethodName(): Argument #1 ($method) must be a valid method name');
        }
        [$class, $name] = explode('::', $method, 2);
        return new static($class, $name);
    }

    /** @internal */
    public static function __fromDesc(array $m): self
    {
        $r = __phpx_refl_new_without_ctor(self::class);
        $r->init($m);
        return $r;
    }

    /** @internal */
    protected function init(array $m): void
    {
        $this->m = $m;
        $this->d = $m['fid'] >= 0 ? __phpx_refl_function($m['fid'], $m['name']) : [
            'name' => $m['name'], 'params' => [], 'ret' => null, 'doc' => false, 'file' => false, 'start' => false,
            'end' => false, 'attrs' => [], 'byref' => false, 'generator' => false, 'static' => $m['static'], 'internal' => true,
        ];
        $this->d['static'] = $m['static'];
        $this->name = $m['name'];
        $this->class = $m['class'];
    }

    public function getName(): string { return $this->m['name']; }
    public function getShortName(): string { return $this->m['name']; }
    public function getNamespaceName(): string { return ''; }
    public function inNamespace(): bool { return false; }
    public function isInternal(): bool { return $this->m['internal']; }
    public function isUserDefined(): bool { return !$this->m['internal']; }
    public function isPublic(): bool { return $this->m['vis'] === 1; }
    public function isProtected(): bool { return $this->m['vis'] === 2; }
    public function isPrivate(): bool { return $this->m['vis'] === 4; }
    public function isStatic(): bool { return $this->m['static']; }
    public function isAbstract(): bool { return $this->m['abstract']; }
    public function isFinal(): bool { return $this->m['final']; }
    public function isConstructor(): bool { return strcasecmp($this->m['name'], '__construct') === 0; }
    public function isDestructor(): bool { return strcasecmp($this->m['name'], '__destruct') === 0; }
    public function getModifiers(): int { return __PhpxRefl::modifiers($this->m); }
    public function getDeclaringClass(): ReflectionClass { return new ReflectionClass($this->m['class']); }
    public function getPrototype(): ReflectionMethod
    {
        $c = new ReflectionClass($this->m['class']);
        foreach ([$c->getParentClass() ?: null, ...array_map(fn($i) => new ReflectionClass($i), $c->getInterfaceNames())] as $p) {
            if ($p !== null && $p->hasMethod($this->m['name'])) {
                return $p->getMethod($this->m['name']);
            }
        }
        throw new ReflectionException(sprintf('Method %s::%s does not have a prototype', $this->m['class'], $this->m['name']));
    }
    public function hasPrototype(): bool
    {
        try {
            $this->getPrototype();
            return true;
        } catch (ReflectionException) {
            return false;
        }
    }
    public function setAccessible(bool $accessible): void
    {
        __phpx_deprecated('Method ReflectionMethod::setAccessible() is deprecated since 8.5, as it has no effect since PHP 8.1');
    }
    public function invoke(?object $object = null, mixed ...$args): mixed
    {
        return $this->invokeArgs($object, $args);
    }
    public function invokeArgs(?object $object = null, array $args = []): mixed
    {
        if (!$this->m['static']) {
            if ($object === null) {
                throw new ReflectionException(sprintf('Trying to invoke non static method %s::%s() without an object', $this->m['class'], $this->m['name']));
            }
            if (!($object instanceof $this->m['class'])) {
                throw new ReflectionException('Given object is not an instance of the class this method was declared in');
            }
        }
        if ($this->m['abstract']) {
            throw new ReflectionException(sprintf('Trying to invoke abstract method %s::%s()', $this->m['class'], $this->m['name']));
        }
        return __phpx_refl_invoke($this->m['class'], $this->m['name'], $object, $args);
    }
    public function getClosure(?object $object = null): Closure
    {
        return __phpx_refl_method_closure($object ?? $this->m['class'], $this->m['name'], $object);
    }
    public function __toString(): string { return 'Method [ <user> public method ' . $this->m['name'] . " ]\n"; }
}

class ReflectionParameter implements Reflector
{
    public string $name = '';
    private array $p;
    private ReflectionFunctionAbstract $fn;

    /** @internal */
    public static function __fromDesc(ReflectionFunctionAbstract $fn, array $p): self
    {
        $r = __phpx_refl_new_without_ctor(self::class);
        $r->fn = $fn;
        $r->p = $p;
        $r->name = $p['name'];
        return $r;
    }

    public function __construct($function, int|string $param)
    {
        $fn = match (true) {
            is_array($function) => new ReflectionMethod($function[0], $function[1]),
            is_string($function) && str_contains($function, '::') => new ReflectionMethod($function),
            $function instanceof ReflectionFunctionAbstract => $function,
            default => new ReflectionFunction($function),
        };
        foreach ($fn->getParameters() as $rp) {
            if ($rp->getPosition() === $param || $rp->name === $param) {
                $this->fn = $fn;
                $this->p = $rp->p;
                $this->name = $rp->name;
                return;
            }
        }
        throw new ReflectionException('The parameter specified by its name could not be found');
    }

    public function getName(): string { return $this->p['name']; }
    public function getPosition(): int { return $this->p['pos']; }
    public function hasType(): bool { return $this->p['type'] !== null; }
    public function getType(): ?ReflectionType { return __PhpxRefl::type($this->p['type']); }
    public function allowsNull(): bool { return $this->p['type'] === null || $this->getType()->allowsNull(); }
    public function isOptional(): bool { return $this->p['optional']; }
    public function isVariadic(): bool { return $this->p['variadic']; }
    public function isPassedByReference(): bool { return $this->p['byref']; }
    public function canBePassedByValue(): bool { return !$this->p['byref']; }
    public function isPromoted(): bool { return $this->p['promoted']; }
    public function isDefaultValueAvailable(): bool { return $this->p['has_default']; }
    public function getDefaultValue(): mixed
    {
        if (!$this->p['has_default']) {
            throw new ReflectionException('Internal error: Failed to retrieve the default value');
        }
        if ($this->p['fid'] < 0) {
            $src = $this->p['default_src'] ?? 'null';
            return match (strtolower($src)) {
                'null' => null, 'true' => true, 'false' => false, '[]' => [], "''", '""' => '',
                default => is_numeric($src) ? $src + 0 : (defined($src) ? constant($src) : trim($src, "'\"")),
            };
        }
        return __phpx_refl_param_default($this->p['fid'], $this->p['pos']);
    }
    public function isDefaultValueConstant(): bool { return $this->p['has_default'] && $this->p['default_const'] !== null; }
    public function getDefaultValueConstantName(): ?string { return $this->p['default_const']; }
    public function getDeclaringFunction(): ReflectionFunctionAbstract { return $this->fn; }
    public function getDeclaringClass(): ?ReflectionClass
    {
        return $this->fn instanceof ReflectionMethod ? $this->fn->getDeclaringClass() : null;
    }
    public function getClass(): ?ReflectionClass
    {
        $t = $this->getType();
        if (!$t instanceof ReflectionNamedType || $t->isBuiltin()) {
            return null;
        }
        $n = $t->getName();
        if ($n === 'self' || $n === 'parent') {
            $c = $this->getDeclaringClass();
            return $n === 'self' ? $c : ($c?->getParentClass() ?: null);
        }
        return new ReflectionClass($n);
    }
    public function isArray(): bool
    {
        $t = $this->getType();
        return $t instanceof ReflectionNamedType && $t->getName() === 'array';
    }
    public function isCallable(): bool
    {
        $t = $this->getType();
        return $t instanceof ReflectionNamedType && $t->getName() === 'callable';
    }
    public function getAttributes(?string $name = null, int $flags = 0): array
    {
        return __PhpxRefl::attributes($this->p['attrs'], $name, $flags, Attribute::TARGET_PARAMETER);
    }
    public function __toString(): string
    {
        return sprintf('Parameter #%d [ <%s> %s$%s ]', $this->p['pos'], $this->p['optional'] ? 'optional' : 'required', $this->p['type'] !== null ? $this->getType() . ' ' : '', $this->p['name']);
    }
}

class ReflectionProperty implements Reflector
{
    const IS_STATIC = 16;
    const IS_READONLY = 128;
    const IS_PUBLIC = 1;
    const IS_PROTECTED = 2;
    const IS_PRIVATE = 4;
    const IS_ABSTRACT = 64;
    const IS_FINAL = 32;

    public string $name = '';
    public string $class = '';
    private array $p;

    public function __construct(object|string $class, string $property)
    {
        {
            $info = __phpx_refl_class($class);
            if ($info === null) {
                throw new ReflectionException(sprintf('Class "%s" does not exist', $class));
            }
            $found = null;
            foreach (__phpx_refl_props($class) as $p) {
                if ($p['name'] === $property) {
                    $found = $p;
                    break;
                }
            }
            if ($found === null && is_object($class) && array_key_exists($property, get_object_vars($class))) {
                $found = ['name' => $property, 'class' => $info['name'], 'vis' => 1, 'static' => false, 'readonly' => false, 'type' => null,
                    'has_default' => false, 'doc' => false, 'attrs' => [], 'promoted' => false, 'dynamic' => true];
            }
            if ($found === null) {
                throw new ReflectionException(sprintf('Property %s::$%s does not exist', $info['name'], $property));
            }
            $this->p = $found;
        }
        $this->name = $this->p['name'];
        $this->class = $this->p['class'];
    }

    /** @internal */
    public static function __fromDesc(array $p): self
    {
        $r = __phpx_refl_new_without_ctor(self::class);
        $r->p = $p;
        $r->name = $p['name'];
        $r->class = $p['class'];
        return $r;
    }

    public function getName(): string { return $this->p['name']; }
    public function getValue(?object $object = null): mixed
    {
        if ($this->p['static']) {
            return __phpx_refl_static_get($this->p['class'], $this->p['name']);
        }
        if ($object === null) {
            throw new TypeError('ReflectionProperty::getValue(): Argument #1 ($object) must be provided for instance properties');
        }
        return __phpx_refl_prop_get($object, $this->p['class'], $this->p['name']);
    }
    public function setValue(mixed $objectOrValue = null, mixed $value = null): void
    {
        if ($this->p['static']) {
            __phpx_refl_static_set($this->p['class'], $this->p['name'], func_num_args() === 2 ? $value : $objectOrValue);
            return;
        }
        __phpx_refl_prop_set($objectOrValue, $this->p['class'], $this->p['name'], $value);
    }
    public function isInitialized(?object $object = null): bool
    {
        if ($this->p['static']) {
            return true;
        }
        return __phpx_refl_prop_isset($object, $this->p['name']);
    }
    public function isPublic(): bool { return $this->p['vis'] === 1; }
    public function isProtected(): bool { return $this->p['vis'] === 2; }
    public function isPrivate(): bool { return $this->p['vis'] === 4; }
    public function isStatic(): bool { return $this->p['static']; }
    public function isReadOnly(): bool { return $this->p['readonly']; }
    public function isDefault(): bool { return empty($this->p['dynamic']); }
    public function isDynamic(): bool { return !empty($this->p['dynamic']); }
    public function isPromoted(): bool { return $this->p['promoted']; }
    public function isAbstract(): bool { return false; }
    public function isFinal(): bool { return false; }
    public function isVirtual(): bool { return false; }
    public function hasHooks(): bool { return false; }
    public function getHooks(): array { return []; }
    public function getModifiers(): int { return __PhpxRefl::modifiers($this->p); }
    public function getDeclaringClass(): ReflectionClass { return new ReflectionClass($this->p['class']); }
    public function getDocComment(): string|false { return $this->p['doc']; }
    public function setAccessible(bool $accessible): void
    {
        __phpx_deprecated('Method ReflectionProperty::setAccessible() is deprecated since 8.5, as it has no effect since PHP 8.1');
    }
    public function hasType(): bool { return $this->p['type'] !== null; }
    public function getType(): ?ReflectionType { return __PhpxRefl::type($this->p['type']); }
    public function getSettableType(): ?ReflectionType { return $this->getType(); }
    public function hasDefaultValue(): bool { return $this->p['has_default']; }
    public function getDefaultValue(): mixed
    {
        return $this->p['has_default'] ? __phpx_refl_prop_default($this->p['class'], $this->p['name']) : null;
    }
    public function getAttributes(?string $name = null, int $flags = 0): array
    {
        return __PhpxRefl::attributes($this->p['attrs'], $name, $flags, Attribute::TARGET_PROPERTY);
    }
    public function __toString(): string { return 'Property [ public $' . $this->p['name'] . " ]\n"; }
}

class ReflectionClassConstant implements Reflector
{
    const IS_PUBLIC = 1;
    const IS_PROTECTED = 2;
    const IS_PRIVATE = 4;
    const IS_FINAL = 32;

    public string $name = '';
    public string $class = '';
    /** @internal */
    protected array $c;

    /** @internal */
    public static function __fromDesc(array $c): static
    {
        $r = __phpx_refl_new_without_ctor(static::class);
        $r->c = $c;
        $r->name = $c['name'];
        $r->class = $c['class'];
        return $r;
    }

    public function __construct(object|string $class, string $constant)
    {
        {
            $found = null;
            foreach (__phpx_refl_consts($class) as $c) {
                if ($c['name'] === $constant) {
                    $found = $c;
                }
            }
            if ($found === null) {
                $info = __phpx_refl_class($class);
                throw new ReflectionException(sprintf('Constant %s::%s does not exist', $info['name'] ?? $class, $constant));
            }
            $this->c = $found;
        }
        $this->name = $this->c['name'];
        $this->class = $this->c['class'];
    }

    public function getName(): string { return $this->c['name']; }
    public function getValue(): mixed { return constant($this->c['class'] . '::' . $this->c['name']); }
    public function isPublic(): bool { return $this->c['vis'] === 1; }
    public function isProtected(): bool { return $this->c['vis'] === 2; }
    public function isPrivate(): bool { return $this->c['vis'] === 4; }
    public function isFinal(): bool { return $this->c['final']; }
    public function isEnumCase(): bool { return $this->c['case']; }
    public function isDeprecated(): bool { return false; }
    public function hasType(): bool { return false; }
    public function getType(): ?ReflectionType { return null; }
    public function getModifiers(): int { return __PhpxRefl::modifiers($this->c); }
    public function getDeclaringClass(): ReflectionClass { return new ReflectionClass($this->c['class']); }
    public function getDocComment(): string|false { return $this->c['doc']; }
    public function getAttributes(?string $name = null, int $flags = 0): array
    {
        return __PhpxRefl::attributes($this->c['attrs'], $name, $flags, Attribute::TARGET_CLASS_CONSTANT);
    }
    public function __toString(): string { return 'Constant [ ' . $this->c['name'] . " ]\n"; }
}

class ReflectionClass implements Reflector
{
    const IS_IMPLICIT_ABSTRACT = 16;
    const IS_EXPLICIT_ABSTRACT = 64;
    const IS_FINAL = 32;
    const IS_READONLY = 65536;
    const SKIP_INITIALIZATION_ON_SERIALIZE = 8;
    const SKIP_DESTRUCTOR = 16;

    public string $name = '';
    /** @internal */
    protected array $d;

    public function __construct(object|string $objectOrClass)
    {
        $d = __phpx_refl_class($objectOrClass);
        if ($d === null) {
            throw new ReflectionException(sprintf('Class "%s" does not exist', $objectOrClass));
        }
        $this->d = $d;
        $this->name = $d['name'];
    }

    public function getName(): string { return $this->d['name']; }
    public function getShortName(): string
    {
        $i = strrpos($this->d['name'], '\\');
        return $i === false ? $this->d['name'] : substr($this->d['name'], $i + 1);
    }
    public function getNamespaceName(): string
    {
        $i = strrpos($this->d['name'], '\\');
        return $i === false ? '' : substr($this->d['name'], 0, $i);
    }
    public function inNamespace(): bool { return str_contains($this->d['name'], '\\'); }
    public function isInterface(): bool { return $this->d['kind'] === 1; }
    public function isTrait(): bool { return $this->d['kind'] === 2; }
    public function isEnum(): bool { return $this->d['kind'] === 3; }
    public function isAbstract(): bool { return $this->d['abstract'] || $this->d['kind'] === 1 || ($this->d['kind'] === 2 && $this->hasAbstractMethods()); }
    private function hasAbstractMethods(): bool
    {
        foreach (__phpx_refl_methods($this->d['name']) as $m) {
            if ($m['abstract']) {
                return true;
            }
        }
        return false;
    }
    public function isFinal(): bool { return $this->d['final']; }
    public function isReadOnly(): bool { return $this->d['readonly']; }
    public function isAnonymous(): bool { return $this->d['anon']; }
    public function isInternal(): bool { return $this->d['internal']; }
    public function isUserDefined(): bool { return !$this->d['internal']; }
    public function isInstantiable(): bool
    {
        if ($this->d['kind'] !== 0 || $this->d['abstract']) {
            return false;
        }
        $c = $this->getConstructor();
        return $c === null || $c->isPublic();
    }
    public function isCloneable(): bool
    {
        if ($this->d['kind'] !== 0 || $this->d['abstract']) {
            return false;
        }
        $m = __phpx_refl_method($this->d['name'], '__clone');
        return $m === null || $m['vis'] === 1;
    }
    public function isIterable(): bool { return $this->d['kind'] === 0 && !$this->d['abstract'] && $this->implementsInterface(Traversable::class); }
    public function isIterateable(): bool { return $this->isIterable(); }
    public function isInstance(object $object): bool { return $object instanceof $this->d['name']; }
    public function getModifiers(): int
    {
        $m = 0;
        if ($this->d['explicit_abstract']) {
            $m |= self::IS_EXPLICIT_ABSTRACT;
        } elseif ($this->d['abstract']) {
            $m |= self::IS_IMPLICIT_ABSTRACT;
        }
        if ($this->d['final']) {
            $m |= self::IS_FINAL;
        }
        if ($this->d['readonly']) {
            $m |= self::IS_READONLY;
        }
        return $m;
    }
    public function getFileName(): string|false { return $this->d['file']; }
    public function getStartLine(): int|false { return $this->d['start']; }
    public function getEndLine(): int|false { return $this->d['end']; }
    public function getDocComment(): string|false { return $this->d['doc']; }
    public function getExtension(): ?object { return null; }
    public function getExtensionName(): string|false { return $this->d['internal'] ? 'Core' : false; }
    public function getParentClass(): ReflectionClass|false
    {
        return $this->d['parent'] === false ? false : new ReflectionClass($this->d['parent']);
    }
    public function isSubclassOf(ReflectionClass|string $class): bool
    {
        $n = $class instanceof ReflectionClass ? $class->getName() : $class;
        if (!class_exists($n) && !interface_exists($n)) {
            throw new ReflectionException(sprintf('Class "%s" does not exist', $n));
        }
        return strcasecmp(ltrim($n, '\\'), $this->d['name']) !== 0 && is_a($this->d['name'], $n, true);
    }
    public function implementsInterface(ReflectionClass|string $interface): bool
    {
        $n = $interface instanceof ReflectionClass ? $interface->getName() : $interface;
        if (!interface_exists($n)) {
            if (class_exists($n)) {
                throw new ReflectionException(sprintf('%s is not an interface', ltrim($n, '\\')));
            }
            throw new ReflectionException(sprintf('Interface "%s" does not exist', $n));
        }
        return is_a($this->d['name'], $n, true);
    }
    public function getInterfaceNames(): array { return $this->d['interfaces']; }
    public function getInterfaces(): array
    {
        $out = [];
        foreach ($this->d['interfaces'] as $i) {
            $out[$i] = new ReflectionClass($i);
        }
        return $out;
    }
    public function getTraitNames(): array { return $this->d['traits']; }
    public function getTraits(): array
    {
        $out = [];
        foreach ($this->d['traits'] as $t) {
            $out[$t] = new ReflectionClass($t);
        }
        return $out;
    }
    public function getTraitAliases(): array { return []; }
    public function getConstructor(): ?ReflectionMethod
    {
        $m = __phpx_refl_method($this->d['name'], '__construct');
        return $m === null ? null : ReflectionMethod::__fromDesc($m);
    }
    public function hasMethod(string $name): bool { return __phpx_refl_method($this->d['name'], $name) !== null; }
    public function getMethod(string $name): ReflectionMethod
    {
        $m = __phpx_refl_method($this->d['name'], $name);
        if ($m === null) {
            throw new ReflectionException(sprintf('Method %s::%s() does not exist', $this->d['name'], $name));
        }
        return ReflectionMethod::__fromDesc($m);
    }
    public function getMethods(?int $filter = null): array
    {
        $out = [];
        foreach (__phpx_refl_methods($this->d['name']) as $m) {
            if ($filter === null || (__PhpxRefl::modifiers($m) & $filter)) {
                $out[] = ReflectionMethod::__fromDesc($m);
            }
        }
        return $out;
    }
    public function hasProperty(string $name): bool
    {
        foreach (__phpx_refl_props($this->d['name']) as $p) {
            if ($p['name'] === $name) {
                return true;
            }
        }
        return false;
    }
    public function getProperty(string $name): ReflectionProperty
    {
        foreach (__phpx_refl_props($this->d['name']) as $p) {
            if ($p['name'] === $name) {
                return ReflectionProperty::__fromDesc($p);
            }
        }
        throw new ReflectionException(sprintf('Property %s::$%s does not exist', $this->d['name'], $name));
    }
    public function getProperties(?int $filter = null): array
    {
        $out = [];
        foreach (__phpx_refl_props($this->d['name']) as $p) {
            if ($filter === null || (__PhpxRefl::modifiers($p) & $filter)) {
                $out[] = ReflectionProperty::__fromDesc($p);
            }
        }
        return $out;
    }
    public function getDefaultProperties(): array
    {
        return __phpx_refl_default_props($this->d['name']);
    }
    public function getStaticProperties(): array
    {
        $out = [];
        foreach (__phpx_refl_props($this->d['name']) as $p) {
            if ($p['static']) {
                $out[$p['name']] = __phpx_refl_static_get($this->d['name'], $p['name']);
            }
        }
        return $out;
    }
    public function getStaticPropertyValue(string $name, mixed $default = null): mixed
    {
        foreach (__phpx_refl_props($this->d['name']) as $p) {
            if ($p['static'] && $p['name'] === $name) {
                return __phpx_refl_static_get($this->d['name'], $name);
            }
        }
        if (func_num_args() > 1) {
            return $default;
        }
        throw new ReflectionException(sprintf('Property %s::$%s does not exist', $this->d['name'], $name));
    }
    public function setStaticPropertyValue(string $name, mixed $value): void
    {
        __phpx_refl_static_set($this->d['name'], $name, $value);
    }
    public function hasConstant(string $name): bool
    {
        foreach (__phpx_refl_consts($this->d['name']) as $c) {
            if ($c['name'] === $name) {
                return true;
            }
        }
        return false;
    }
    public function getConstants(?int $filter = null): array
    {
        $out = [];
        foreach (__phpx_refl_consts($this->d['name']) as $c) {
            if ($filter === null || (__PhpxRefl::modifiers($c) & $filter)) {
                $out[$c['name']] = constant($this->d['name'] . '::' . $c['name']);
            }
        }
        return $out;
    }
    public function getConstant(string $name): mixed
    {
        return $this->hasConstant($name) ? constant($this->d['name'] . '::' . $name) : false;
    }
    public function getReflectionConstant(string $name): ReflectionClassConstant|false
    {
        foreach (__phpx_refl_consts($this->d['name']) as $c) {
            if ($c['name'] === $name) {
                return ReflectionClassConstant::__fromDesc($c);
            }
        }
        return false;
    }
    public function getReflectionConstants(?int $filter = null): array
    {
        $out = [];
        foreach (__phpx_refl_consts($this->d['name']) as $c) {
            if ($filter === null || (__PhpxRefl::modifiers($c) & $filter)) {
                $out[] = ReflectionClassConstant::__fromDesc($c);
            }
        }
        return $out;
    }
    public function getAttributes(?string $name = null, int $flags = 0): array
    {
        return __PhpxRefl::attributes($this->d['attrs'], $name, $flags, Attribute::TARGET_CLASS);
    }
    public function newInstance(mixed ...$args): object
    {
        return $this->newInstanceArgs($args);
    }
    public function newInstanceArgs(array $args = []): object
    {
        if (!$this->d['has_ctor'] && $args !== []) {
            throw new ReflectionException(sprintf('Class %s does not have a constructor, so you cannot pass any constructor arguments', $this->d['name']));
        }
        $c = $this->getConstructor();
        if ($c !== null && !$c->isPublic()) {
            throw new ReflectionException(sprintf('Access to non-public constructor of class %s', $this->d['name']));
        }
        return new ($this->d['name'])(...$args);
    }
    public function newInstanceWithoutConstructor(): object
    {
        if ($this->d['kind'] !== 0 || $this->d['abstract']) {
            throw new Error(sprintf('Cannot instantiate %s %s', match ($this->d['kind']) { 1 => 'interface', 2 => 'trait', 3 => 'enum', default => 'abstract class' }, $this->d['name']));
        }
        return __phpx_refl_new_without_ctor($this->d['name']);
    }
    public function newLazyGhost(callable $initializer, int $options = 0): object
    {
        $o = $this->newInstanceWithoutConstructor();
        $initializer($o);
        return $o;
    }
    public function newLazyProxy(callable $factory, int $options = 0): object
    {
        return $factory($this->newInstanceWithoutConstructor());
    }
    public function __toString(): string { return 'Class [ <user> class ' . $this->d['name'] . " ]\n"; }
}

class ReflectionObject extends ReflectionClass
{
    public function __construct(object $object)
    {
        parent::__construct($object);
    }
}

class ReflectionEnum extends ReflectionClass
{
    public function __construct(object|string $objectOrClass)
    {
        parent::__construct($objectOrClass);
        if ($this->d['kind'] !== 3) {
            throw new ReflectionException(sprintf('Class "%s" is not an enum', $this->d['name']));
        }
    }

    public function isBacked(): bool { return $this->d['backing'] !== null; }
    public function getBackingType(): ?ReflectionNamedType
    {
        return $this->d['backing'] === null ? null : new ReflectionNamedType($this->d['backing'], false, true);
    }
    public function hasCase(string $name): bool
    {
        foreach (__phpx_refl_consts($this->d['name']) as $c) {
            if ($c['case'] && $c['name'] === $name) {
                return true;
            }
        }
        return false;
    }
    public function getCase(string $name): ReflectionEnumUnitCase
    {
        foreach (__phpx_refl_consts($this->d['name']) as $c) {
            if ($c['case'] && $c['name'] === $name) {
                return $this->d['backing'] === null ? ReflectionEnumUnitCase::__fromDesc($c) : ReflectionEnumBackedCase::__fromDesc($c);
            }
        }
        throw new ReflectionException(sprintf('Case %s::%s does not exist', $this->d['name'], $name));
    }
    public function getCases(): array
    {
        $out = [];
        foreach (__phpx_refl_consts($this->d['name']) as $c) {
            if ($c['case']) {
                $out[] = $this->d['backing'] === null ? ReflectionEnumUnitCase::__fromDesc($c) : ReflectionEnumBackedCase::__fromDesc($c);
            }
        }
        return $out;
    }
}

class ReflectionEnumUnitCase extends ReflectionClassConstant
{
    public function getEnum(): ReflectionEnum { return new ReflectionEnum($this->c['class']); }
}

class ReflectionEnumBackedCase extends ReflectionEnumUnitCase
{
    public function getBackingValue(): int|string { return $this->getValue()->value; }
}

final class ReflectionReference
{
    private string $id;

    private function __construct()
    {
    }

    public static function fromArrayElement(array $array, int|string $key): ?ReflectionReference
    {
        if (!array_key_exists($key, $array)) {
            throw new ReflectionException('Array key not found');
        }
        $id = \__phpx_reflection_reference_id($array, $key);
        if ($id === null) {
            return null;
        }
        $r = new self();
        $r->id = $id;
        return $r;
    }

    public function getId(): string
    {
        return $this->id;
    }
}
