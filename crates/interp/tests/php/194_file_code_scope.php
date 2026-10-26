<?php
// File code (include) sees and changes its includer's variables: variable
// variables, get_defined_vars(), extract(), unset(), $this, return.
$view = tempnam(sys_get_temp_dir(), 'v');
file_put_contents($view, <<<'PHPV'
<?php
$attributes ??= ['class' => 'x'];
foreach (array_filter(['title' => 't', 'size' => 2], 'is_string', ARRAY_FILTER_USE_KEY) as $__key => $__value) {
    $$__key = $$__key ?? $__value;
}
$__defined_vars = get_defined_vars();
foreach (['size' => 1, 'title' => 0] as $__key => $__value) {
    if (array_key_exists($__key, $__defined_vars)) unset($$__key);
}
unset($__defined_vars, $__key, $__value);
$later = 'L';
$__k = array_keys(get_defined_vars()); sort($__k); echo implode(',', $__k), "\n"; unset($__k);
echo isset($title) ? "title set\n" : "title unset\n";
if (isset($this)) { echo get_class($this), ' ', $this->name, "\n"; }
extract(['ex1' => 'E1', 'later' => 'L2']);
echo $ex1, $later, "\n";
$n = 'dyn'; $$n = 5; echo $dyn, ' ', ${'d' . 'yn'}, "\n";
$ret = compact('later', 'n');
return $ret;

PHPV);
$props = tempnam(sys_get_temp_dir(), 'p');
file_put_contents($props, <<<'PHPV'
<?php $attributes ??= new Bag([]);

$__newAttributes = [];
$__propNames = Bag::extractPropNames(([
    'user' => cur(),
]));

foreach ($attributes->all() as $__key => $__value) {
    if (in_array($__key, $__propNames)) {
        $$__key = $$__key ?? $__value;
    } else {
        $__newAttributes[$__key] = $__value;
    }
}

$attributes = new Bag($__newAttributes);

unset($__propNames);
unset($__newAttributes);

foreach (array_filter(([
    'user' => cur(),
]), 'is_string', ARRAY_FILTER_USE_KEY) as $__key => $__value) {
    $$__key = $$__key ?? $__value;
}

$__defined_vars = get_defined_vars();

foreach ($attributes->all() as $__key => $__value) {
    if (array_key_exists($__key, $__defined_vars)) unset($$__key);
}

unset($__defined_vars, $__key, $__value); ?>
<?php echo name($user), "|", implode(',', (function ($k) { sort($k); return $k; })(array_keys(get_defined_vars()))), "\n"; ?>

PHPV);
function keys($a) { $k = array_keys($a); sort($k); return $k; }
class Bag { function __construct(public $a) {} function all() { return $this->a; } static function extractPropNames($k) { return array_keys($k); } }
function cur() { return 'U'; }
function name($u) { if ($u === null) return 'NULL!'; return "name($u)"; }
class Comp { public $name = 'comp'; function render($view, $data) { extract($data); $r = include $view; return [$r, keys(get_defined_vars())]; } }
function plain($view, $data) { extract($data); $r = include $view; return [$r, keys(get_defined_vars()), $later ?? null]; }
$c = new Comp;
$render = function ($__path, $__data) { extract($__data, EXTR_SKIP); include $__path; };
for ($i = 0; $i < 4; $i++) {
  var_dump($c->render($view, ['size' => 3]));
  var_dump(plain($view, ['title' => 'given']));
  $cl = function () use ($view) { $x = 1; return include $view; };
  var_dump($cl());
  $render($props, ['attributes' => new Bag(['class' => 'c'])]);
  $render($props, ['attributes' => new Bag(['user' => 'given'])]);
  $render($props, []);
}
$globalv = 'G';
$gfile = tempnam(sys_get_temp_dir(), 'g');
file_put_contents($gfile, '<?php $later = "L"; $n = "dyn"; $$n = 5; return $later . $dyn;');
function readg() { global $later, $globalv, $dyn; return [$later, $globalv, $dyn]; }
$r = include $gfile;
var_dump($r, readg(), $later);
function gu() { unset($GLOBALS['later']); } gu(); var_dump(isset($later));
unlink($gfile);
unlink($view); unlink($props);
