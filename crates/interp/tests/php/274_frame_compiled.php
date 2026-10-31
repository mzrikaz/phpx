<?php
class Arr {
    public static function forget(&$array, $keys) {
        $original = &$array;
        $keys = (array) $keys;
        if (count($keys) === 0) return;
        foreach ($keys as $key) {
            if (array_key_exists($key, $array)) { unset($array[$key]); continue; }
            $parts = explode('.', $key);
            $array = &$original;
            while (count($parts) > 1) {
                $part = array_shift($parts);
                if (isset($array[$part]) && is_array($array[$part])) { $array = &$array[$part]; } else { continue 2; }
            }
            unset($array[array_shift($parts)]);
        }
    }
    public static function set(&$array, $key, $value) {
        if (is_null($key)) return $array = $value;
        $keys = explode('.', $key);
        foreach ($keys as $i => $key) {
            if (count($keys) === 1) break;
            unset($keys[$i]);
            if (! isset($array[$key]) || ! is_array($array[$key])) $array[$key] = [];
            $array = &$array[$key];
        }
        $array[array_shift($keys)] = $value;
        return $array;
    }
}
function counter() { static $n = 0; return ++$n; }
function inc_all(array &$xs, $by = 1) { foreach ($xs as &$x) { $x += $by; } unset($x); return count($xs); }
function load($f) { return include $f; }
class K { private $items = ['a' => 1]; function sum() { $t = 0; $add = function ($v) use (&$t) { $t += $v; }; foreach ($this->items as $v) $add($v); array_map(fn($x) => $add($x), [1, 2]); return $t; } }
$inc = sys_get_temp_dir() . '/phpx_frame_inc_' . getmypid() . '.php'; file_put_contents($inc, '<?php return $f . "!";');
$tot = 0;
for ($i = 0; $i < 3000; $i++) {
    $a = ['x' => ['y' => 1, 'z' => 2], 'w' => 3];
    Arr::forget($a, ['x.y', 'w', 'q.r']);
    Arr::set($a, 'm.n.o', $i);
    $tot += count($a, COUNT_RECURSIVE) + $a['m']['n']['o'];
    $c = counter();
    $xs = [1, 2, 3]; inc_all($xs, 2); $tot += array_sum($xs);
    $tot += strlen(load($inc));
    $tot += (new K)->sum();
}
echo $tot, " ", $c, " ", json_encode($a), "\n";
unlink($inc);
// By-reference arguments from code compiled on its frame share the
// variable itself (a fiber suspended inside sees later changes).
function walk(int $n, array &$log): int { if ($n === 0) { $log[] = "bottom"; Fiber::suspend(); $log[] = "resumed"; return 0; } return walk($n - 1, $log) + $n; }
$log = [];
for ($r = 0; $r < 40; $r++) {
    $f = new Fiber(function () use (&$log) { return walk(3, $log); });
    $f->start(); $log[] = "outside"; $f->resume();
}
echo count($log), " ", implode(",", array_slice($log, 0, 3)), "\n";
