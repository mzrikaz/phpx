<?php
/**
 * Parser oracle: print every operator tree in each file using nikic/php-parser,
 * in the same format as `phpx optrees`, so the two can be diffed.
 *
 * Usage: php optrees.php <path-to-nikic/php-parser> < file-list
 *
 * Output per file: "== <path>" then one sorted line per maximal operator tree,
 * with non-operator nodes printed as `_`, e.g. `(or (and _ _) (= _ (?? _ _)))`.
 */

declare(strict_types=1);

use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\Node\Expr\BinaryOp;
use PhpParser\Node\Expr\AssignOp;
use PhpParser\ParserFactory;

$lib = rtrim($argv[1] ?? '', '/') . '/lib/';
spl_autoload_register(function (string $class) use ($lib): void {
    if (str_starts_with($class, 'PhpParser\\')) {
        $file = $lib . str_replace('\\', '/', $class) . '.php';
        if (is_file($file)) {
            require $file;
        }
    }
});

const BINARY = [
    'BooleanAnd' => 'and', 'LogicalAnd' => 'and', 'BooleanOr' => 'or', 'LogicalOr' => 'or', 'LogicalXor' => 'xor',
    'Plus' => '+', 'Minus' => '-', 'Mul' => '*', 'Div' => '/', 'Mod' => '%', 'Pow' => '**', 'Concat' => '.',
    'BitwiseAnd' => '&', 'BitwiseOr' => '|', 'BitwiseXor' => '^', 'ShiftLeft' => '<<', 'ShiftRight' => '>>',
    'Equal' => '==', 'NotEqual' => '!=', 'Identical' => '===', 'NotIdentical' => '!==',
    'Smaller' => '<', 'SmallerOrEqual' => '<=', 'Greater' => '>', 'GreaterOrEqual' => '>=',
    'Spaceship' => '<=>', 'Coalesce' => '??',
];

function short(Node $n): string
{
    $parts = explode('\\', get_class($n));
    return rtrim(end($parts), '_');
}

/** Label and operand list if $n is an operator node, else null. */
function op(Node $n): ?array
{
    $s = short($n);
    return match (true) {
        $n instanceof BinaryOp => [BINARY[$s], [$n->left, $n->right]],
        $n instanceof AssignOp => [BINARY[$s] . '=', [$n->var, $n->expr]],
        $n instanceof Expr\Assign, $n instanceof Expr\AssignRef => ['=', [$n->var, $n->expr]],
        $n instanceof Expr\BooleanNot => ['!', [$n->expr]],
        $n instanceof Expr\UnaryMinus => ['neg', [$n->expr]],
        $n instanceof Expr\UnaryPlus => ['plus', [$n->expr]],
        $n instanceof Expr\BitwiseNot => ['~', [$n->expr]],
        $n instanceof Expr\ErrorSuppress => ['@', [$n->expr]],
        $n instanceof Expr\Cast => ['cast', [$n->expr]],
        $n instanceof Expr\PreInc, $n instanceof Expr\PreDec,
        $n instanceof Expr\PostInc, $n instanceof Expr\PostDec => ['incdec', [$n->var]],
        $n instanceof Expr\Clone_ => ['clone', [$n->expr]],
        $n instanceof Expr\Print_ => ['print', [$n->expr]],
        $n instanceof Expr\Throw_ => ['throw', [$n->expr]],
        $n instanceof Expr\Instanceof_ => ['instanceof', [$n->expr]],
        $n instanceof Expr\Ternary => $n->if === null
            ? ['?:', [$n->cond, $n->else]]
            : ['?', [$n->cond, $n->if, $n->else]],
        default => null,
    };
}

function tree(Node $n): string
{
    $op = op($n);
    if ($op === null) {
        return '_';
    }
    return '(' . $op[0] . ' ' . implode(' ', array_map('tree', $op[1])) . ')';
}

/** Walk everything; record trees rooted at operators whose parent isn't one. */
function collect(mixed $node, bool $parentIsOp, array &$out): void
{
    if (is_array($node)) {
        foreach ($node as $child) {
            collect($child, $parentIsOp, $out);
        }
        return;
    }
    if (!$node instanceof Node) {
        return;
    }
    $op = op($node);
    if ($op !== null && !$parentIsOp) {
        $out[] = tree($node);
    }
    $operands = $op[1] ?? [];
    foreach ($node->getSubNodeNames() as $name) {
        $child = $node->$name;
        // Operands of an operator are part of its tree; everything else
        // (call args, array items, ...) starts fresh.
        $inTree = $op !== null && in_array($child, $operands, true);
        collect($child, $inTree && op($child) !== null, $out);
    }
}

$parser = (new ParserFactory())->createForNewestSupportedVersion();
while (($path = fgets(STDIN)) !== false) {
    $path = rtrim($path, "\n");
    echo "== $path\n";
    try {
        $ast = $parser->parse(file_get_contents($path));
    } catch (Throwable $e) {
        echo "!! parse error: {$e->getMessage()}\n";
        continue;
    }
    $out = [];
    collect($ast, false, $out);
    sort($out, SORT_STRING);
    foreach ($out as $line) {
        echo $line, "\n";
    }
}
